//! Cấu hình đặt lịch của cửa hàng: dịch vụ (thời lượng, đệm, sức chứa, cọc), nhân viên / phòng /
//! thiết bị, lịch làm việc lặp theo thứ (giờ địa phương của nút) và ngoại lệ (nghỉ, ca thêm, UTC).
//! Cần `bookings.services.manage` ở nút của bản ghi.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::collections::{BTreeMap, HashSet};

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, Set,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::{exception, resource, schedule, service, service_resource};
use crate::events::{self, Change};
use crate::permissions::SERVICES_MANAGE;
use crate::service::{
    BookingsService, at, check_minutes, check_text, default_org, finish, hidden, now,
};

pub(crate) const PERFORMER: &str = "performer";
pub(crate) const REQUIRED: &str = "required";

/// Cách tính tiền cọc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Deposit {
    None,
    /// Phần trăm giá (1–100).
    Percent(i64),
    /// Số tiền cố định (đơn vị nhỏ nhất, cùng tiền tệ với giá).
    Amount(i64),
}

impl Deposit {
    pub(crate) fn kind(self) -> &'static str {
        match self {
            Deposit::None => "none",
            Deposit::Percent(_) => "percent",
            Deposit::Amount(_) => "amount",
        }
    }

    pub(crate) fn value(self) -> i64 {
        match self {
            Deposit::None => 0,
            Deposit::Percent(value) | Deposit::Amount(value) => value,
        }
    }

    pub(crate) fn from_columns(kind: &str, value: i64) -> Self {
        match kind {
            "percent" => Deposit::Percent(value),
            "amount" => Deposit::Amount(value),
            _ => Deposit::None,
        }
    }

    /// Tiền cọc cho giá `price` (phần trăm làm tròn xuống tới đơn vị nhỏ nhất).
    pub(crate) fn of(self, price: i64) -> i64 {
        match self {
            Deposit::None => 0,
            Deposit::Percent(percent) => price.saturating_mul(percent) / 100,
            Deposit::Amount(amount) => amount.min(price),
        }
    }
}

/// Dịch vụ cùng nhân viên làm được và tài nguyên cần.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServiceView {
    pub row: service::Model,
    pub performers: Vec<Uuid>,
    pub required: Vec<Uuid>,
}

impl ServiceView {
    pub(crate) fn deposit(&self) -> Deposit {
        Deposit::from_columns(&self.row.deposit_kind, self.row.deposit_value)
    }

    pub(crate) fn names(&self) -> BTreeMap<String, String> {
        serde_json::from_str(&self.row.names).unwrap_or_default()
    }

    pub(crate) fn price(&self) -> Result<Money, Error> {
        Money::from_columns(self.row.price_minor, &self.row.currency)
    }
}

/// Dữ liệu để tạo dịch vụ.
#[derive(Debug, Clone)]
pub(crate) struct NewService {
    pub org_id: Option<Uuid>,
    pub name: String,
    pub names: BTreeMap<String, String>,
    pub description: String,
    pub duration_minutes: i32,
    pub buffer_before_minutes: i32,
    pub buffer_after_minutes: i32,
    pub price: Money,
    pub deposit: Deposit,
    pub capacity: i32,
    pub performers: Vec<Uuid>,
    pub required: Vec<Uuid>,
}

/// Thay đổi cho dịch vụ; trường `None` giữ nguyên.
#[derive(Debug, Clone, Default)]
pub(crate) struct ServiceChanges {
    pub name: Option<String>,
    pub names: Option<BTreeMap<String, String>>,
    pub description: Option<String>,
    pub duration_minutes: Option<i32>,
    pub buffer_before_minutes: Option<i32>,
    pub buffer_after_minutes: Option<i32>,
    pub price: Option<Money>,
    pub deposit: Option<Deposit>,
    pub capacity: Option<i32>,
    pub active: Option<bool>,
    pub performers: Option<Vec<Uuid>>,
    pub required: Option<Vec<Uuid>>,
}

/// Dữ liệu để tạo tài nguyên.
#[derive(Debug, Clone)]
pub(crate) struct NewResource {
    pub org_id: Option<Uuid>,
    pub kind: String,
    pub name: String,
    pub user_id: Option<Uuid>,
}

/// Thay đổi cho tài nguyên.
#[derive(Debug, Clone, Default)]
pub(crate) struct ResourceChanges {
    pub name: Option<String>,
    /// `Some(None)`: bỏ gắn tài khoản.
    pub user_id: Option<Option<Uuid>>,
    pub active: Option<bool>,
}

/// Một khung giờ lặp do người dùng gửi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WeeklyInput {
    pub weekday: i16,
    pub start_minute: i32,
    pub end_minute: i32,
}

/// Một ngoại lệ mới.
#[derive(Debug, Clone)]
pub(crate) struct NewException {
    pub resource_id: Uuid,
    pub kind: String,
    pub starts_at: Timestamp,
    pub ends_at: Timestamp,
    pub reason: String,
}

pub(crate) const RESOURCE_KINDS: [&str; 3] = ["staff", "room", "equipment"];

pub(crate) fn service_not_found(id: Uuid) -> Error {
    Error::not_found("bookings.service_not_found", "There is no bookable service `{id}`.")
        .with_param("id", id)
}

pub(crate) fn resource_not_found(id: Uuid) -> Error {
    Error::not_found(
        "bookings.resource_not_found",
        "There is no staff member, room or equipment `{id}`.",
    )
    .with_param("id", id)
}

fn exception_not_found(id: Uuid) -> Error {
    Error::not_found("bookings.exception_not_found", "There is no schedule exception `{id}`.")
        .with_param("id", id)
}

fn service_snapshot(view: &ServiceView) -> Value {
    let row = &view.row;
    json!({
        "org_id": row.org_id, "name": row.name, "duration_minutes": row.duration_minutes,
        "buffer_before_minutes": row.buffer_before_minutes, "buffer_after_minutes": row.buffer_after_minutes,
        "price_minor": row.price_minor, "currency": row.currency, "deposit_kind": row.deposit_kind,
        "deposit_value": row.deposit_value, "capacity": row.capacity, "active": row.active,
        "performers": view.performers, "required": view.required,
    })
}

fn resource_snapshot(row: &resource::Model) -> Value {
    json!({"org_id": row.org_id, "kind": row.kind, "name": row.name, "user_id": row.user_id, "active": row.active})
}

/// Kiểm các trường số của dịch vụ, gom vào `problems`.
fn check_service_numbers(
    problems: &mut Vec<FieldError>,
    duration: i32,
    before: i32,
    after: i32,
    capacity: i32,
    price: Money,
    deposit: Deposit,
) {
    check_minutes(problems, "duration_minutes", duration, 5, 1440, 5);
    check_minutes(problems, "buffer_before_minutes", before, 0, 240, 5);
    check_minutes(problems, "buffer_after_minutes", after, 0, 240, 5);
    if !(1..=500).contains(&capacity) {
        problems.push(FieldError::new("capacity", "range", "must be 1 to 500"));
    }
    if price.is_negative() {
        problems.push(FieldError::new("price", "range", "must not be negative"));
    }
    match deposit {
        Deposit::Percent(percent) if !(1..=100).contains(&percent) => {
            problems.push(FieldError::new(
                "deposit.value",
                "range",
                "a percentage must be 1 to 100",
            ));
        }
        Deposit::Amount(amount) if amount <= 0 || amount > price.minor() => {
            problems.push(FieldError::new(
                "deposit.value",
                "range",
                "an amount must be more than 0 and at most the price",
            ));
        }
        _ => {}
    }
}

fn check_names(problems: &mut Vec<FieldError>, names: &BTreeMap<String, String>) {
    if names.len() > 20 {
        problems.push(FieldError::new("names", "length", "must list at most 20 languages"));
    }
    for (locale, name) in names {
        if locale.is_empty() || locale.len() > 35 {
            problems.push(FieldError::new(
                format!("names.{locale}"),
                "invalid",
                "is not a language tag",
            ));
        }
        check_text(problems, &format!("names.{locale}"), name, 1, 200);
    }
}

impl BookingsService {
    // ---- Dịch vụ -----------------------------------------------------------------------------

    /// Liên kết tài nguyên của các dịch vụ `ids`.
    async fn links<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        ids: &[Uuid],
    ) -> Result<Vec<service_resource::Model>, Error> {
        Ok(service_resource::Entity::find()
            .filter(service_resource::Column::ServiceId.is_in(ids.iter().copied()))
            .scoped(tenant)
            .order_by_asc(service_resource::Column::ResourceId)
            .all(connection)
            .await?)
    }

    async fn views<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        rows: Vec<service::Model>,
    ) -> Result<Vec<ServiceView>, Error> {
        let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
        let links = Self::links(connection, tenant, &ids).await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let pick = |role: &str| {
                    links
                        .iter()
                        .filter(|link| link.service_id == row.id && link.role == role)
                        .map(|link| link.resource_id)
                        .collect()
                };
                ServiceView { performers: pick(PERFORMER), required: pick(REQUIRED), row }
            })
            .collect())
    }

    /// Dịch vụ `id` (mọi trạng thái), không kiểm quyền — cho đường công khai và đặt chỗ.
    pub(crate) async fn service_view<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<ServiceView, Error> {
        let row = service::Entity::find_by_id(id)
            .scoped(tenant)
            .one(connection)
            .await?
            .ok_or_else(|| service_not_found(id))?;
        Self::views(connection, tenant, vec![row]).await?.pop().ok_or_else(|| service_not_found(id))
    }

    /// Kiểm danh sách tài nguyên của dịch vụ: có thật, không trùng, không vừa làm vừa cần.
    async fn check_links<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        performers: &[Uuid],
        required: &[Uuid],
        problems: &mut Vec<FieldError>,
    ) -> Result<(), Error> {
        if performers.is_empty() {
            problems.push(FieldError::new(
                "performers",
                "required",
                "list at least one resource that gives the service",
            ));
        }
        if performers.len() + required.len() > 100 {
            problems.push(FieldError::new(
                "performers",
                "length",
                "a service can use at most 100 resources",
            ));
        }
        let mut seen = HashSet::new();
        for (field, ids) in [("performers", performers), ("required", required)] {
            for (index, id) in ids.iter().enumerate() {
                if !seen.insert(*id) {
                    problems.push(FieldError::new(
                        format!("{field}[{index}]"),
                        "duplicate",
                        "is listed twice",
                    ));
                }
            }
        }
        let all: Vec<Uuid> = performers.iter().chain(required).copied().collect();
        let found: HashSet<Uuid> = resource::Entity::find()
            .filter(resource::Column::Id.is_in(all.iter().copied()))
            .scoped(tenant)
            .all(connection)
            .await?
            .into_iter()
            .map(|row| row.id)
            .collect();
        for (field, ids) in [("performers", performers), ("required", required)] {
            for (index, id) in ids.iter().enumerate() {
                if !found.contains(id) {
                    problems.push(FieldError::new(
                        format!("{field}[{index}]"),
                        "not_found",
                        "no such resource",
                    ));
                }
            }
        }
        Ok(())
    }

    async fn save_links<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        service_id: Uuid,
        performers: &[Uuid],
        required: &[Uuid],
    ) -> Result<(), Error> {
        service_resource::Entity::delete_many()
            .filter(service_resource::Column::ServiceId.eq(service_id))
            .scoped(tenant)
            .exec(connection)
            .await?;
        let rows: Vec<service_resource::ActiveModel> = performers
            .iter()
            .map(|id| (id, PERFORMER))
            .chain(required.iter().map(|id| (id, REQUIRED)))
            .map(|(id, role)| service_resource::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                service_id: Set(service_id),
                resource_id: Set(*id),
                role: Set(role.to_owned()),
            })
            .collect();
        if !rows.is_empty() {
            service_resource::Entity::insert_many(rows).exec(connection).await?;
        }
        Ok(())
    }

    pub(crate) async fn create_service(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewService,
    ) -> Result<ServiceView, Error> {
        principal.require(&SERVICES_MANAGE)?;
        let mut problems = Vec::new();
        check_text(&mut problems, "name", &input.name, 1, 200);
        check_names(&mut problems, &input.names);
        check_text(&mut problems, "description", &input.description, 0, 2000);
        check_service_numbers(
            &mut problems,
            input.duration_minutes,
            input.buffer_before_minutes,
            input.buffer_after_minutes,
            input.capacity,
            input.price,
            input.deposit,
        );
        let org = match input.org_id {
            Some(org) => OrgId::from_uuid(org),
            None => default_org(tenant, principal, &SERVICES_MANAGE)?,
        };
        let transaction = tenant.database().begin_write().await?;
        principal.require_write_at(&transaction, &SERVICES_MANAGE, at(org.as_uuid())).await?;
        Self::check_links(&transaction, tenant, &input.performers, &input.required, &mut problems)
            .await?;
        finish(problems)?;
        let at_now = now();
        let row = service::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(org.as_uuid()),
            name: Set(input.name.trim().to_owned()),
            names: Set(serde_json::to_string(&input.names).map_err(Error::internal)?),
            description: Set(input.description.trim().to_owned()),
            duration_minutes: Set(input.duration_minutes),
            buffer_before_minutes: Set(input.buffer_before_minutes),
            buffer_after_minutes: Set(input.buffer_after_minutes),
            price_minor: Set(input.price.minor()),
            currency: Set(input.price.currency().code().to_owned()),
            deposit_kind: Set(input.deposit.kind().to_owned()),
            deposit_value: Set(input.deposit.value()),
            capacity: Set(input.capacity),
            active: Set(true),
            created_at: Set(at_now),
            updated_at: Set(at_now),
        }
        .insert(&transaction)
        .await?;
        Self::save_links(&transaction, tenant, row.id, &input.performers, &input.required).await?;
        let view = ServiceView { row, performers: input.performers, required: input.required };
        let event =
            events::ServiceCreated(Change { before: None, after: Some(service_snapshot(&view)) });
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::service_subject(view.row.id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        Ok(view)
    }

    pub(crate) async fn update_service(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        changes: ServiceChanges,
    ) -> Result<ServiceView, Error> {
        principal.require(&SERVICES_MANAGE)?;
        let transaction = tenant.database().begin_write().await?;
        let before = Self::service_view(&transaction, tenant, id).await?;
        hidden(
            principal.require_at(&transaction, &SERVICES_MANAGE, at(before.row.org_id)).await,
            || service_not_found(id),
        )?;
        principal.require_write_at(&transaction, &SERVICES_MANAGE, at(before.row.org_id)).await?;
        let row = &before.row;
        let price = changes.price.map_or_else(|| before.price(), Ok)?;
        let deposit = changes.deposit.unwrap_or_else(|| before.deposit());
        let mut problems = Vec::new();
        if let Some(name) = &changes.name {
            check_text(&mut problems, "name", name, 1, 200);
        }
        if let Some(names) = &changes.names {
            check_names(&mut problems, names);
        }
        if let Some(description) = &changes.description {
            check_text(&mut problems, "description", description, 0, 2000);
        }
        if changes.price.is_some() && price.currency().code() != row.currency {
            // Đổi tiền tệ là dịch vụ khác: lịch đã đặt giữ giá cũ, không đổi chéo được.
            problems.push(
                FieldError::new("price.currency", "invalid", "must stay {currency}")
                    .with_param("currency", &row.currency),
            );
        }
        check_service_numbers(
            &mut problems,
            changes.duration_minutes.unwrap_or(row.duration_minutes),
            changes.buffer_before_minutes.unwrap_or(row.buffer_before_minutes),
            changes.buffer_after_minutes.unwrap_or(row.buffer_after_minutes),
            changes.capacity.unwrap_or(row.capacity),
            price,
            deposit,
        );
        let performers = changes.performers.clone().unwrap_or_else(|| before.performers.clone());
        let required = changes.required.clone().unwrap_or_else(|| before.required.clone());
        if changes.performers.is_some() || changes.required.is_some() {
            Self::check_links(&transaction, tenant, &performers, &required, &mut problems).await?;
        }
        finish(problems)?;
        let mut active = row.clone().into_active_model();
        if let Some(name) = changes.name {
            active.name = Set(name.trim().to_owned());
        }
        if let Some(names) = changes.names {
            active.names = Set(serde_json::to_string(&names).map_err(Error::internal)?);
        }
        if let Some(description) = changes.description {
            active.description = Set(description.trim().to_owned());
        }
        if let Some(value) = changes.duration_minutes {
            active.duration_minutes = Set(value);
        }
        if let Some(value) = changes.buffer_before_minutes {
            active.buffer_before_minutes = Set(value);
        }
        if let Some(value) = changes.buffer_after_minutes {
            active.buffer_after_minutes = Set(value);
        }
        if let Some(value) = changes.capacity {
            active.capacity = Set(value);
        }
        if let Some(value) = changes.active {
            active.active = Set(value);
        }
        active.price_minor = Set(price.minor());
        active.deposit_kind = Set(deposit.kind().to_owned());
        active.deposit_value = Set(deposit.value());
        active.updated_at = Set(now());
        let saved =
            service::Entity::update(active).validate()?.scoped(tenant).exec(&transaction).await?;
        if changes.performers.is_some() || changes.required.is_some() {
            Self::save_links(&transaction, tenant, id, &performers, &required).await?;
        }
        let after = ServiceView { row: saved, performers, required };
        let event = events::ServiceUpdated(Change {
            before: Some(service_snapshot(&before)),
            after: Some(service_snapshot(&after)),
        });
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::service_subject(id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        Ok(after)
    }

    pub(crate) async fn get_service(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<ServiceView, Error> {
        principal.require(&SERVICES_MANAGE)?;
        let read = tenant.database().read();
        let view = Self::service_view(read, tenant, id).await?;
        hidden(principal.require_at(read, &SERVICES_MANAGE, at(view.row.org_id)).await, || {
            service_not_found(id)
        })?;
        Ok(view)
    }

    pub(crate) async fn list_services(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<ServiceView>, Error> {
        let scope = principal.scope_condition(&SERVICES_MANAGE, service::Column::OrgId, None)?;
        let read = tenant.database().read();
        let select = service::Entity::find().filter(scope).scoped(tenant);
        let page = fetch_page(select, service::Column::Id, |row| row.id, page, read).await?;
        let views = Self::views(read, tenant, page.data).await?;
        Ok(autoxide_kernel::pagination::Page { data: views, pagination: page.pagination })
    }

    /// Dịch vụ đang nhận đặt (công khai), tuỳ chọn chỉ của nút `org`, sắp theo tên.
    pub(crate) async fn public_services(
        &self,
        tenant: &TenantContext,
        org: Option<Uuid>,
    ) -> Result<Vec<ServiceView>, Error> {
        let read = tenant.database().read();
        let mut select =
            service::Entity::find().filter(service::Column::Active.eq(true)).scoped(tenant);
        if let Some(org) = org {
            select = select.filter(service::Column::OrgId.eq(org));
        }
        let rows = select.order_by_asc(service::Column::Name).limit_rows(500).all(read).await?;
        Self::views(read, tenant, rows).await
    }

    // ---- Tài nguyên --------------------------------------------------------------------------

    pub(crate) async fn resource_row<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<resource::Model, Error> {
        resource::Entity::find_by_id(id)
            .scoped(tenant)
            .one(connection)
            .await?
            .ok_or_else(|| resource_not_found(id))
    }

    pub(crate) async fn create_resource(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewResource,
    ) -> Result<resource::Model, Error> {
        principal.require(&SERVICES_MANAGE)?;
        let mut problems = Vec::new();
        check_text(&mut problems, "name", &input.name, 1, 200);
        if !RESOURCE_KINDS.contains(&input.kind.as_str()) {
            problems.push(FieldError::new(
                "kind",
                "invalid",
                "must be `staff`, `room` or `equipment`",
            ));
        }
        if input.user_id.is_some() && input.kind != "staff" {
            problems.push(FieldError::new(
                "user_id",
                "invalid",
                "only staff can be linked to an account",
            ));
        }
        finish(problems)?;
        let org = match input.org_id {
            Some(org) => OrgId::from_uuid(org),
            None => default_org(tenant, principal, &SERVICES_MANAGE)?,
        };
        let transaction = tenant.database().begin_write().await?;
        principal.require_write_at(&transaction, &SERVICES_MANAGE, at(org.as_uuid())).await?;
        let at_now = now();
        let row = resource::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(org.as_uuid()),
            kind: Set(input.kind),
            name: Set(input.name.trim().to_owned()),
            user_id: Set(input.user_id),
            active: Set(true),
            created_at: Set(at_now),
            updated_at: Set(at_now),
        }
        .insert(&transaction)
        .await?;
        let event =
            events::ResourceCreated(Change { before: None, after: Some(resource_snapshot(&row)) });
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::resource_subject(row.id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        Ok(row)
    }

    pub(crate) async fn update_resource(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        changes: ResourceChanges,
    ) -> Result<resource::Model, Error> {
        principal.require(&SERVICES_MANAGE)?;
        let mut problems = Vec::new();
        if let Some(name) = &changes.name {
            check_text(&mut problems, "name", name, 1, 200);
        }
        let transaction = tenant.database().begin_write().await?;
        let before = Self::resource_row(&transaction, tenant, id).await?;
        hidden(
            principal.require_at(&transaction, &SERVICES_MANAGE, at(before.org_id)).await,
            || resource_not_found(id),
        )?;
        principal.require_write_at(&transaction, &SERVICES_MANAGE, at(before.org_id)).await?;
        if matches!(changes.user_id, Some(Some(_))) && before.kind != "staff" {
            problems.push(FieldError::new(
                "user_id",
                "invalid",
                "only staff can be linked to an account",
            ));
        }
        finish(problems)?;
        let mut active = before.clone().into_active_model();
        if let Some(name) = changes.name {
            active.name = Set(name.trim().to_owned());
        }
        if let Some(user) = changes.user_id {
            active.user_id = Set(user);
        }
        if let Some(value) = changes.active {
            active.active = Set(value);
        }
        active.updated_at = Set(now());
        let after =
            resource::Entity::update(active).validate()?.scoped(tenant).exec(&transaction).await?;
        let event = events::ResourceUpdated(Change {
            before: Some(resource_snapshot(&before)),
            after: Some(resource_snapshot(&after)),
        });
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::resource_subject(id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        Ok(after)
    }

    pub(crate) async fn get_resource(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<resource::Model, Error> {
        principal.require(&SERVICES_MANAGE)?;
        let read = tenant.database().read();
        let row = Self::resource_row(read, tenant, id).await?;
        hidden(principal.require_at(read, &SERVICES_MANAGE, at(row.org_id)).await, || {
            resource_not_found(id)
        })?;
        Ok(row)
    }

    pub(crate) async fn list_resources(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<resource::Model>, Error> {
        let scope = principal.scope_condition(&SERVICES_MANAGE, resource::Column::OrgId, None)?;
        let select = resource::Entity::find().filter(scope).scoped(tenant);
        fetch_page(select, resource::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    // ---- Lịch làm việc ------------------------------------------------------------------------

    pub(crate) async fn schedule(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        resource_id: Uuid,
    ) -> Result<Vec<schedule::Model>, Error> {
        self.get_resource(tenant, principal, resource_id).await?;
        Self::schedule_rows(tenant.database().read(), tenant, &[resource_id]).await
    }

    pub(crate) async fn schedule_rows<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        resources: &[Uuid],
    ) -> Result<Vec<schedule::Model>, Error> {
        Ok(schedule::Entity::find()
            .filter(schedule::Column::ResourceId.is_in(resources.iter().copied()))
            .scoped(tenant)
            .order_by_asc(schedule::Column::Weekday)
            .order_by_asc(schedule::Column::StartMinute)
            .all(connection)
            .await?)
    }

    /// Thay cả lịch tuần của tài nguyên. Khung trong cùng một ngày không chồng nhau.
    pub(crate) async fn replace_schedule(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        resource_id: Uuid,
        ranges: Vec<WeeklyInput>,
    ) -> Result<Vec<schedule::Model>, Error> {
        principal.require(&SERVICES_MANAGE)?;
        let mut problems = Vec::new();
        if ranges.len() > 70 {
            problems.push(FieldError::new("ranges", "length", "must list at most 70 ranges"));
        }
        for (index, range) in ranges.iter().enumerate() {
            if !(1..=7).contains(&range.weekday) {
                problems.push(FieldError::new(
                    format!("ranges[{index}].weekday"),
                    "range",
                    "must be 1 (Monday) to 7 (Sunday)",
                ));
            }
            if range.start_minute < 0 || range.start_minute % 5 != 0 || range.start_minute >= 1440 {
                problems.push(FieldError::new(
                    format!("ranges[{index}].start_minute"),
                    "range",
                    "must be 0 to 1435, in steps of 5",
                ));
            }
            if range.end_minute <= range.start_minute
                || range.end_minute > 1440
                || range.end_minute % 5 != 0
            {
                problems.push(FieldError::new(
                    format!("ranges[{index}].end_minute"),
                    "range",
                    "must be after `start_minute`, at most 1440, in steps of 5",
                ));
            }
        }
        let mut sorted = ranges.clone();
        sorted.sort_by_key(|range| (range.weekday, range.start_minute));
        if sorted.windows(2).any(|pair| {
            pair[0].weekday == pair[1].weekday && pair[1].start_minute < pair[0].end_minute
        }) {
            problems.push(FieldError::new(
                "ranges",
                "overlap",
                "ranges of the same day must not overlap",
            ));
        }
        finish(problems)?;
        let transaction = tenant.database().begin_write().await?;
        let row = Self::resource_row(&transaction, tenant, resource_id).await?;
        hidden(principal.require_at(&transaction, &SERVICES_MANAGE, at(row.org_id)).await, || {
            resource_not_found(resource_id)
        })?;
        principal.require_write_at(&transaction, &SERVICES_MANAGE, at(row.org_id)).await?;
        let before = Self::schedule_rows(&transaction, tenant, &[resource_id]).await?;
        schedule::Entity::delete_many()
            .filter(schedule::Column::ResourceId.eq(resource_id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let rows: Vec<schedule::ActiveModel> = sorted
            .iter()
            .map(|range| schedule::ActiveModel {
                id: Set(Uuid::now_v7()),
                tenant_id: Set(tenant.id().as_uuid()),
                resource_id: Set(resource_id),
                weekday: Set(range.weekday),
                start_minute: Set(range.start_minute),
                end_minute: Set(range.end_minute),
            })
            .collect();
        if !rows.is_empty() {
            schedule::Entity::insert_many(rows).exec(&transaction).await?;
        }
        let after = Self::schedule_rows(&transaction, tenant, &[resource_id]).await?;
        let snapshot = |rows: &[schedule::Model]| {
            Value::from(
                rows.iter()
                    .map(|row| json!([row.weekday, row.start_minute, row.end_minute]))
                    .collect::<Vec<_>>(),
            )
        };
        let event = events::ScheduleReplaced(Change {
            before: Some(snapshot(&before)),
            after: Some(snapshot(&after)),
        });
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::resource_subject(resource_id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        Ok(after)
    }

    // ---- Ngoại lệ ----------------------------------------------------------------------------

    pub(crate) async fn create_exception(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewException,
    ) -> Result<exception::Model, Error> {
        principal.require(&SERVICES_MANAGE)?;
        let mut problems = Vec::new();
        if !["closed", "open"].contains(&input.kind.as_str()) {
            problems.push(FieldError::new(
                "kind",
                "invalid",
                "must be `closed` (time off) or `open` (extra hours)",
            ));
        }
        if input.ends_at <= input.starts_at {
            problems.push(FieldError::new("ends_at", "range", "must be after `starts_at`"));
        } else if input.ends_at - input.starts_at > SignedDuration::from_hours(24 * 366) {
            problems.push(FieldError::new(
                "ends_at",
                "range",
                "an exception lasts at most 366 days",
            ));
        }
        check_text(&mut problems, "reason", &input.reason, 0, 200);
        finish(problems)?;
        let transaction = tenant.database().begin_write().await?;
        let resource = Self::resource_row(&transaction, tenant, input.resource_id).await?;
        hidden(
            principal.require_at(&transaction, &SERVICES_MANAGE, at(resource.org_id)).await,
            || resource_not_found(input.resource_id),
        )?;
        principal.require_write_at(&transaction, &SERVICES_MANAGE, at(resource.org_id)).await?;
        let row = exception::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            resource_id: Set(input.resource_id),
            kind: Set(input.kind),
            starts_at: Set(input.starts_at),
            ends_at: Set(input.ends_at),
            reason: Set(input.reason.trim().to_owned()),
            created_at: Set(now()),
        }
        .insert(&transaction)
        .await?;
        let event = events::ExceptionCreated(Change {
            before: None,
            after: Some(exception_snapshot(&row)),
        });
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::resource_subject(row.resource_id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        Ok(row)
    }

    pub(crate) async fn delete_exception(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        principal.require(&SERVICES_MANAGE)?;
        let transaction = tenant.database().begin_write().await?;
        let row = exception::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| exception_not_found(id))?;
        let resource = Self::resource_row(&transaction, tenant, row.resource_id).await?;
        hidden(
            principal.require_at(&transaction, &SERVICES_MANAGE, at(resource.org_id)).await,
            || exception_not_found(id),
        )?;
        principal.require_write_at(&transaction, &SERVICES_MANAGE, at(resource.org_id)).await?;
        let deleted = exception::Entity::delete_many()
            .filter(exception::Column::Id.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        if deleted.rows_affected == 0 {
            transaction.rollback().await?;
            return Err(exception_not_found(id));
        }
        let event = events::ExceptionDeleted(Change {
            before: Some(exception_snapshot(&row)),
            after: None,
        });
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::resource_subject(row.resource_id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Ngoại lệ của tài nguyên chạm khoảng [from, to).
    pub(crate) async fn exceptions(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        resource_id: Uuid,
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<exception::Model>, Error> {
        self.get_resource(tenant, principal, resource_id).await?;
        Self::exception_rows(tenant.database().read(), tenant, &[resource_id], from, to).await
    }

    pub(crate) async fn exception_rows<C: ConnectionTrait>(
        connection: &C,
        tenant: &TenantContext,
        resources: &[Uuid],
        from: Timestamp,
        to: Timestamp,
    ) -> Result<Vec<exception::Model>, Error> {
        Ok(exception::Entity::find()
            .filter(exception::Column::ResourceId.is_in(resources.iter().copied()))
            .filter(exception::Column::StartsAt.lt(to))
            .filter(exception::Column::EndsAt.gt(from))
            .scoped(tenant)
            .order_by_asc(exception::Column::StartsAt)
            .limit_rows(2000)
            .all(connection)
            .await?)
    }
}

fn exception_snapshot(row: &exception::Model) -> Value {
    json!({"id": row.id, "kind": row.kind, "starts_at": row.starts_at, "ends_at": row.ends_at, "reason": row.reason})
}

/// `.limit(n)` cho truy vấn trả nhiều dòng (tên riêng để không lẫn với giới hạn của trang).
trait LimitRows: Sized {
    fn limit_rows(self, limit: u64) -> Self;
}

impl<E: EntityTrait> LimitRows for sea_orm::Select<E> {
    fn limit_rows(self, limit: u64) -> Self {
        sea_orm::QuerySelect::limit(self, limit)
    }
}
