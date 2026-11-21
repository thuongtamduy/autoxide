//! Nhận tại cửa hàng (mục 5 của bước 45, D-201, B-c7, D-20261119-step45-pickup-preorder): nhân viên
//! báo đơn sẵn sàng → mã nhận 6 số gửi người mua (event `orders.pickup_ready`), giữ hàng tới
//! `hold_until` (`hold_days` của phương thức nhận tại cửa hàng ở `shipping`); giao bằng mã hay cho
//! người nhận hộ đã khai lúc đặt → đơn `delivered`. Job `orders.expire_pickups` nhắc một lần trước 24
//! giờ, hết hạn thì làm theo `orders.pickup_expired_action`.

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::database::Database;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, TenantId};
use jiff::SignedDuration;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::OrderStatus;
use crate::checkout::Contact;
use crate::entity::{order, pickup};
use crate::errors;
use crate::events::{Change, PickupExpired, PickupReady, PickupReminder};
use crate::jobs::BATCH;
use crate::lifecycle::{self, Applied, Cause, Transition};
use crate::permissions::MANAGE;
use crate::service::OrdersService;

pub(crate) const READY: &str = "ready";
pub(crate) const PICKED_UP: &str = "picked_up";
pub(crate) const EXPIRED: &str = "expired";
/// Số ngày giữ hàng khi phương thức không khai `hold_days`.
const DEFAULT_HOLD_DAYS: i64 = 3;

/// Ai nhận hàng ở quầy.
#[derive(Debug, Clone)]
pub(crate) enum Collector {
    /// Người mua đọc mã nhận.
    Code(String),
    /// Người nhận hộ đã khai lúc đặt hàng.
    Proxy { name: String, phone: String },
}

/// Mã nhận 6 số. Lấy từ phần ngẫu nhiên của UUIDv7 (62 bit cuối do bộ sinh ngẫu nhiên của hệ điều
/// hành điền): mã chỉ để đối chiếu ở quầy, nhân viên nhập — không phải secret đăng nhập.
fn new_code() -> String {
    let random = Uuid::now_v7().as_u128() & ((1u128 << 62) - 1);
    format!("{:06}", random % 1_000_000)
}

/// Chỉ giữ chữ số (so số điện thoại bỏ qua dấu cách, `+`, `-`); `0…` và `84…` coi là một.
fn phone_digits(phone: &str) -> String {
    let digits: String = phone.chars().filter(char::is_ascii_digit).collect();
    match digits.strip_prefix("84") {
        Some(rest) if digits.len() > 10 => format!("0{rest}"),
        _ => digits,
    }
}

fn same_name(left: &str, right: &str) -> bool {
    let normal = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    normal(left) == normal(right)
}

impl OrdersService {
    async fn pickup_order(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<order::Model, Error> {
        principal.require(&MANAGE)?;
        let row = self.order(tenant, id).await?;
        let target = OrgTarget::new(OrgId::from_uuid(row.org_id), None);
        principal.require_at(tenant.database().read(), &MANAGE, target).await?;
        if row.pickup_org_id.is_none() {
            let status = OrderStatus::parse(&row.status).unwrap_or(OrderStatus::Cancelled);
            return Err(errors::pickup_not_allowed(status));
        }
        Ok(row)
    }

    /// Dòng nhận tại cửa hàng của đơn `id`, nếu đã sẵn sàng.
    pub(crate) async fn pickup_of(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<Option<pickup::Model>, Error> {
        Ok(pickup::Entity::find()
            .filter(pickup::Column::OrderId.eq(id))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?)
    }

    /// Báo đơn nhận tại cửa hàng `id` đã sẵn sàng (`orders.manage`): nhận đơn nếu còn
    /// `confirmed`, tạo mã nhận, giữ hàng `hold_days`. Gọi lại khi đã sẵn sàng: kết quả cũ.
    pub(crate) async fn pickup_ready(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<pickup::Model, Error> {
        let row = self.pickup_order(tenant, principal, id).await?;
        let hold_days = row
            .shipping_option
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            .and_then(|option| option["hold_days"].as_i64())
            .filter(|days| *days > 0)
            .unwrap_or(DEFAULT_HOLD_DAYS);
        let actor = Actor::user(principal.user_id());
        let (database, outbox) = (tenant.database(), &self.outbox);
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let current = lifecycle::lock(&transaction, tenant, id).await?;
                if let Some(existing) = pickup::Entity::find()
                    .filter(pickup::Column::OrderId.eq(id))
                    .scoped(tenant)
                    .one(&transaction)
                    .await?
                {
                    return Ok(existing);
                }
                let status = OrderStatus::parse(&current.status).unwrap_or(OrderStatus::Cancelled);
                let current = match status {
                    OrderStatus::Confirmed => {
                        let accept = Transition::new(OrderStatus::Preparing, Cause::Staff, actor);
                        lifecycle::apply(&transaction, tenant, outbox, current, &accept, |_| {})
                            .await?;
                        lifecycle::lock(&transaction, tenant, id).await?
                    }
                    OrderStatus::Preparing => current,
                    other => return Err(errors::pickup_not_allowed(other)),
                };
                let now = crate::service::now();
                let pickup_org = current.pickup_org_id.unwrap_or(current.org_id);
                let saved = pickup::ActiveModel {
                    tenant_id: Set(tenant.id().as_uuid()),
                    order_id: Set(id),
                    pickup_org_id: Set(pickup_org),
                    status: Set(READY.to_owned()),
                    code: Set(new_code()),
                    ready_at: Set(now),
                    hold_until: Set(now + SignedDuration::from_hours(24 * hold_days)),
                    reminded_at: Set(None),
                    handed_over_at: Set(None),
                    handed_over_to: Set(None),
                    created_at: Set(now),
                    updated_at: Set(now),
                }
                .insert(&transaction)
                .await?;
                let event = PickupReady(Change::after(json!({
                    "order_id": id, "tenant_id": current.tenant_id, "org_id": current.org_id,
                    "channel": current.channel, "number": current.number,
                    "buyer_user_id": current.buyer_user_id, "pickup_org_id": pickup_org,
                    "pickup_code": saved.code, "hold_until": saved.hold_until,
                })));
                outbox.record(&transaction, tenant, actor, subject(id), &event).await?;
                transaction.commit().await?;
                Ok(saved)
            })
            .await
    }

    /// Giao đơn nhận tại cửa hàng `id` (`orders.manage`) cho người đọc đúng mã, hay cho người nhận
    /// hộ đã khai lúc đặt (tên và số điện thoại khớp) → đơn `delivered`.
    pub(crate) async fn pickup_handover(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        collector: Collector,
    ) -> Result<order::Model, Error> {
        self.pickup_order(tenant, principal, id).await?;
        let actor = Actor::user(principal.user_id());
        let (database, outbox, collector) = (tenant.database(), &self.outbox, &collector);
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let current = lifecycle::lock(&transaction, tenant, id).await?;
                let stored = pickup::Entity::find()
                    .filter(pickup::Column::OrderId.eq(id))
                    .scoped(tenant)
                    .one(&transaction)
                    .await?
                    .ok_or_else(errors::pickup_not_ready)?;
                if stored.status == PICKED_UP {
                    return Ok(current);
                }
                let handed_to = match collector {
                    Collector::Code(code) if code.trim() == stored.code => json!({"by": "code"}),
                    Collector::Proxy { name, phone } => {
                        let contact: Option<Contact> = serde_json::from_str(&current.contact).ok();
                        let matches = contact.is_some_and(|contact| {
                            contact.proxy_name.is_some_and(|proxy| same_name(&proxy, name))
                                && contact.proxy_phone.is_some_and(|proxy| {
                                    phone_digits(&proxy) == phone_digits(phone)
                                })
                        });
                        if !matches {
                            return Err(errors::pickup_code_invalid());
                        }
                        json!({"by": "proxy", "name": name, "phone": phone})
                    }
                    Collector::Code(_) => return Err(errors::pickup_code_invalid()),
                };
                let status = OrderStatus::parse(&current.status).unwrap_or(OrderStatus::Cancelled);
                if status != OrderStatus::Preparing {
                    return Err(errors::pickup_not_allowed(status));
                }
                let deliver = Transition::new(OrderStatus::Delivered, Cause::Staff, actor);
                lifecycle::apply(&transaction, tenant, outbox, current, &deliver, |_| {}).await?;
                let now = crate::service::now();
                let mut done: pickup::ActiveModel = stored.into();
                done.status = Set(PICKED_UP.to_owned());
                done.handed_over_at = Set(Some(now));
                done.handed_over_to = Set(Some(handed_to.to_string()));
                done.updated_at = Set(now);
                done.update(&transaction).await?;
                let delivered = lifecycle::lock(&transaction, tenant, id).await?;
                transaction.commit().await?;
                Ok(delivered)
            })
            .await
    }

    /// Job `orders.expire_pickups`: nhắc một lần khi còn dưới 24 giờ; hết hạn thì huỷ và hoàn
    /// tiền (`cancel_refund`, mặc định) hay chỉ đánh dấu hết hạn, vẫn giao được (`keep_waiting`).
    pub(crate) async fn expire_pickups(&self, database: &Database) -> Result<u64, Error> {
        let soon = crate::service::now() + SignedDuration::from_hours(24);
        let due: Vec<(Uuid, Uuid)> = pickup::Entity::find()
            .select_only()
            .column(pickup::Column::TenantId)
            .column(pickup::Column::OrderId)
            .filter(pickup::Column::Status.eq(READY))
            .filter(pickup::Column::HoldUntil.lte(soon))
            .order_by_asc(pickup::Column::HoldUntil)
            .limit(BATCH)
            .into_tuple()
            .all(database.read())
            .await?;
        let mut handled = 0;
        for (tenant_id, id) in due {
            let tenant = self.databases.tenant(TenantId::from_uuid(tenant_id)).await?;
            match self.expire_pickup(&tenant, id).await {
                Ok(true) => handled += 1,
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(order_id = %id, code = error.code(), "pickup not expired");
                }
            }
        }
        Ok(handled)
    }

    async fn expire_pickup(&self, tenant: &TenantContext, id: Uuid) -> Result<bool, Error> {
        let Some(stored) = self.pickup_of(tenant, id).await? else { return Ok(false) };
        let row = self.order(tenant, id).await?;
        let now = crate::service::now();
        if stored.status != READY {
            return Ok(false);
        }
        if stored.hold_until > now {
            if stored.reminded_at.is_some() {
                return Ok(false);
            }
            return self.remind_pickup(tenant, &row, stored).await;
        }
        let policy = self.policy(tenant, OrgId::from_uuid(row.org_id)).await?;
        let cancel = policy.pickup_cancel_on_expiry;
        let (database, outbox) = (tenant.database(), &self.outbox);
        let applied = database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let current = lifecycle::lock(&transaction, tenant, id).await?;
                let Some(latest) = pickup::Entity::find()
                    .filter(pickup::Column::OrderId.eq(id))
                    .filter(pickup::Column::Status.eq(READY))
                    .scoped(tenant)
                    .one(&transaction)
                    .await?
                else {
                    return Ok(None);
                };
                let mut changed = None;
                if cancel {
                    let transition =
                        Transition::new(OrderStatus::Cancelled, Cause::System, Actor::system())
                            .reason("pickup_expired");
                    let reason = transition.reason.clone();
                    changed = Some(
                        lifecycle::apply(
                            &transaction,
                            tenant,
                            outbox,
                            current.clone(),
                            &transition,
                            |row| {
                                row.cancel_reason = Set(reason);
                            },
                        )
                        .await?,
                    );
                }
                let mut expired: pickup::ActiveModel = latest.into();
                expired.status = Set(EXPIRED.to_owned());
                expired.updated_at = Set(crate::service::now());
                expired.update(&transaction).await?;
                let event = PickupExpired(Change::after(json!({
                    "order_id": id, "tenant_id": current.tenant_id, "org_id": current.org_id,
                    "channel": current.channel, "number": current.number,
                    "buyer_user_id": current.buyer_user_id,
                    "action": if cancel { "cancel_refund" } else { "keep_waiting" },
                })));
                outbox.record(&transaction, tenant, Actor::system(), subject(id), &event).await?;
                transaction.commit().await?;
                Ok(changed)
            })
            .await?;
        if matches!(applied, Some(Applied::Changed)) {
            self.refund_cancelled(tenant, id).await?;
        }
        Ok(true)
    }

    async fn remind_pickup(
        &self,
        tenant: &TenantContext,
        row: &order::Model,
        stored: pickup::Model,
    ) -> Result<bool, Error> {
        let (database, outbox) = (tenant.database(), &self.outbox);
        let (row, stored) = (row, &stored);
        database
            .write_with_retry(move || async move {
                let transaction = database.begin_write().await?;
                let changed = pickup::Entity::update_many()
                    .col_expr(pickup::Column::RemindedAt, crate::service::now().into())
                    .filter(pickup::Column::OrderId.eq(row.id))
                    .filter(pickup::Column::RemindedAt.is_null())
                    .scoped(tenant)
                    .exec(&transaction)
                    .await?;
                if changed.rows_affected == 0 {
                    return Ok(false);
                }
                let event = PickupReminder(Change::after(json!({
                    "order_id": row.id, "tenant_id": row.tenant_id, "org_id": row.org_id,
                    "channel": row.channel, "number": row.number,
                    "buyer_user_id": row.buyer_user_id, "pickup_org_id": stored.pickup_org_id,
                    "hold_until": stored.hold_until,
                })));
                outbox
                    .record(&transaction, tenant, Actor::system(), subject(row.id), &event)
                    .await?;
                transaction.commit().await?;
                Ok(true)
            })
            .await
    }
}

fn subject(order_id: Uuid) -> Option<String> {
    Some(format!("orders.order:{order_id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ma_nhan_sau_so_va_so_dien_thoai_so_khop_bo_dinh_dang() {
        let code = new_code();
        assert_eq!(code.len(), 6);
        assert!(code.bytes().all(|b| b.is_ascii_digit()));
        assert_eq!(phone_digits("+84 901 234 567"), phone_digits("0901-234-567"));
        assert!(same_name("  Nguyễn  Văn A ", "nguyễn văn a"));
    }
}
