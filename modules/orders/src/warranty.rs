//! Bảo hành (đặc tả 45 mục 14, Luật 19/2023/QH15 — trách nhiệm bảo hành; thời hạn do người bán công
//! bố, lưu theo dòng):
//!
//! - **Mở bảo hành** khi đơn giao xong (`orders.order_delivered`): mỗi dòng có `warranty_months`
//!   (chụp từ `catalog` lúc đặt) một bảo hành; sản phẩm theo serial thì mỗi serial đã ghi một bảo
//!   hành (`POST /{id}/lines/{line_id}/serials` lúc chuẩn bị hàng). Unique (dòng, serial / `-`) nên
//!   event giao lại không mở hai lần. Hạn tính từ lúc giao.
//! - **Tra cứu** theo serial hay số điện thoại người mua (khoá liên hệ theo khách, không lưu số).
//! - **Phiếu bảo hành**: tiếp nhận (còn hạn), số phiếu tuần tự, trạng thái xử lý theo bảng chuyển,
//!   hạn xử lý theo cài đặt `orders.warranty_claim_days` (cần đối chiếu quy định về thời hạn sửa
//!   chữa, NĐ 55/2024 — dữ liệu, không viết cứng); event `orders.warranty_claim_changed`.

use jiff::SignedDuration;
use std::sync::Arc;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::{Actor, Delivered, EventHandler, Subscription};
use autoxide_kernel::sms::PhoneNumber;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    Set,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{OrderStatus, events as names};
use crate::entity::{line, line_serial, warranty, warranty_claim};
use crate::errors;
use crate::events::{Change, WarrantyClaimChanged};
use crate::guests::DEFAULT_COUNTRY;
use crate::permissions::{MANAGE, WARRANTY_MANAGE};
use crate::policy::WARRANTY_CLAIM_DAYS;
use crate::service::{OrdersService, now};

pub(crate) const ACTIVE: &str = "active";
pub(crate) const EXPIRED: &str = "expired";

/// Bảng chuyển trạng thái của phiếu bảo hành.
const CLAIM_TRANSITIONS: [(&str, &str); 8] = [
    ("received", "inspecting"),
    ("received", "rejected"),
    ("inspecting", "repairing"),
    ("inspecting", "replacing"),
    ("inspecting", "rejected"),
    ("repairing", "ready"),
    ("replacing", "ready"),
    ("ready", "returned"),
];

pub(crate) fn subscriptions(service: &Arc<OrdersService>) -> Vec<Subscription> {
    vec![Subscription::new(names::ORDER_DELIVERED, Arc::new(OpenOnDelivery(service.clone())))]
}

struct OpenOnDelivery(Arc<OrdersService>);

#[async_trait::async_trait]
impl EventHandler for OpenOnDelivery {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(id) =
            event.payload["after"]["order_id"].as_str().and_then(|id| id.parse::<Uuid>().ok())
        else {
            return Ok(());
        };
        self.0.open_warranties(&event.tenant, id).await
    }
}

/// Số tháng bảo hành chụp trong dòng đơn.
fn months_of(row: &line::Model) -> Option<u32> {
    serde_json::from_str::<Value>(&row.snapshot)
        .ok()
        .and_then(|snapshot| snapshot["warranty_months"].as_u64())
        .and_then(|months| u32::try_from(months).ok())
        .filter(|months| *months > 0)
}

impl OrdersService {
    /// Mở bảo hành cho các dòng của đơn đã giao (gọi lại không mở thêm).
    pub(crate) async fn open_warranties(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<(), Error> {
        let row = self.order(tenant, id).await?;
        let Some(starts_at) = row.delivered_at else { return Ok(()) };
        let lines = line::Entity::find()
            .filter(line::Column::OrderId.eq(id))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        let serials = line_serial::Entity::find()
            .filter(line_serial::Column::OrderId.eq(id))
            .scoped(tenant)
            .all(tenant.database().read())
            .await?;
        let transaction = tenant.database().begin_write().await?;
        for item in &lines {
            let Some(months) = months_of(item) else { continue };
            let ends_at = i32::try_from(months)
                .ok()
                .and_then(|months| starts_at.checked_add_months(months))
                .unwrap_or(starts_at);
            let own: Vec<Option<String>> = {
                let found: Vec<Option<String>> = serials
                    .iter()
                    .filter(|serial| serial.line_id == item.id)
                    .map(|serial| Some(serial.serial.clone()))
                    .collect();
                if found.is_empty() { vec![None] } else { found }
            };
            for serial in own {
                let record = warranty::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    org_id: Set(row.org_id),
                    order_id: Set(id),
                    order_line_id: Set(item.id),
                    product_id: Set(item.product_id),
                    variant_id: Set(item.variant_id),
                    serial_key: Set(serial.clone().unwrap_or_else(|| "-".to_owned())),
                    serial: Set(serial),
                    buyer_user_id: Set(row.buyer_user_id),
                    buyer_key: Set(row.buyer_key.clone()),
                    starts_at: Set(starts_at),
                    ends_at: Set(ends_at),
                    status: Set(ACTIVE.to_owned()),
                    created_at: Set(now()),
                };
                warranty::Entity::insert(record)
                    .on_conflict_do_nothing()
                    .exec_without_returning(&transaction)
                    .await?;
            }
        }
        transaction.commit().await?;
        Ok(())
    }

    /// `POST /{id}/lines/{line_id}/serials` (`orders.manage`): ghi số serial của dòng (không vượt
    /// số lượng). Đơn đã giao thì mở bảo hành theo serial ngay.
    pub(crate) async fn record_serials(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        line_id: Uuid,
        serials: Vec<String>,
    ) -> Result<Vec<String>, Error> {
        principal.require(&MANAGE)?;
        let row = self.order(tenant, id).await?;
        let target = OrgTarget::new(OrgId::from_uuid(row.org_id), None);
        principal.require_at(tenant.database().read(), &MANAGE, target).await?;
        let item = line::Entity::find_by_id(line_id)
            .filter(line::Column::OrderId.eq(id))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(errors::order_not_found)?;
        let cleaned: Vec<String> = serials.iter().map(|serial| serial.trim().to_owned()).collect();
        let mut problems = Vec::new();
        for (index, serial) in cleaned.iter().enumerate() {
            if !(1..=100).contains(&serial.chars().count())
                || cleaned.iter().filter(|other| *other == serial).count() > 1
            {
                problems.push(FieldError::new(
                    format!("serials[{index}]"),
                    "invalid",
                    "is empty, too long or repeated",
                ));
            }
        }
        let existing = line_serial::Entity::find()
            .filter(line_serial::Column::LineId.eq(line_id))
            .scoped(tenant)
            .count(tenant.database().read())
            .await?;
        let new_count = i64::try_from(existing).unwrap_or(i64::MAX)
            + i64::try_from(cleaned.len()).unwrap_or(i64::MAX);
        if new_count > item.quantity {
            problems.push(
                FieldError::new(
                    "serials",
                    "too_many",
                    "must be at most {max} serials for this line",
                )
                .with_param("max", item.quantity),
            );
        }
        if !problems.is_empty() {
            return Err(errors::serials_invalid(problems));
        }
        let transaction = tenant.database().begin_write().await?;
        for serial in &cleaned {
            line_serial::Entity::insert(line_serial::ActiveModel {
                tenant_id: Set(tenant.id().as_uuid()),
                order_id: Set(id),
                line_id: Set(line_id),
                serial: Set(serial.clone()),
                created_at: Set(now()),
            })
            .on_conflict_do_nothing()
            .exec_without_returning(&transaction)
            .await?;
        }
        transaction.commit().await?;
        let status = OrderStatus::parse(&row.status).unwrap_or(OrderStatus::Cancelled);
        if matches!(status, OrderStatus::Delivered | OrderStatus::Completed) {
            // Đã có bảo hành theo dòng (chưa có serial): thay bằng bảo hành theo serial.
            warranty::Entity::delete_many()
                .filter(warranty::Column::OrderLineId.eq(line_id))
                .filter(warranty::Column::SerialKey.eq("-"))
                .scoped(tenant)
                .exec(tenant.database().write())
                .await?;
            self.open_warranties(tenant, id).await?;
        }
        let all: Vec<String> = line_serial::Entity::find()
            .filter(line_serial::Column::LineId.eq(line_id))
            .scoped(tenant)
            .order_by_asc(line_serial::Column::CreatedAt)
            .all(tenant.database().read())
            .await?
            .into_iter()
            .map(|row| row.serial)
            .collect();
        Ok(all)
    }

    /// `GET /warranties?serial=&phone=` (`orders.warranty.manage`, theo phạm vi nút bán).
    pub(crate) async fn find_warranties(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        serial: Option<&str>,
        phone: Option<&str>,
    ) -> Result<Vec<warranty::Model>, Error> {
        principal.require(&WARRANTY_MANAGE)?;
        let mut condition = Condition::any();
        if let Some(serial) = serial.map(str::trim).filter(|serial| !serial.is_empty()) {
            condition = condition.add(warranty::Column::Serial.eq(serial));
        }
        if let Some(phone) = phone {
            let Some(parsed) = PhoneNumber::parse(phone, DEFAULT_COUNTRY) else {
                return Err(Error::validation(vec![FieldError::new(
                    "phone",
                    "invalid",
                    "is not a phone number",
                )]));
            };
            let key = self.secret.phone_key(tenant.id(), &parsed).await?;
            condition = condition.add(warranty::Column::BuyerKey.eq(key));
        }
        if condition.is_empty() {
            return Err(Error::validation(vec![FieldError::new(
                "serial",
                "required",
                "give `serial` or `phone`",
            )]));
        }
        let scope = principal.scope_condition(&WARRANTY_MANAGE, warranty::Column::OrgId, None)?;
        Ok(warranty::Entity::find()
            .filter(condition)
            .filter(scope)
            .scoped(tenant)
            .order_by_desc(warranty::Column::Id)
            .all(tenant.database().read())
            .await?)
    }

    /// `GET /me/warranties`: bảo hành của người mua.
    pub(crate) async fn my_warranties(
        &self,
        tenant: &TenantContext,
        buyer: Uuid,
    ) -> Result<Vec<warranty::Model>, Error> {
        Ok(warranty::Entity::find()
            .filter(warranty::Column::BuyerUserId.eq(buyer))
            .scoped(tenant)
            .order_by_desc(warranty::Column::Id)
            .all(tenant.database().read())
            .await?)
    }

    /// `POST /warranty-claims`: tiếp nhận yêu cầu bảo hành tại nút `org` (còn hạn, chưa huỷ).
    pub(crate) async fn open_claim(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        warranty_id: Uuid,
        org: OrgId,
        issue: &str,
    ) -> Result<warranty_claim::Model, Error> {
        principal.require(&WARRANTY_MANAGE)?;
        principal
            .require_at(tenant.database().read(), &WARRANTY_MANAGE, OrgTarget::new(org, None))
            .await?;
        let issue = issue.trim();
        if !(1..=5000).contains(&issue.chars().count()) {
            return Err(Error::validation(vec![FieldError::new(
                "issue",
                "length",
                "must be 1 to 5000 characters",
            )]));
        }
        let found = warranty::Entity::find_by_id(warranty_id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(errors::warranty_not_found)?;
        let at = now();
        if found.status != ACTIVE || found.ends_at <= at {
            return Err(errors::warranty_expired(found.ends_at.date_utc().to_string()));
        }
        let days = self
            .setting(tenant, org, &WARRANTY_CLAIM_DAYS)
            .await?
            .as_ref()
            .and_then(Value::as_i64)
            .unwrap_or(15);
        let actor = Actor::user(principal.user_id());
        let user = principal.user_id();
        let (database, outbox) = (tenant.database(), &self.outbox);
        let issue = issue.to_owned();
        database
            .write_with_retry(move || {
                let issue = issue.clone();
                async move {
                    let transaction = database.begin_write().await?;
                    let year = at.date_utc().year();
                    let sequence = autoxide_kernel::numbering::next(
                        &transaction,
                        tenant.id(),
                        &format!("orders.warranty_claim:{year}"),
                    )
                    .await?;
                    let saved = warranty_claim::ActiveModel {
                        id: Set(Uuid::now_v7()),
                        tenant_id: Set(tenant.id().as_uuid()),
                        warranty_id: Set(warranty_id),
                        org_id: Set(org.as_uuid()),
                        number: Set(format!("BH{year}-{sequence:06}")),
                        status: Set("received".to_owned()),
                        issue: Set(issue),
                        resolution: Set(None),
                        due_at: Set(at + SignedDuration::from_hours(24 * days)),
                        created_by: Set(user),
                        created_at: Set(at),
                        updated_at: Set(at),
                    }
                    .insert(&transaction)
                    .await?;
                    record_claim_event(&transaction, tenant, outbox, actor, &saved, None).await?;
                    transaction.commit().await?;
                    Ok(saved)
                }
            })
            .await
    }

    /// `POST /warranty-claims/{id}/transitions`: chuyển theo bảng; gọi lại cùng đích không đổi gì.
    pub(crate) async fn transition_claim(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        to: &str,
        resolution: Option<String>,
    ) -> Result<warranty_claim::Model, Error> {
        principal.require(&WARRANTY_MANAGE)?;
        let found = warranty_claim::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(errors::warranty_claim_not_found)?;
        let target = OrgTarget::new(OrgId::from_uuid(found.org_id), None);
        principal.require_at(tenant.database().read(), &WARRANTY_MANAGE, target).await?;
        if found.status == to {
            return Ok(found);
        }
        if !CLAIM_TRANSITIONS.contains(&(found.status.as_str(), to)) {
            return Err(errors::warranty_claim_transition(&found.status, to));
        }
        let actor = Actor::user(principal.user_id());
        let (database, outbox) = (tenant.database(), &self.outbox);
        let to = to.to_owned();
        database
            .write_with_retry(move || {
                let (to, before) = (to.clone(), found.status.clone());
                let resolution = resolution.clone().or_else(|| found.resolution.clone());
                async move {
                    let transaction = database.begin_write().await?;
                    let changed = warranty_claim::Entity::update_many()
                        .col_expr(warranty_claim::Column::Status, Expr::value(to.clone()))
                        .col_expr(warranty_claim::Column::UpdatedAt, Expr::value(now()))
                        .col_expr(warranty_claim::Column::Resolution, Expr::value(resolution))
                        .filter(warranty_claim::Column::Id.eq(id))
                        .filter(warranty_claim::Column::Status.eq(before.as_str()))
                        .scoped(tenant)
                        .exec(&transaction)
                        .await?;
                    if changed.rows_affected == 0 {
                        return Err(errors::warranty_claim_transition(&before, &to));
                    }
                    let saved = warranty_claim::Entity::find_by_id(id)
                        .scoped(tenant)
                        .one(&transaction)
                        .await?
                        .ok_or_else(errors::warranty_claim_not_found)?;
                    record_claim_event(&transaction, tenant, outbox, actor, &saved, Some(&before))
                        .await?;
                    transaction.commit().await?;
                    Ok(saved)
                }
            })
            .await
    }

    /// Job `orders.expire_warranties`: bảo hành quá hạn → `expired` (mọi khách của database).
    pub(crate) async fn expire_warranties(
        &self,
        database: &autoxide_kernel::database::Database,
    ) -> Result<u64, Error> {
        let result = warranty::Entity::update_many()
            .col_expr(warranty::Column::Status, Expr::value(EXPIRED))
            .filter(warranty::Column::Status.eq(ACTIVE))
            .filter(warranty::Column::EndsAt.lte(now()))
            .exec(database.write())
            .await?;
        Ok(result.rows_affected)
    }
}

async fn record_claim_event(
    transaction: &impl sea_orm::ConnectionTrait,
    tenant: &TenantContext,
    outbox: &autoxide_kernel::event::Outbox,
    actor: Actor,
    saved: &warranty_claim::Model,
    before: Option<&str>,
) -> Result<(), Error> {
    let event = WarrantyClaimChanged(Change {
        before: before.map(|status| json!({"status": status})),
        after: Some(json!({
            "claim_id": saved.id, "tenant_id": saved.tenant_id, "org_id": saved.org_id,
            "warranty_id": saved.warranty_id, "number": saved.number, "status": saved.status,
            "due_at": saved.due_at,
        })),
    });
    outbox
        .record(
            transaction,
            tenant,
            actor,
            Some(format!("orders.warranty_claim:{}", saved.id)),
            &event,
        )
        .await
}
