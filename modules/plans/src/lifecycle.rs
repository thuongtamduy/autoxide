//! Vòng đời thuê bao (bước 35 phần E, D-327): dùng thử → đang dùng → quá hạn (ân hạn) → tạm khoá (chỉ
//! đọc) → huỷ (chỉ đọc, giữ dữ liệu) → xoá. Job gọi với `now` của nó (test truyền đồng hồ giả qua
//! dữ liệu). Mọi chuyển trạng thái: khoá dòng thuê bao, xét lại trạng thái, rồi mới đổi — job và lần
//! thu tiền đến cùng lúc không đè nhau.
//!
//! Chưa có cổng thanh toán (bước 36): kỳ thanh toán `open` chỉ được nhà vận hành đánh dấu đã trả
//! (`mark-paid`) hay huỷ (`void`).

use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::event::{Actor, Event};
use autoxide_kernel::mail::MailTemplate;
use autoxide_kernel::money::Money;
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::{TenantChanges, TenantId};
use chrono::{DateTime, Duration, Utc};
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter,
    QueryOrder, QuerySelect, Set,
};
use serde_json::json;
use uuid::Uuid;

use crate::entity::{addon, billing_period, plan, price, subscription};
use crate::events;
use crate::service::{
    PlansService, invalid_transition, now, period_end, plan_not_active, plan_not_found,
    price_not_found, subscription_json, subscription_not_found,
};

/// Lô thuê bao mỗi lượt job.
const BATCH: u64 = 100;

pub(crate) const REMINDER: MailTemplate = MailTemplate::new(
    "plans.mail.reminder",
    "Your subscription: {amount} {currency} due on {due}",
    "The next period of your subscription ({plan}) costs {amount} {currency} and is due on {due}. \
     Pay it before then to keep full access.",
);

pub(crate) const OVERDUE: MailTemplate = MailTemplate::new(
    "plans.mail.overdue",
    "Your subscription payment is overdue",
    "The payment of {amount} {currency} for your subscription ({plan}) was due on {due}. Until it is \
     paid your access continues for a grace period; after that the data can only be read.",
);

/// Đổi gói do chính khách yêu cầu.
#[derive(Debug, Clone)]
pub(crate) struct ChangeInput {
    pub plan_id: Uuid,
    pub price_id: Option<Uuid>,
    pub addons: Option<Vec<(Uuid, i32)>>,
    pub billing_email: Option<String>,
}

/// Mã tham chiếu của kỳ thanh toán (D-184): `plans:<subscription_id>:<period_start>:<kind>`.
fn period_reference(subscription: Uuid, start: DateTime<Utc>, kind: &str) -> String {
    format!("plans:{subscription}:{}:{kind}", start.timestamp())
}

/// Giá theo tháng (để so nâng / hạ gói), bằng `i128` cho khỏi tràn.
fn monthly(price: Option<&price::Model>) -> i128 {
    price.map_or(0, |row| {
        let months =
            if row.interval == "year" { 12 } else { 1 } * i128::from(row.interval_count.max(1));
        i128::from(row.amount_minor) / months
    })
}

/// Phần chênh lệch tỉ lệ khi nâng gói giữa kỳ: `(mới − cũ) × giây còn lại / giây của kỳ`, làm tròn
/// xuống đơn vị nhỏ nhất. Nhân trước chia sau bằng `i128`; không âm.
pub(crate) fn proration(
    old: i64,
    new: i64,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    at: DateTime<Utc>,
) -> i64 {
    let total = i128::from((end - start).num_seconds().max(1));
    let left = i128::from((end - at).num_seconds().clamp(0, (end - start).num_seconds().max(1)));
    let difference = (i128::from(new) - i128::from(old)).max(0);
    i64::try_from(difference * left / total).unwrap_or(i64::MAX)
}

impl PlansService {
    // ---- Job ----------------------------------------------------------------------------------

    /// Chuyển các thuê bao tới hạn tại `at` theo bảng vòng đời; trả số thuê bao đã đổi.
    pub(crate) async fn run_lifecycle(&self, at: DateTime<Utc>) -> Result<usize, Error> {
        let read = self.databases.control().read();
        let mut tenants: Vec<Uuid> = subscription::Entity::find()
            .select_only()
            .column(subscription::Column::TenantId)
            .filter(
                Condition::any()
                    .add(
                        Condition::all()
                            .add(subscription::Column::Status.is_in(["trialing", "active"]))
                            .add(subscription::Column::CurrentPeriodEnd.lte(at)),
                    )
                    .add(
                        Condition::all()
                            .add(subscription::Column::Status.eq("past_due"))
                            .add(subscription::Column::GraceEndsAt.lte(at)),
                    )
                    .add(subscription::Column::Status.eq("suspended"))
                    .add(
                        Condition::all()
                            .add(subscription::Column::Status.eq("canceled"))
                            .add(subscription::Column::DeleteAfter.lte(at)),
                    ),
            )
            .order_by_asc(subscription::Column::CurrentPeriodEnd)
            .limit(BATCH)
            .into_tuple()
            .all(read)
            .await?;
        // Kỳ đầu / tỉ lệ chưa trả tới hạn: thuê bao đang dùng thành quá hạn.
        let overdue: Vec<Uuid> = billing_period::Entity::find()
            .select_only()
            .column(billing_period::Column::TenantId)
            .filter(billing_period::Column::Status.eq("open"))
            .filter(billing_period::Column::Kind.is_in(["initial", "proration"]))
            .filter(billing_period::Column::DueAt.lte(at))
            .limit(BATCH)
            .into_tuple()
            .all(read)
            .await?;
        tenants.extend(overdue);
        tenants.sort_unstable();
        tenants.dedup();
        let mut changed = 0;
        for tenant in tenants {
            if self.step(TenantId::from_uuid(tenant), at).await? {
                changed += 1;
            }
        }
        Ok(changed)
    }

    /// Một bước của thuê bao của khách `tenant` tại `at`, nếu tới hạn.
    async fn step(&self, tenant: TenantId, at: DateTime<Utc>) -> Result<bool, Error> {
        let transaction = self.databases.control().begin_write().await?;
        let Some(sub) = rows::lock_one(
            &transaction,
            subscription::Entity::find()
                .filter(subscription::Column::TenantId.eq(tenant.as_uuid())),
        )
        .await?
        else {
            return Ok(false);
        };
        let plan = plan::Entity::find_by_id(sub.plan_id)
            .one(&transaction)
            .await?
            .ok_or_else(|| plan_not_found(sub.plan_id))?;
        let days = |value: i32| Duration::days(i64::from(value));
        let mut deleted = false;
        let changed = match sub.status.as_str() {
            "trialing" | "active" if sub.current_period_end <= at => {
                if sub.cancel_at_period_end {
                    let delete_after = at + days(plan.retention_days);
                    self.move_to(&transaction, sub, "canceled", at, |row| {
                        row.canceled_at = Set(Some(at));
                        row.delete_after = Set(Some(delete_after));
                    })
                    .await?;
                } else {
                    self.renew(&transaction, sub, &plan, at).await?;
                }
                true
            }
            "trialing" | "active" if self.overdue(&transaction, sub.id, at).await? => {
                let grace = at + days(plan.grace_days);
                self.move_to(&transaction, sub, "past_due", at, |row| {
                    row.grace_ends_at = Set(Some(grace))
                })
                .await?;
                true
            }
            "past_due" if sub.grace_ends_at.is_some_and(|grace| grace <= at) => {
                self.move_to(&transaction, sub, "suspended", at, |row| {
                    row.suspended_at = Set(Some(at))
                })
                .await?;
                true
            }
            "suspended"
                if sub.suspended_at.is_some_and(|since| since + days(plan.suspend_days) <= at) =>
            {
                let delete_after = at + days(plan.retention_days);
                self.move_to(&transaction, sub, "canceled", at, |row| {
                    row.canceled_at = Set(Some(at));
                    row.delete_after = Set(Some(delete_after));
                })
                .await?;
                true
            }
            // Hết thời hạn lưu: xoá khách TRƯỚC (ngoài transaction này — database khác, file), rồi
            // mới thành `deleted`; xoá hỏng thì thuê bao vẫn `canceled` tới hạn, lượt sau thử lại.
            "canceled" if sub.delete_after.is_some_and(|after| after <= at) => {
                deleted = true;
                false
            }
            _ => false,
        };
        transaction.commit().await?;
        if deleted {
            // Khoá hẳn khách (mọi máy chủ ngừng phục vụ) rồi xoá dữ liệu qua kernel (D-337) — bảng
            // kế toán module khai giữ lại thì còn.
            self.erase_tenant(tenant).await?;
            let transaction = self.databases.control().begin_write().await?;
            if let Some(sub) = rows::lock_one(
                &transaction,
                subscription::Entity::find()
                    .filter(subscription::Column::TenantId.eq(tenant.as_uuid())),
            )
            .await?
                && sub.status == "canceled"
            {
                self.move_to(&transaction, sub, "deleted", at, |_| {}).await?;
            }
            transaction.commit().await?;
            self.cache.forget(tenant);
            return Ok(true);
        }
        if changed {
            self.cache.forget(tenant);
        }
        // Vừa thành `canceled`: tự xin một lượt xuất dữ liệu cho khách tải về (A-b9).
        let status = subscription::Entity::find()
            .filter(subscription::Column::TenantId.eq(tenant.as_uuid()))
            .one(self.databases.control().write())
            .await?
            .map(|row| row.status);
        if changed && status.as_deref() == Some("canceled") {
            self.offer_export(tenant).await;
        }
        Ok(changed)
    }

    /// Có kỳ đầu / kỳ tỉ lệ chưa trả đã tới hạn không.
    async fn overdue<C: ConnectionTrait>(
        &self,
        connection: &C,
        subscription: Uuid,
        at: DateTime<Utc>,
    ) -> Result<bool, Error> {
        let found = billing_period::Entity::find()
            .filter(billing_period::Column::SubscriptionId.eq(subscription))
            .filter(billing_period::Column::Status.eq("open"))
            .filter(billing_period::Column::Kind.is_in(["initial", "proration"]))
            .filter(billing_period::Column::DueAt.lte(at))
            .one(connection)
            .await?;
        Ok(found.is_some())
    }

    /// Hết kỳ (dùng thử hay kỳ thường): áp gói hạ đã hẹn; giá 0 → kỳ mới; giá > 0 → kỳ mới nếu kỳ
    /// kế đã trả, không thì quá hạn (ân hạn) và bảo đảm có kỳ thanh toán `renewal` để trả.
    async fn renew<C: ConnectionTrait>(
        &self,
        connection: &C,
        sub: subscription::Model,
        plan: &plan::Model,
        at: DateTime<Utc>,
    ) -> Result<subscription::Model, Error> {
        let plan_id = sub.scheduled_plan_id.unwrap_or(sub.plan_id);
        let price_id =
            if sub.scheduled_plan_id.is_some() { sub.scheduled_price_id } else { sub.price_id };
        let chosen = match price_id {
            Some(id) => price::Entity::find_by_id(id).one(connection).await?,
            None => None,
        };
        let start = sub.current_period_end;
        let end = period_end(start, chosen.as_ref())?;
        let charged = chosen.as_ref().is_some_and(|row| row.amount_minor > 0);
        let paid = if charged {
            let reference = period_reference(sub.id, start, "renewal");
            let found = billing_period::Entity::find()
                .filter(billing_period::Column::TenantId.eq(sub.tenant_id))
                .filter(billing_period::Column::Reference.eq(reference))
                .one(connection)
                .await?;
            if found.is_none() {
                let window = (start, end, start);
                self.open_period(connection, &sub, "renewal", plan_id, chosen.as_ref(), window)
                    .await?;
            }
            found.is_some_and(|row| row.status == "paid")
        } else {
            true
        };
        let grace = at + Duration::days(i64::from(plan.grace_days));
        let to = if paid { "active" } else { "past_due" };
        self.move_to(connection, sub, to, at, |row| {
            row.plan_id = Set(plan_id);
            row.price_id = Set(price_id);
            row.scheduled_plan_id = Set(None);
            row.scheduled_price_id = Set(None);
            if paid {
                row.current_period_start = Set(start);
                row.current_period_end = Set(end);
                row.trial_ends_at = Set(None);
            } else {
                row.grace_ends_at = Set(Some(grace));
            }
        })
        .await
    }

    /// Mở kỳ thanh toán theo giá `chosen` (không có giá: không mở).
    async fn open_period<C: ConnectionTrait>(
        &self,
        connection: &C,
        sub: &subscription::Model,
        kind: &'static str,
        plan_id: Uuid,
        chosen: Option<&price::Model>,
        (start, end, due): (DateTime<Utc>, DateTime<Utc>, DateTime<Utc>),
    ) -> Result<(), Error> {
        let Some(chosen) = chosen else {
            return Ok(());
        };
        let period = NewPeriod {
            kind,
            plan_id,
            price_id: Some(chosen.id),
            amount: chosen.amount_minor,
            currency: &chosen.amount_currency,
            start,
            end,
            due,
        };
        self.insert_period(connection, sub, period).await
    }

    /// Thêm kỳ thanh toán (không trùng: unique `(tenant_id, reference)`).
    async fn insert_period<C: ConnectionTrait>(
        &self,
        connection: &C,
        sub: &subscription::Model,
        period: NewPeriod<'_>,
    ) -> Result<(), Error> {
        let NewPeriod { kind, plan_id, price_id, amount, currency, start, end, due } = period;
        let at = now();
        let reference = period_reference(sub.id, start, kind);
        let period_id = Uuid::now_v7();
        let row = billing_period::ActiveModel {
            id: Set(period_id),
            tenant_id: Set(sub.tenant_id),
            subscription_id: Set(sub.id),
            kind: Set(kind.to_owned()),
            plan_id: Set(plan_id),
            price_id: Set(price_id),
            period_start: Set(start),
            period_end: Set(end),
            amount_minor: Set(amount),
            amount_currency: Set(currency.to_owned()),
            reference: Set(reference.clone()),
            invoice_id: Set(None),
            invoice_number: Set(None),
            status: Set("open".to_owned()),
            due_at: Set(due),
            paid_at: Set(None),
            provider_reference: Set(None),
            reminders: Set("[]".to_owned()),
            created_at: Set(at),
            updated_at: Set(at),
        };
        let inserted = billing_period::Entity::insert(row)
            .on_conflict(
                OnConflict::columns([
                    billing_period::Column::TenantId,
                    billing_period::Column::Reference,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(connection)
            .await?;
        if inserted > 0 {
            if amount > 0 {
                self.enqueue_invoice(connection, period_id).await?;
            }
            let event = events::BillingPeriodOpened(json!({
                "tenant_id": sub.tenant_id, "kind": kind, "reference": reference,
                "amount": amount.to_string(), "currency": currency, "due_at": due,
            }));
            self.record_event(connection, Actor::system(), sub.tenant_id, &event).await?;
        }
        Ok(())
    }

    /// Đổi trạng thái của `sub` sang `to` (kèm `edit`), ghi event trạng thái và quyền dùng.
    async fn move_to<C, F>(
        &self,
        connection: &C,
        sub: subscription::Model,
        to: &str,
        at: DateTime<Utc>,
        edit: F,
    ) -> Result<subscription::Model, Error>
    where
        C: ConnectionTrait,
        F: FnOnce(&mut subscription::ActiveModel),
    {
        let from = sub.status.clone();
        let tenant = sub.tenant_id;
        let mut row: subscription::ActiveModel = sub.into();
        row.status = Set(to.to_owned());
        edit(&mut row);
        row.updated_at = Set(at);
        let row = row.update(connection).await?;
        if from != to {
            let event = events::SubscriptionStatusChanged(
                json!({ "tenant_id": tenant, "from": from, "to": to }),
            );
            self.record_event(connection, Actor::system(), tenant, &event).await?;
            let event = events::EntitlementsChanged(json!({ "tenant_id": tenant }));
            self.record_event(connection, Actor::system(), tenant, &event).await?;
            tracing::info!(tenant_id = %tenant, from = %from, to, "subscription status changed");
        }
        Ok(row)
    }

    /// Thuê bao của khách `tenant` vừa bị huỷ: tự xin một lượt xuất dữ liệu để khách tải về (lỗi chỉ
    /// ghi log — huỷ thuê bao không phụ thuộc việc xuất).
    async fn offer_export(&self, tenant: TenantId) {
        if let Some(exports) = &self.exports
            && let Err(error) =
                exports.request(Actor::system(), tenant, "subscription_canceled").await
        {
            tracing::warn!(tenant_id = %tenant, error = %error, "cannot start the data export of a canceled subscription");
        }
    }

    /// Ghi event về khách `tenant` (dưới khách nền tảng).
    pub(crate) async fn record_event<C: ConnectionTrait, E: Event>(
        &self,
        connection: &C,
        actor: Actor,
        tenant: Uuid,
        event: &E,
    ) -> Result<(), Error> {
        let platform = self.databases.default_tenant();
        let subject = Some(format!("plans.subscription:{tenant}"));
        self.outbox.record(connection, &platform, actor, subject, event).await
    }

    /// Lập kỳ thanh toán sắp tới cho thuê bao có giá > 0 trong `renewal_lead_days` trước khi hết
    /// kỳ (không trùng). Trả số kỳ đã lập.
    pub(crate) async fn run_renewals(&self, at: DateTime<Utc>) -> Result<usize, Error> {
        let read = self.databases.control().read();
        let due = subscription::Entity::find()
            .filter(subscription::Column::Status.is_in(["trialing", "active"]))
            .filter(subscription::Column::CancelAtPeriodEnd.eq(false))
            .filter(subscription::Column::CurrentPeriodEnd.gt(at))
            .order_by_asc(subscription::Column::CurrentPeriodEnd)
            .limit(BATCH * 10)
            .all(read)
            .await?;
        let mut opened = 0;
        for sub in due {
            let plan_id = sub.scheduled_plan_id.unwrap_or(sub.plan_id);
            let price_id =
                if sub.scheduled_plan_id.is_some() { sub.scheduled_price_id } else { sub.price_id };
            let Some(plan) = plan::Entity::find_by_id(plan_id).one(read).await? else {
                continue;
            };
            if sub.current_period_end - Duration::days(i64::from(plan.renewal_lead_days)) > at {
                continue;
            }
            let Some(chosen) = (match price_id {
                Some(id) => price::Entity::find_by_id(id).one(read).await?,
                None => None,
            })
            .filter(|row| row.amount_minor > 0) else {
                continue;
            };
            let start = sub.current_period_end;
            let end = period_end(start, Some(&chosen))?;
            let write = self.databases.control().write();
            let window = (start, end, start);
            self.open_period(write, &sub, "renewal", plan_id, Some(&chosen), window).await?;
            opened += 1;
        }
        Ok(opened)
    }

    /// Gửi mail nhắc cho kỳ `open`: trước hạn theo `reminder_days`, khi quá hạn, và mỗi 3 ngày
    /// trong ân hạn; ghi mốc đã gửi vào kỳ để không gửi trùng. Thuê bao chưa có email thanh toán
    /// thì bỏ qua. Trả số mail đã xếp hàng.
    pub(crate) async fn run_reminders(&self, at: DateTime<Utc>) -> Result<usize, Error> {
        let Some(mailer) = self.mailer.clone() else {
            return Ok(0);
        };
        let read = self.databases.control().read();
        let open = billing_period::Entity::find()
            .filter(billing_period::Column::Status.eq("open"))
            .filter(billing_period::Column::DueAt.lte(
                at + Duration::days(i64::from(
                    self.reminder_days.iter().copied().max().unwrap_or(0),
                )),
            ))
            .limit(BATCH * 10)
            .all(read)
            .await?;
        let mut sent = 0;
        for period in open {
            let Some(sub) =
                subscription::Entity::find_by_id(period.subscription_id).one(read).await?
            else {
                continue;
            };
            // Email riêng của thuê bao, không có thì email của hồ sơ xuất hoá đơn.
            let email = match sub.billing_email.clone() {
                Some(email) => Some(email),
                None => self
                    .billing_profile(TenantId::from_uuid(sub.tenant_id))
                    .await?
                    .map(|profile| profile.email),
            };
            let Some(email) = email else {
                continue;
            };
            let Some(key) = self.reminder_due(&period, at) else {
                continue;
            };
            let mut sent_keys: Vec<String> =
                serde_json::from_str(&period.reminders).unwrap_or_default();
            if sent_keys.contains(&key) {
                continue;
            }
            let plan_name = plan::Entity::find_by_id(period.plan_id)
                .one(read)
                .await?
                .map_or_else(String::new, |row| row.name);
            let amount = Money::from_columns(period.amount_minor, &period.amount_currency)
                .map_or_else(|_| period.amount_minor.to_string(), |money| money.to_string());
            let due = period.due_at.format("%Y-%m-%d").to_string();
            let params = [
                ("amount", amount.as_str()),
                ("currency", period.amount_currency.as_str()),
                ("due", due.as_str()),
                ("plan", plan_name.as_str()),
            ];
            let template = if key.starts_with("before:") { REMINDER } else { OVERDUE };
            let tenant = self.databases.tenant(TenantId::from_uuid(period.tenant_id)).await?;
            let mail = template.mail(
                &email,
                &self.translations,
                autoxide_kernel::i18n::Language::Vi,
                &params,
            );
            mailer.send(&tenant, mail).await?;
            sent_keys.push(key);
            let mut row: billing_period::ActiveModel = period.into();
            row.reminders = Set(serde_json::to_string(&sent_keys).map_err(Error::internal)?);
            row.updated_at = Set(now());
            row.update(self.databases.control().write()).await?;
            sent += 1;
        }
        Ok(sent)
    }

    /// Mốc nhắc của kỳ `period` tại `at`, nếu có: `before:<n>` (còn ≤ n ngày), `overdue` (vừa quá
    /// hạn), `grace:<k>` (mỗi 3 ngày quá hạn).
    fn reminder_due(&self, period: &billing_period::Model, at: DateTime<Utc>) -> Option<String> {
        if at < period.due_at {
            let left = (period.due_at - at).num_days();
            return self
                .reminder_days
                .iter()
                .copied()
                .filter(|days| i64::from(*days) >= left)
                .min()
                .map(|days| format!("before:{days}"));
        }
        let late = (at - period.due_at).num_days();
        if late < 3 { Some("overdue".to_owned()) } else { Some(format!("grace:{}", late / 3)) }
    }

    // ---- Thanh toán tay ------------------------------------------------------------------------

    /// Nhà vận hành ghi nhận đã thu kỳ `id` (chuyển khoản đối soát tay). Thuê bao đang quá hạn hay
    /// tạm khoá thì đang dùng lại, kỳ mới bắt đầu từ kỳ đã trả (hay từ bây giờ nếu kỳ đó đã qua).
    pub(crate) async fn mark_paid(
        &self,
        actor: Actor,
        id: Uuid,
        provider_reference: String,
        paid_at: Option<DateTime<Utc>>,
    ) -> Result<billing_period::Model, Error> {
        let at = now();
        let transaction = self.databases.control().begin_write().await?;
        let period = rows::lock_one(&transaction, billing_period::Entity::find_by_id(id))
            .await?
            .ok_or_else(|| billing_period_not_found(id))?;
        if period.status != "open" {
            return Err(billing_period_not_payable());
        }
        let tenant = period.tenant_id;
        let window = (period.period_start, period.period_end);
        let mut row: billing_period::ActiveModel = period.into();
        row.status = Set("paid".to_owned());
        row.paid_at = Set(Some(paid_at.unwrap_or(at)));
        row.provider_reference = Set(Some(provider_reference));
        row.updated_at = Set(at);
        let paid = row.update(&transaction).await?;
        let event = events::BillingPeriodPaid(
            json!({ "tenant_id": tenant, "reference": paid.reference, "amount": paid.amount_minor.to_string(), "currency": paid.amount_currency }),
        );
        self.record_event(&transaction, actor, tenant, &event).await?;
        if let Some(sub) = rows::lock_one(
            &transaction,
            subscription::Entity::find().filter(subscription::Column::TenantId.eq(tenant)),
        )
        .await?
            && ["past_due", "suspended"].contains(&sub.status.as_str())
        {
            let price = match sub.price_id {
                Some(price_id) => price::Entity::find_by_id(price_id).one(&transaction).await?,
                None => None,
            };
            let (start, end) =
                if window.1 > at { window } else { (at, period_end(at, price.as_ref())?) };
            // Khách khác còn kỳ chưa trả tới hạn thì vẫn quá hạn.
            let still_due = self.overdue(&transaction, sub.id, at).await?;
            if !still_due {
                self.move_to(&transaction, sub, "active", at, |row| {
                    row.current_period_start = Set(start);
                    row.current_period_end = Set(end);
                    row.grace_ends_at = Set(None);
                    row.suspended_at = Set(None);
                })
                .await?;
            }
        }
        transaction.commit().await?;
        self.cache.forget(TenantId::from_uuid(tenant));
        Ok(paid)
    }

    /// Nhà vận hành huỷ kỳ `id` chưa trả (lập nhầm, miễn phí cho khách).
    pub(crate) async fn void_period(
        &self,
        actor: Actor,
        id: Uuid,
    ) -> Result<billing_period::Model, Error> {
        let transaction = self.databases.control().begin_write().await?;
        let period = rows::lock_one(&transaction, billing_period::Entity::find_by_id(id))
            .await?
            .ok_or_else(|| billing_period_not_found(id))?;
        if period.status != "open" {
            return Err(billing_period_not_payable());
        }
        let tenant = period.tenant_id;
        let mut row: billing_period::ActiveModel = period.into();
        row.status = Set("void".to_owned());
        row.updated_at = Set(now());
        let voided = row.update(&transaction).await?;
        let event = events::BillingPeriodVoided(
            json!({ "tenant_id": tenant, "reference": voided.reference }),
        );
        self.record_event(&transaction, actor, tenant, &event).await?;
        transaction.commit().await?;
        // Sau commit: huỷ hoá đơn của kỳ (gọi `dyn Invoices`, có thể gọi nhà cung cấp).
        self.cancel_invoice(&voided).await?;
        Ok(voided)
    }

    pub(crate) async fn billing_periods(
        &self,
        tenant: TenantId,
    ) -> Result<Vec<billing_period::Model>, Error> {
        Ok(billing_period::Entity::find()
            .filter(billing_period::Column::TenantId.eq(tenant.as_uuid()))
            .order_by_desc(billing_period::Column::Id)
            .limit(200)
            .all(self.databases.control().read())
            .await?)
    }

    /// Kỳ `id` của khách `tenant` (khách khác: không có).
    pub(crate) async fn billing_period(
        &self,
        tenant: Option<TenantId>,
        id: Uuid,
    ) -> Result<billing_period::Model, Error> {
        let mut select = billing_period::Entity::find_by_id(id);
        if let Some(tenant) = tenant {
            select = select.filter(billing_period::Column::TenantId.eq(tenant.as_uuid()));
        }
        select
            .one(self.databases.control().read())
            .await?
            .ok_or_else(|| billing_period_not_found(id))
    }

    // ---- Khách tự đổi / huỷ / tiếp tục --------------------------------------------------------

    /// Khách đổi gói: nâng (giá theo tháng không thấp hơn) có hiệu lực ngay, kèm kỳ tỉ lệ cho phần
    /// còn lại của kỳ; hạ thì hẹn tới hết kỳ (không hoàn tiền).
    pub(crate) async fn change(
        &self,
        actor: Actor,
        tenant: TenantId,
        input: ChangeInput,
    ) -> Result<(), Error> {
        let at = now();
        let transaction = self.databases.control().begin_write().await?;
        let chosen = plan::Entity::find_by_id(input.plan_id)
            .one(&transaction)
            .await?
            .ok_or_else(|| plan_not_found(input.plan_id))?;
        if chosen.kind != "base" || chosen.status != "active" {
            return Err(plan_not_active(&chosen.code));
        }
        let new_price = match input.price_id {
            Some(id) => Some(
                price::Entity::find_by_id(id)
                    .one(&transaction)
                    .await?
                    .filter(|row| row.plan_id == chosen.id && row.active && row.kind == "recurring")
                    .ok_or_else(|| price_not_found(id))?,
            ),
            None => None,
        };
        let sub = rows::lock_one(
            &transaction,
            subscription::Entity::find()
                .filter(subscription::Column::TenantId.eq(tenant.as_uuid())),
        )
        .await?
        .ok_or_else(subscription_not_found)?;
        if !["trialing", "active"].contains(&sub.status.as_str()) {
            return Err(invalid_transition(&sub.status));
        }
        let old_price = match sub.price_id {
            Some(id) => price::Entity::find_by_id(id).one(&transaction).await?,
            None => None,
        };
        let before = subscription_json(&sub);
        let upgrade = monthly(new_price.as_ref()) >= monthly(old_price.as_ref());
        let email = input
            .billing_email
            .map(|email| email.trim().to_owned())
            .filter(|email| !email.is_empty());
        let mut row: subscription::ActiveModel = sub.clone().into();
        if upgrade {
            row.plan_id = Set(chosen.id);
            row.price_id = Set(new_price.as_ref().map(|row| row.id));
            row.scheduled_plan_id = Set(None);
            row.scheduled_price_id = Set(None);
        } else {
            row.scheduled_plan_id = Set(Some(chosen.id));
            row.scheduled_price_id = Set(new_price.as_ref().map(|row| row.id));
        }
        if let Some(email) = email {
            row.billing_email = Set(Some(email));
        }
        row.updated_at = Set(at);
        let after = row.update(&transaction).await?;
        // Nâng gói giữa kỳ đang tính tiền: kỳ tỉ lệ cho phần còn lại.
        if upgrade
            && sub.status == "active"
            && let Some(price) = &new_price
        {
            let old = old_price.as_ref().map_or(0, |row| row.amount_minor);
            let amount = proration(
                old,
                price.amount_minor,
                sub.current_period_start,
                sub.current_period_end,
                at,
            );
            if amount > 0 {
                let period = NewPeriod {
                    kind: "proration",
                    plan_id: chosen.id,
                    price_id: Some(price.id),
                    amount,
                    currency: &price.amount_currency,
                    start: at,
                    end: sub.current_period_end,
                    due: at,
                };
                self.insert_period(&transaction, &after, period).await?;
            }
        }
        if let Some(addons) = input.addons {
            addon::Entity::delete_many()
                .filter(addon::Column::TenantId.eq(tenant.as_uuid()))
                .exec(&transaction)
                .await?;
            for (plan_id, quantity) in addons {
                let extra = plan::Entity::find_by_id(plan_id)
                    .one(&transaction)
                    .await?
                    .ok_or_else(|| plan_not_found(plan_id))?;
                if extra.kind != "addon"
                    || extra.status != "active"
                    || !(1..=1000).contains(&quantity)
                {
                    return Err(Error::new(ErrorKind::Validation, "plans.addon_not_allowed", "`{code}` is not the right kind of plan here: a subscription has one base plan and any number of add-ons.").with_param("code", extra.code));
                }
                addon::ActiveModel {
                    tenant_id: Set(tenant.as_uuid()),
                    plan_id: Set(plan_id),
                    quantity: Set(quantity),
                    created_at: Set(at),
                }
                .insert(&transaction)
                .await?;
            }
        }
        let event = events::SubscriptionPlanChanged(
            json!({ "tenant_id": tenant, "before": before, "after": subscription_json(&after), "scheduled": !upgrade }),
        );
        self.record_event(&transaction, actor, tenant.as_uuid(), &event).await?;
        let event = events::EntitlementsChanged(json!({ "tenant_id": tenant }));
        self.record_event(&transaction, actor, tenant.as_uuid(), &event).await?;
        transaction.commit().await?;
        self.cache.forget(tenant);
        Ok(())
    }

    /// Huỷ thuê bao: `at_period_end` → hết kỳ mới huỷ (tiếp tục được tới lúc đó); không thì huỷ
    /// ngay (chỉ đọc, giữ dữ liệu `retention_days`).
    pub(crate) async fn cancel(
        &self,
        actor: Actor,
        tenant: TenantId,
        at_period_end: bool,
    ) -> Result<(), Error> {
        let at = now();
        let transaction = self.databases.control().begin_write().await?;
        let sub = rows::lock_one(
            &transaction,
            subscription::Entity::find()
                .filter(subscription::Column::TenantId.eq(tenant.as_uuid())),
        )
        .await?
        .ok_or_else(subscription_not_found)?;
        if ["canceled", "deleted"].contains(&sub.status.as_str()) {
            return Err(invalid_transition(&sub.status));
        }
        if at_period_end {
            if !["trialing", "active", "past_due"].contains(&sub.status.as_str()) {
                return Err(invalid_transition(&sub.status));
            }
            let mut row: subscription::ActiveModel = sub.into();
            row.cancel_at_period_end = Set(true);
            row.updated_at = Set(at);
            row.update(&transaction).await?;
        } else {
            let plan = plan::Entity::find_by_id(sub.plan_id)
                .one(&transaction)
                .await?
                .ok_or_else(|| plan_not_found(sub.plan_id))?;
            let delete_after = at + Duration::days(i64::from(plan.retention_days));
            self.move_to(&transaction, sub, "canceled", at, |row| {
                row.canceled_at = Set(Some(at));
                row.delete_after = Set(Some(delete_after));
            })
            .await?;
        }
        let _ = actor;
        transaction.commit().await?;
        self.cache.forget(tenant);
        if !at_period_end {
            self.offer_export(tenant).await;
        }
        Ok(())
    }

    /// Tiếp tục: bỏ hẹn huỷ cuối kỳ; thuê bao đã huỷ (chưa tới hạn xoá) thì đăng ký lại, kỳ mới bắt
    /// đầu từ bây giờ.
    pub(crate) async fn resume(&self, tenant: TenantId) -> Result<(), Error> {
        let at = now();
        let transaction = self.databases.control().begin_write().await?;
        let sub = rows::lock_one(
            &transaction,
            subscription::Entity::find()
                .filter(subscription::Column::TenantId.eq(tenant.as_uuid())),
        )
        .await?
        .ok_or_else(subscription_not_found)?;
        match sub.status.as_str() {
            "canceled" if sub.delete_after.is_none_or(|after| after > at) => {
                let price = match sub.price_id {
                    Some(id) => price::Entity::find_by_id(id).one(&transaction).await?,
                    None => None,
                };
                let end = period_end(at, price.as_ref())?;
                let after = self
                    .move_to(&transaction, sub, "active", at, |row| {
                        row.current_period_start = Set(at);
                        row.current_period_end = Set(end);
                        row.canceled_at = Set(None);
                        row.delete_after = Set(None);
                        row.cancel_at_period_end = Set(false);
                        row.suspended_at = Set(None);
                        row.grace_ends_at = Set(None);
                    })
                    .await?;
                // Đăng ký lại có giá: kỳ đầu phải trả.
                if let Some(price) = price.filter(|row| row.amount_minor > 0) {
                    let period = NewPeriod {
                        kind: "initial",
                        plan_id: after.plan_id,
                        price_id: Some(price.id),
                        amount: price.amount_minor,
                        currency: &price.amount_currency,
                        start: at,
                        end,
                        due: at,
                    };
                    self.insert_period(&transaction, &after, period).await?;
                }
            }
            "trialing" | "active" | "past_due" if sub.cancel_at_period_end => {
                let mut row: subscription::ActiveModel = sub.into();
                row.cancel_at_period_end = Set(false);
                row.updated_at = Set(at);
                row.update(&transaction).await?;
            }
            status => return Err(invalid_transition(status)),
        }
        transaction.commit().await?;
        self.cache.forget(tenant);
        Ok(())
    }

    /// Khoá rồi xoá hẳn khách `tenant` (thuê bao hết thời hạn lưu dữ liệu).
    async fn erase_tenant(&self, tenant: TenantId) -> Result<(), Error> {
        let Some(admin) = self.tenants.clone() else {
            return Err(Error::internal("the kernel did not provide the tenant service"));
        };
        let reason = "Subscription deleted after its retention period";
        let id = tenant.as_uuid().to_string();
        let current = match admin.get(&id).await {
            Ok(found) => found,
            // Khách đã không còn trong sổ: không còn gì để xoá.
            Err(error) if error.code() == "tenant_not_found" => return Ok(()),
            Err(error) => return Err(error),
        };
        match current.status.as_str() {
            // Lượt trước đã xoá xong nhưng chưa kịp ghi `deleted` cho thuê bao.
            "deleted" => return Ok(()),
            "suspended" => {}
            _ => self.suspend_tenant(Actor::system(), tenant, reason).await?,
        }
        let erased = admin.erase(Actor::system(), &id, reason).await?;
        tracing::warn!(tenant_id = %tenant, rows = erased.rows, files = erased.files, "subscription deleted after its retention; the tenant's data is deleted");
        Ok(())
    }

    /// Nhân sự nền tảng khoá hẳn khách `tenant` (mọi request bị từ chối, kể cả đọc; khác chỉ đọc
    /// của thuê bao tạm khoá) — qua sổ khách của kernel, có event và audit.
    pub(crate) async fn suspend_tenant(
        &self,
        actor: Actor,
        tenant: TenantId,
        reason: &str,
    ) -> Result<(), Error> {
        let Some(admin) = self.tenants.clone() else {
            return Err(Error::internal("the kernel did not provide the tenant service"));
        };
        let changes = TenantChanges {
            status: Some("suspended".to_owned()),
            reason: Some(reason.to_owned()),
            ..TenantChanges::default()
        };
        admin.update(actor, tenant.as_uuid(), changes, None).await?;
        Ok(())
    }
}

/// Một kỳ thanh toán sắp thêm.
struct NewPeriod<'a> {
    kind: &'static str,
    plan_id: Uuid,
    price_id: Option<Uuid>,
    amount: i64,
    currency: &'a str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    due: DateTime<Utc>,
}

pub(crate) fn billing_period_not_found(id: Uuid) -> Error {
    Error::not_found("plans.billing_period_not_found", "There is no billing period `{id}`.")
        .with_param("id", id)
}

pub(crate) fn billing_period_not_payable() -> Error {
    Error::conflict(
        "plans.billing_period_not_payable",
        "This billing period is already paid or void.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn nang_goi_lap_ky_tinh_ti_le_lam_tron_xuong() {
        let start = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 1, 31, 0, 0, 0).unwrap();
        // Nửa kỳ còn lại, chênh 100 001 → 50 000 (làm tròn xuống, không 50 000,5).
        let half = start + (end - start) / 2;
        assert_eq!(proration(100_000, 200_001, start, end, half), 50_000);
        // Hết kỳ: 0; trước kỳ: cả phần chênh; hạ giá: 0 (không hoàn tiền).
        assert_eq!(proration(1, 2, start, end, end), 0);
        assert_eq!(proration(0, 300, start, end, start - Duration::days(1)), 300);
        assert_eq!(proration(500, 100, start, end, half), 0);
        // Số lớn không tràn (i128).
        assert_eq!(proration(0, i64::MAX, start, end, start), i64::MAX);
    }
}
