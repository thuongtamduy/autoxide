//! Vận đơn (bước 44 mục 4): nhận yêu cầu giao của `orders` (event) hay nhân viên tạo tay, nhiều
//! kiện, tách (giao một phần), áp cập nhật trạng thái của hãng / nhân viên theo bảng chuyển
//! (`status.rs`), hành trình chỉ thêm, phát event cho `orders`.
//!
//! Chống trùng: unique (`tenant_id`, `order_id`, `request_key`) + dấu vân tay của yêu cầu — nhận
//! lại cùng yêu cầu trả vận đơn cũ, cùng khoá khác nội dung bị từ chối (không ghi đè).

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::job::EnqueueOptions;
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::secret::sha256;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use autoxide_kernel::time::Timestamp;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, DatabaseTransaction, EntityTrait,
    QueryFilter, QueryOrder, Set,
};
use uuid::Uuid;

use crate::addresses::address_of;
use crate::api::{
    Direction, MethodKind, ShipmentEvent, ShipmentRequest, ShipmentStatus, events as names,
};
use crate::entity::{parcel, shipment, shipment_item, tracking_event};
use crate::errors;
use crate::events::{self, Change, RecordNamed};
use crate::jobs::{BookShipment, PostCod};
use crate::permissions::{SHIPMENTS_MANAGE, SHIPMENTS_READ};
use crate::service::{ShippingService, now, unique_violation};
use crate::settings::{find_method, readable, writable};
use crate::status;

/// Nguồn của một bước hành trình.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    Carrier,
    Staff,
    System,
}

impl Source {
    fn as_str(self) -> &'static str {
        match self {
            Source::Carrier => "carrier",
            Source::Staff => "staff",
            Source::System => "system",
        }
    }
}

/// Một cập nhật trạng thái cần áp.
#[derive(Debug, Clone)]
pub(crate) struct StatusUpdate {
    pub event_key: String,
    pub status: ShipmentStatus,
    pub carrier_status: String,
    pub occurred_at: Timestamp,
    pub note: String,
    pub source: Source,
}

/// Vận đơn đầy đủ.
#[derive(Debug, Clone)]
pub(crate) struct ShipmentView {
    pub shipment: shipment::Model,
    pub parcels: Vec<parcel::Model>,
    pub items: Vec<shipment_item::Model>,
    pub events: Vec<tracking_event::Model>,
}

/// Lọc danh sách vận đơn.
#[derive(Debug, Clone, Default)]
pub(crate) struct ShipmentFilter {
    pub order_id: Option<Uuid>,
    pub status: Option<ShipmentStatus>,
    pub org_id: Option<Uuid>,
    pub tracking: Option<String>,
}

/// Một kiện gửi lên (`PUT /shipments/{id}/parcels`).
#[derive(Debug, Clone)]
pub(crate) struct ParcelInput {
    pub weight_grams: i64,
    pub length_mm: Option<i64>,
    pub width_mm: Option<i64>,
    pub height_mm: Option<i64>,
    /// Khoá dòng của đơn nằm trong kiện này.
    pub items: Vec<String>,
}

/// Payload `after` của event vận đơn.
pub(crate) fn event_of(row: &shipment::Model) -> ShipmentEvent {
    ShipmentEvent {
        shipment_id: row.id,
        order_id: row.order_id,
        request_key: row.request_key.clone(),
        direction: Direction::parse(&row.direction).unwrap_or(Direction::Outbound),
        org_id: OrgId::from_uuid(row.org_id),
        status: status_of(row),
        tracking_number: row.tracking_number.clone(),
        carrier_code: row.carrier_code.clone(),
        cod_amount: row.cod_minor.and_then(|minor| Money::from_columns(minor, &row.currency).ok()),
        at: row.status_at,
    }
}

pub(crate) fn status_of(row: &shipment::Model) -> ShipmentStatus {
    ShipmentStatus::parse(&row.status).unwrap_or(ShipmentStatus::Pending)
}

/// Dấu vân tay của yêu cầu (SHA-256 của JSON chuẩn).
fn fingerprint(request: &ShipmentRequest) -> String {
    sha256(&serde_json::to_string(request).unwrap_or_default())
}

pub(crate) fn invalid(problems: Vec<FieldError>) -> Result<(), Error> {
    if problems.is_empty() { Ok(()) } else { Err(Error::validation(problems)) }
}

/// Kiểm yêu cầu giao.
fn check_request(request: &ShipmentRequest) -> Result<(), Error> {
    let mut problems = Vec::new();
    crate::service::check_length(&mut problems, "request_key", &request.request_key, 1, 100);
    crate::service::check_length(&mut problems, "order_number", &request.order_number, 1, 64);
    crate::service::check_length(&mut problems, "recipient_name", &request.recipient_name, 1, 200);
    crate::addresses::check_phone(&mut problems, "recipient_phone", &request.recipient_phone);
    if request.items.is_empty() {
        problems.push(FieldError::new("items", "required", "list at least one item"));
    }
    for (index, item) in request.items.iter().enumerate() {
        if item.quantity <= 0 {
            problems.push(FieldError::new(
                format!("items[{index}].quantity"),
                "range",
                "must be more than 0",
            ));
        }
    }
    let currency = request.declared_value.currency();
    for (field, money) in [("cod_amount", request.cod_amount), ("quoted_fee", request.quoted_fee)] {
        if let Some(money) = money
            && (money.currency() != currency || money.is_negative())
        {
            problems.push(FieldError::new(
                field,
                "money",
                "must be 0 or more, in the currency of `declared_value`",
            ));
        }
    }
    invalid(problems)
}

impl ShippingService {
    /// Tạo vận đơn `pending` từ yêu cầu (event của `orders` hay nhân viên). Nhận lại cùng yêu cầu
    /// → vận đơn cũ (`false`: không tạo mới).
    pub(crate) async fn receive_request(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        request: &ShipmentRequest,
    ) -> Result<(shipment::Model, bool), Error> {
        check_request(request)?;
        let print = fingerprint(request);
        if let Some(existing) = self.by_request(tenant.database().read(), tenant, request).await? {
            return same_request(existing, &print).map(|row| (row, false));
        }
        let method = find_method(tenant.database().read(), tenant, request.method_id).await?;
        let transaction = tenant.database().begin_write().await?;
        let created = self.insert_shipment(&transaction, tenant, request, &method, &print).await;
        let row = match created {
            Ok(row) => row,
            Err(error) if error.code() == "shipping.request_duplicate" => {
                drop(transaction);
                let existing = self
                    .by_request(tenant.database().read(), tenant, request)
                    .await?
                    .ok_or(error)?;
                return same_request(existing, &print).map(|row| (row, false));
            }
            Err(error) => return Err(error),
        };
        self.emit(&transaction, tenant, actor, &[names::SHIPMENT_CREATED], None, &row).await?;
        if method.auto_book && method.kind == MethodKind::Carrier.as_str() {
            self.jobs
                .enqueue(
                    &transaction,
                    tenant,
                    &BookShipment { shipment_id: row.id },
                    EnqueueOptions::default().unique_key(format!("book:{}", row.id)),
                )
                .await?;
        }
        transaction.commit().await?;
        tracing::info!(shipment_id = %row.id, order_id = %row.order_id, "shipment created");
        Ok((row, true))
    }

    async fn by_request<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        request: &ShipmentRequest,
    ) -> Result<Option<shipment::Model>, Error> {
        Ok(shipment::Entity::find()
            .filter(shipment::Column::OrderId.eq(request.order_id))
            .filter(shipment::Column::RequestKey.eq(&request.request_key))
            .scoped(tenant)
            .one(connection)
            .await?)
    }

    async fn insert_shipment(
        &self,
        transaction: &DatabaseTransaction,
        tenant: &TenantContext,
        request: &ShipmentRequest,
        method: &crate::entity::method::Model,
        print: &str,
    ) -> Result<shipment::Model, Error> {
        let at = now();
        let id = Uuid::now_v7();
        let address = &request.address;
        let row = shipment::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(request.org_id.as_uuid()),
            origin_org_id: Set(request.origin_org_id.as_uuid()),
            order_id: Set(request.order_id),
            order_number: Set(request.order_number.clone()),
            request_key: Set(request.request_key.clone()),
            request_fingerprint: Set(print.to_owned()),
            direction: Set(request.direction.as_str().to_owned()),
            buyer_id: Set(request.buyer_id),
            method_id: Set(method.id),
            method_kind: Set(method.kind.clone()),
            carrier_account_id: Set(method.carrier_account_id),
            carrier_code: Set(None),
            service_code: Set(method.service_code.clone()),
            tracking_number: Set(None),
            status: Set(ShipmentStatus::Pending.as_str().to_owned()),
            status_at: Set(at),
            recipient_name: Set(request.recipient_name.clone()),
            recipient_phone: Set(request.recipient_phone.clone()),
            country: Set(address.country.clone()),
            province_code: Set(address.province_code.clone()),
            province_name: Set(address.province_name.clone()),
            commune_code: Set(address.commune_code.clone()),
            commune_name: Set(address.commune_name.clone()),
            line: Set(address.line.clone()),
            postal_code: Set(None),
            declared_value_minor: Set(request.declared_value.minor()),
            cod_minor: Set(request.cod_amount.map(Money::minor)),
            currency: Set(request.declared_value.currency().code().to_owned()),
            quoted_fee_minor: Set(request.quoted_fee.map(Money::minor)),
            actual_fee_minor: Set(None),
            cod_status: Set(if request.cod_amount.is_some_and(Money::is_positive) {
                "pending"
            } else {
                "none"
            }
            .to_owned()),
            delivered_at: Set(None),
            attempts: Set(0),
            last_error_code: Set(None),
            note: Set(request.note.chars().take(1000).collect()),
            created_at: Set(at),
            updated_at: Set(at),
        };
        let row = match row.insert(transaction).await {
            Ok(row) => row,
            Err(error) if unique_violation(&error).is_some() => {
                return Err(errors::request_duplicate());
            }
            Err(error) => return Err(error.into()),
        };
        // Một kiện chứa mọi món; nhân viên chia kiện lại bằng `PUT /shipments/{id}/parcels`.
        let weight: i64 = request
            .items
            .iter()
            .map(|item| item.weight_grams.unwrap_or(0).saturating_mul(item.quantity))
            // Cộng bão hoà (PT-5): cân nặng do người gọi gửi, không có trần; `.sum()` tràn là panic.
            .fold(0i64, i64::saturating_add);
        let parcel_id = Uuid::now_v7();
        parcel::ActiveModel {
            id: Set(parcel_id),
            tenant_id: Set(tenant.id().as_uuid()),
            shipment_id: Set(id),
            position: Set(1),
            weight_grams: Set(weight),
            length_mm: Set(None),
            width_mm: Set(None),
            height_mm: Set(None),
            tracking_number: Set(None),
        }
        .insert(transaction)
        .await?;
        for item in &request.items {
            shipment_item::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                shipment_id: Set(id),
                parcel_id: Set(Some(parcel_id)),
                order_line_key: Set(item.order_line_key.clone()),
                variant_id: Set(item.variant_id),
                sku: Set(item.sku.clone()),
                name: Set(item.name.clone()),
                quantity: Set(item.quantity),
                weight_grams: Set(item.weight_grams),
                length_mm: Set(item.length_mm),
                width_mm: Set(item.width_mm),
                height_mm: Set(item.height_mm),
            }
            .insert(transaction)
            .await?;
        }
        self.add_event(
            transaction,
            tenant,
            id,
            &StatusUpdate {
                event_key: "created".to_owned(),
                status: ShipmentStatus::Pending,
                carrier_status: String::new(),
                occurred_at: at,
                note: String::new(),
                source: Source::System,
            },
        )
        .await?;
        Ok(row)
    }

    /// Ghi một bước hành trình; đã có `event_key` → `false`.
    pub(crate) async fn add_event<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        shipment_id: Uuid,
        update: &StatusUpdate,
    ) -> Result<bool, Error> {
        let exists = tracking_event::Entity::find()
            .filter(tracking_event::Column::ShipmentId.eq(shipment_id))
            .filter(tracking_event::Column::EventKey.eq(&update.event_key))
            .scoped(tenant)
            .one(connection)
            .await?;
        if exists.is_some() {
            return Ok(false);
        }
        let inserted = tracking_event::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            shipment_id: Set(shipment_id),
            source: Set(update.source.as_str().to_owned()),
            event_key: Set(update.event_key.chars().take(200).collect()),
            status: Set(update.status.as_str().to_owned()),
            carrier_status: Set(update.carrier_status.chars().take(200).collect()),
            occurred_at: Set(crate::service::micros(update.occurred_at)),
            note: Set(update.note.chars().take(1000).collect()),
            created_at: Set(now()),
        }
        .insert(connection)
        .await;
        match inserted {
            Ok(_) => Ok(true),
            // Hai bản cùng `event_key` chạy cùng lúc: bản sau thua unique.
            Err(error) if unique_violation(&error).is_some() => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    /// Ghi event `names` cho thay đổi `before → after` (cùng transaction).
    pub(crate) async fn emit(
        &self,
        transaction: &DatabaseTransaction,
        tenant: &TenantContext,
        actor: Actor,
        names: &[&'static str],
        before: Option<&shipment::Model>,
        after: &shipment::Model,
    ) -> Result<(), Error> {
        let after_value = serde_json::to_value(event_of(after)).map_err(Error::internal)?;
        let before_value = before
            .map(|row| serde_json::to_value(event_of(row)))
            .transpose()
            .map_err(Error::internal)?;
        for name in names {
            let change = Change { before: before_value.clone(), after: Some(after_value.clone()) };
            self.outbox
                .record_named(
                    transaction,
                    tenant,
                    actor,
                    name,
                    events::shipment_subject(after.id),
                    &change,
                )
                .await?;
        }
        Ok(())
    }

    /// Áp một cập nhật lên vận đơn `id`. `Ok(true)` khi trạng thái đổi. Cập nhật đã có (`event_key`)
    /// → không làm gì. Cập nhật của hãng trái bảng / cũ hơn → chỉ ghi hành trình; của nhân viên trái
    /// bảng → `shipping.invalid_transition`.
    pub(crate) async fn apply_update(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
        update: StatusUpdate,
    ) -> Result<bool, Error> {
        let transaction = tenant.database().begin_write().await?;
        let before = lock_shipment(&transaction, tenant, id).await?;
        let current = status_of(&before);
        let moves = status::allowed(current, update.status);
        if !moves && update.source == Source::Staff {
            return Err(errors::invalid_transition(current, update.status));
        }
        if !self.add_event(&transaction, tenant, id, &update).await? {
            return Ok(false);
        }
        let older = update.occurred_at < before.status_at && update.source == Source::Carrier;
        if !moves || (older && !matches!(current, ShipmentStatus::DeliveryFailed)) {
            transaction.commit().await?;
            return Ok(false);
        }
        let changed = self
            .set_status(&transaction, tenant, actor, &before, update.status, update.occurred_at)
            .await?;
        transaction.commit().await?;
        Ok(changed)
    }

    /// Đổi trạng thái (đã kiểm bảng), ghi event, xếp job ghi sổ COD khi giao xong.
    pub(crate) async fn set_status(
        &self,
        transaction: &DatabaseTransaction,
        tenant: &TenantContext,
        actor: Actor,
        before: &shipment::Model,
        to: ShipmentStatus,
        at: Timestamp,
    ) -> Result<bool, Error> {
        let mut after = before.clone();
        after.status = to.as_str().to_owned();
        after.status_at = crate::service::micros(at.max(before.status_at));
        after.updated_at = now();
        let mut names = vec![names::SHIPMENT_STATUS_CHANGED];
        match to {
            ShipmentStatus::Delivered | ShipmentStatus::PickedUpByBuyer => {
                after.delivered_at = Some(after.status_at);
                if to == ShipmentStatus::Delivered {
                    names.push(names::SHIPMENT_DELIVERED);
                }
                if before.cod_status == "pending" {
                    after.cod_status = "collected".to_owned();
                    names.push(names::COD_COLLECTED);
                }
            }
            ShipmentStatus::DeliveryFailed => names.push(names::SHIPMENT_DELIVERY_FAILED),
            ShipmentStatus::Returned => names.push(names::SHIPMENT_RETURNED),
            ShipmentStatus::Cancelled => names.push(names::SHIPMENT_CANCELLED),
            _ => {}
        }
        let updated = shipment::Entity::update(
            sea_orm::IntoActiveModel::into_active_model(after.clone()).reset_all(),
        )
        .validate()?
        .filter(shipment::Column::Status.eq(&before.status))
        .scoped(tenant)
        .exec(transaction)
        .await;
        let after = match updated {
            Ok(row) => row,
            Err(sea_orm::DbErr::RecordNotUpdated) => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        self.emit(transaction, tenant, actor, &names, Some(before), &after).await?;
        if after.cod_status == "collected" && before.cod_status == "pending" {
            self.jobs
                .enqueue(
                    transaction,
                    tenant,
                    &PostCod { shipment_id: after.id },
                    EnqueueOptions::default().unique_key(format!("post_cod:{}", after.id)),
                )
                .await?;
        }
        Ok(true)
    }

    // ---- Đọc ---------------------------------------------------------------------------

    pub(crate) async fn list_shipments(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: ShipmentFilter,
        page: &PageRequest,
    ) -> Result<Page<shipment::Model>, Error> {
        let mut condition = Condition::all().add(principal.scope_condition(
            &SHIPMENTS_READ,
            shipment::Column::OrgId,
            None,
        )?);
        if let Some(order) = filter.order_id {
            condition = condition.add(shipment::Column::OrderId.eq(order));
        }
        if let Some(status) = filter.status {
            condition = condition.add(shipment::Column::Status.eq(status.as_str()));
        }
        if let Some(org) = filter.org_id {
            condition = condition.add(shipment::Column::OrgId.eq(org));
        }
        if let Some(tracking) = filter.tracking {
            condition = condition.add(shipment::Column::TrackingNumber.eq(tracking));
        }
        let select = shipment::Entity::find().filter(condition).scoped(tenant);
        fetch_page(select, shipment::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    pub(crate) async fn get_shipment(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<ShipmentView, Error> {
        let read = tenant.database().read();
        let row = find_shipment(read, tenant, id).await?;
        readable(principal, read, &SHIPMENTS_READ, row.org_id, || errors::shipment_not_found(id))
            .await?;
        self.view_of(read, tenant, row).await
    }

    pub(crate) async fn view_of<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        shipment: shipment::Model,
    ) -> Result<ShipmentView, Error> {
        let parcels = parcel::Entity::find()
            .filter(parcel::Column::ShipmentId.eq(shipment.id))
            .scoped(tenant)
            .order_by_asc(parcel::Column::Position)
            .all(connection)
            .await?;
        let items = shipment_item::Entity::find()
            .filter(shipment_item::Column::ShipmentId.eq(shipment.id))
            .scoped(tenant)
            .order_by_asc(shipment_item::Column::Id)
            .all(connection)
            .await?;
        let events = tracking_event::Entity::find()
            .filter(tracking_event::Column::ShipmentId.eq(shipment.id))
            .scoped(tenant)
            .order_by_asc(tracking_event::Column::OccurredAt)
            .order_by_asc(tracking_event::Column::Id)
            .all(connection)
            .await?;
        Ok(ShipmentView { shipment, parcels, items, events })
    }

    // ---- Nhân viên ---------------------------------------------------------------------

    /// Nhân viên tạo vận đơn tay (bán tại quầy giao sau). Cần `shipping.shipments.manage` ở nút
    /// bán.
    pub(crate) async fn create_shipment(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        request: ShipmentRequest,
    ) -> Result<(ShipmentView, bool), Error> {
        principal
            .require_write_at(
                tenant.database().write(),
                &SHIPMENTS_MANAGE,
                OrgTarget::new(request.org_id, None),
            )
            .await?;
        let (row, created) =
            self.receive_request(tenant, Actor::user(principal.user_id()), &request).await?;
        let view = self.view_of(tenant.database().read(), tenant, row).await?;
        Ok((view, created))
    }

    /// Nhân viên đổi trạng thái tay (tự giao, nhận tại cửa hàng).
    pub(crate) async fn staff_status(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        to: ShipmentStatus,
        note: String,
    ) -> Result<ShipmentView, Error> {
        let row = find_shipment(tenant.database().read(), tenant, id).await?;
        writable(principal, tenant.database().write(), &SHIPMENTS_MANAGE, row.org_id, || {
            errors::shipment_not_found(id)
        })
        .await?;
        let at = now();
        let update = StatusUpdate {
            event_key: format!("staff:{}", Uuid::now_v7()),
            status: to,
            carrier_status: String::new(),
            occurred_at: at,
            note,
            source: Source::Staff,
        };
        self.apply_update(tenant, Actor::user(principal.user_id()), id, update).await?;
        self.get_shipment(tenant, principal, id).await
    }
}

fn same_request(existing: shipment::Model, print: &str) -> Result<shipment::Model, Error> {
    if existing.request_fingerprint == print {
        Ok(existing)
    } else {
        tracing::error!(
            shipment_id = %existing.id,
            order_id = %existing.order_id,
            "a shipment request reused a request key with different content; it was ignored"
        );
        Err(errors::request_conflict(&existing.request_key))
    }
}

pub(crate) async fn find_shipment<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    id: Uuid,
) -> Result<shipment::Model, Error> {
    shipment::Entity::find_by_id(id)
        .scoped(tenant)
        .one(connection)
        .await?
        .ok_or_else(|| errors::shipment_not_found(id))
}

/// Đọc vận đơn trong transaction ghi, khoá dòng (Postgres `FOR UPDATE`; SQLite đã giữ quyền ghi).
pub(crate) async fn lock_shipment(
    transaction: &DatabaseTransaction,
    tenant: &TenantContext,
    id: Uuid,
) -> Result<shipment::Model, Error> {
    use sea_orm::QuerySelect;
    let mut select = shipment::Entity::find_by_id(id).scoped(tenant);
    if transaction.get_database_backend() == sea_orm::DatabaseBackend::Postgres {
        select = select.lock_exclusive();
    }
    select.one(transaction).await?.ok_or_else(|| errors::shipment_not_found(id))
}

/// Địa chỉ người nhận của vận đơn.
pub(crate) fn address_of_shipment(row: &shipment::Model) -> Result<crate::api::Address, Error> {
    address_of(
        &row.country,
        &row.province_code,
        &row.province_name,
        &row.commune_code,
        &row.commune_name,
        &row.line,
    )
}
