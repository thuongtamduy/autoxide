//! Chuyển trạng thái của chương trình: `draft` → (`activate`) `scheduled` / `active` ⇄ `paused`
//! → `ended`; mọi trạng thái chưa kết thúc → `cancelled`. Job `promotions.tick` đổi `scheduled`
//! → `active` khi tới giờ và → `ended` khi qua `ends_at`. Mỗi lần chuyển sau khi bật thêm một
//! dòng lưu vết.

use autoxide_kernel::access::Principal;
use autoxide_kernel::entitlement::Limit;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::json;
use uuid::Uuid;

use super::{PromotionView, not_found, record_version, store};
use crate::compliance::{Check, Severity, activation_error};
use crate::entity::promotion;
use crate::events::{self, Change};
use crate::model::Status;
use crate::permissions::PUBLISH;
use crate::service::{
    PromotionsService, bump_revision, lock_revision, now, readable, writable,
};

/// Hạn mức số chương trình đang bật của một khách (bước 35).
pub(crate) const ACTIVE_PROMOTIONS: Limit = Limit::count(
    "promotions.active_promotions",
    "Promotions turned on at the same time (scheduled, running or paused).",
);

fn changed_while_checking(id: Uuid) -> Error {
    Error::conflict(
        "promotions.changed_while_activating",
        "Promotion `{id}` changed while it was being checked for activation. Activate it again.",
    )
    .with_param("id", id)
}

pub(crate) fn invalid_transition(from: Status, to: Status) -> Error {
    Error::conflict(
        "promotions.invalid_transition",
        "A promotion cannot go from `{from}` to `{to}`.",
    )
    .with_param("from", from.as_str())
    .with_param("to", to.as_str())
}

/// Một lệnh chuyển trạng thái.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Transition {
    Pause,
    Resume,
    End,
    Cancel,
}

/// Kết quả của `activate`.
#[derive(Debug, Clone)]
pub(crate) struct Activated {
    pub view: PromotionView,
    /// Cảnh báo (không chặn), vd xung đột với chương trình khác.
    pub warnings: Vec<Check>,
}

fn entitlement_key(id: Uuid) -> String {
    format!("promotions.promotion:{id}")
}

impl PromotionsService {
    /// Kiểm trước khi bật, không ghi (D-225).
    pub(crate) async fn activation_check(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Vec<Check>, Error> {
        let read = tenant.database().read();
        let view = store::load(read, tenant, id).await?.ok_or_else(|| not_found(id))?;
        readable(principal, read, &PUBLISH, view.row.org_id, || not_found(id)).await?;
        self.activation_checks(tenant, &crate::compiled::program_of(&view)).await
    }

    /// Bật chương trình `draft`: kiểm tuân thủ (lỗi → `errors[]`), giữ hạn mức số chương trình
    /// đang bật, rồi `scheduled` (chưa tới giờ) hay `active`.
    pub(crate) async fn activate(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Activated, Error> {
        let read = tenant.database().read();
        let view = store::load(read, tenant, id).await?.ok_or_else(|| not_found(id))?;
        readable(principal, read, &PUBLISH, view.row.org_id, || not_found(id)).await?;
        if view.status() != Status::Draft {
            return Err(invalid_transition(view.status(), Status::Active));
        }
        // Gọi `catalog` và đọc dữ liệu luật NGOÀI transaction ghi.
        let checks = self.activation_checks(tenant, &crate::compiled::program_of(&view)).await?;
        let failed: Vec<&Check> =
            checks.iter().filter(|found| !found.ok && found.severity == Severity::Error).collect();
        if !failed.is_empty() {
            return Err(activation_error(&failed));
        }
        let warnings: Vec<Check> = checks.into_iter().filter(|found| !found.ok).collect();
        let limit = self.legal_limit_of(&crate::compiled::program_of(&view)).await?;
        let key = entitlement_key(id);
        self.entitlements.reserve(tenant.id(), &ACTIVE_PROMOTIONS, 1, &key).await?;
        let activated = self.write_activation(tenant, principal, &view, limit.as_ref()).await;
        match &activated {
            Ok(_) => self.entitlements.confirm(tenant.id(), &ACTIVE_PROMOTIONS, &key).await?,
            Err(_) => self.entitlements.release(tenant.id(), &ACTIVE_PROMOTIONS, &key).await?,
        }
        Ok(Activated { view: activated?, warnings })
    }

    /// Ghi lần bật của chương trình `checked` (bản đã qua bộ kiểm ngoài transaction). `limit`: hạn
    /// mức luật đã tìm ngoài transaction, để kiểm lại tổng số ngày giảm giá dưới khoá.
    async fn write_activation(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        checked: &PromotionView,
        limit: Option<&crate::legal::LegalLimit>,
    ) -> Result<PromotionView, Error> {
        let id = checked.row.id;
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        let view =
            store::load_locked(&transaction, tenant, id).await?.ok_or_else(|| not_found(id))?;
        writable(principal, &transaction, &PUBLISH, view.row.org_id, || not_found(id)).await?;
        if view.status() != Status::Draft {
            return Err(invalid_transition(view.status(), Status::Active));
        }
        // Bộ kiểm tuân thủ chạy ngoài transaction (gọi `catalog`). Chương trình đổi giữa lúc kiểm
        // và lúc ghi (sửa bản nháp, duyệt thêm sản phẩm của người bán) thì kết quả kiểm không còn
        // đúng: báo để gọi lại, không bật bản chưa kiểm vượt trần pháp lý.
        if view.snapshot() != checked.snapshot() {
            return Err(changed_while_checking(id));
        }
        // Tổng số ngày giảm giá trong năm cộng cả chương trình khác: hai lần bật cùng lúc từng cùng
        // thấy chương trình kia còn nháp và cùng qua, tổng vượt trần (cả hai DB — bước kiểm ngoài
        // transaction). Khoá chung của khách rồi kiểm lại trong transaction (D-181).
        lock_revision(&transaction, tenant).await?;
        if let Some(limit) = limit {
            let program = crate::compiled::program_of(&view);
            let duration = self.duration_check(&transaction, tenant, &program, limit).await?;
            if !duration.ok {
                return Err(activation_error(&[&duration]));
            }
        }
        let at = now();
        let next = if view.row.starts_at > at { Status::Scheduled } else { Status::Active };
        promotion::Entity::update_many()
            .col_expr(promotion::Column::Status, Expr::value(next.as_str()))
            .col_expr(promotion::Column::ActivatedAt, Expr::value(Some(at)))
            .col_expr(promotion::Column::UpdatedAt, Expr::value(at))
            .filter(promotion::Column::Id.eq(id))
            .filter(promotion::Column::Status.eq(Status::Draft.as_str()))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let after = store::load(&transaction, tenant, id).await?.ok_or_else(|| not_found(id))?;
        record_version(&transaction, tenant, &after, "activated", &actor).await?;
        bump_revision(&transaction, tenant).await?;
        let subject = events::promotion_subject(id);
        self.outbox
            .record(
                &transaction,
                tenant,
                actor,
                subject.clone(),
                &events::PromotionActivated(Change::after(after.snapshot())),
            )
            .await?;
        if next == Status::Active {
            if let Some(prices) = events::automatic_prices_changed(&after) {
                self.outbox.record(&transaction, tenant, actor, subject.clone(), &prices).await?;
            }
            self.outbox
                .record(
                    &transaction,
                    tenant,
                    actor,
                    subject,
                    &events::PromotionStarted(Change::after(json!({"id": id}))),
                )
                .await?;
        }
        transaction.commit().await?;
        self.compiled.invalidate(tenant.id());
        Ok(after)
    }

    /// Tạm dừng, chạy lại, kết thúc sớm, huỷ.
    pub(crate) async fn transition(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        transition: Transition,
    ) -> Result<PromotionView, Error> {
        let actor = Actor::from(principal);
        let transaction = tenant.database().begin_write().await?;
        let view =
            store::load_locked(&transaction, tenant, id).await?.ok_or_else(|| not_found(id))?;
        writable(principal, &transaction, &PUBLISH, view.row.org_id, || not_found(id)).await?;
        let from = view.status();
        let at = now();
        let to = match (transition, from) {
            (Transition::Pause, Status::Active | Status::Scheduled) => Status::Paused,
            (Transition::Resume, Status::Paused) if view.row.ends_at <= at => Status::Ended,
            (Transition::Resume, Status::Paused) if view.row.starts_at > at => Status::Scheduled,
            (Transition::Resume, Status::Paused) => Status::Active,
            (Transition::End, Status::Active | Status::Scheduled | Status::Paused) => Status::Ended,
            (
                Transition::Cancel,
                Status::Draft | Status::Active | Status::Scheduled | Status::Paused,
            ) => Status::Cancelled,
            (Transition::Pause, _) => return Err(invalid_transition(from, Status::Paused)),
            (Transition::Resume, _) => return Err(invalid_transition(from, Status::Active)),
            (Transition::End, _) => return Err(invalid_transition(from, Status::Ended)),
            (Transition::Cancel, _) => return Err(invalid_transition(from, Status::Cancelled)),
        };
        let mut update = promotion::Entity::update_many()
            .col_expr(promotion::Column::Status, Expr::value(to.as_str()))
            .col_expr(promotion::Column::UpdatedAt, Expr::value(at));
        // Kết thúc sớm: thời điểm kết thúc công bố thành bây giờ (đếm ngày giảm giá đúng).
        if to == Status::Ended && view.row.ends_at > at {
            update = update.col_expr(promotion::Column::EndsAt, Expr::value(at));
        }
        update
            .filter(promotion::Column::Id.eq(id))
            .filter(promotion::Column::Status.eq(from.as_str()))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let after = store::load(&transaction, tenant, id).await?.ok_or_else(|| not_found(id))?;
        if from != Status::Draft {
            record_version(&transaction, tenant, &after, to.as_str(), &actor).await?;
        }
        bump_revision(&transaction, tenant).await?;
        let change = Change::after(json!({"id": id, "from": from.as_str(), "to": to.as_str()}));
        let subject = events::promotion_subject(id);
        // Giá giảm tự động chỉ có khi chương trình `active`: vào hay rời trạng thái đó là giá đổi.
        if (from == Status::Active || to == Status::Active)
            && let Some(prices) = events::automatic_prices_changed(&after)
        {
            self.outbox.record(&transaction, tenant, actor, subject.clone(), &prices).await?;
        }
        match to {
            Status::Paused => {
                self.outbox
                    .record(&transaction, tenant, actor, subject, &events::PromotionPaused(change))
                    .await?;
            }
            Status::Ended => {
                self.outbox
                    .record(&transaction, tenant, actor, subject, &events::PromotionEnded(change))
                    .await?;
            }
            Status::Cancelled => {
                self.outbox
                    .record(
                        &transaction,
                        tenant,
                        actor,
                        subject,
                        &events::PromotionCancelled(change),
                    )
                    .await?;
            }
            _ => {
                self.outbox
                    .record(&transaction, tenant, actor, subject, &events::PromotionUpdated(change))
                    .await?;
            }
        }
        transaction.commit().await?;
        self.compiled.invalidate(tenant.id());
        if matches!(to, Status::Ended | Status::Cancelled) && from != Status::Draft {
            self.entitlements
                .release(tenant.id(), &ACTIVE_PROMOTIONS, &entitlement_key(id))
                .await?;
        }
        Ok(after)
    }

    /// Job `promotions.tick` (mỗi phút): `scheduled` → `active` khi tới giờ, chưa kết thúc →
    /// `ended` khi qua `ends_at`. Đổi có điều kiện: hai worker chạy cùng lúc không phát event hai
    /// lần.
    pub(crate) async fn tick(&self, tenant: &TenantContext) -> Result<u64, Error> {
        let at = now();
        let read = tenant.database().read();
        let due = promotion::Entity::find()
            .filter(
                sea_orm::Condition::any()
                    .add(
                        sea_orm::Condition::all()
                            .add(promotion::Column::Status.eq(Status::Scheduled.as_str()))
                            .add(promotion::Column::StartsAt.lte(at)),
                    )
                    .add(
                        sea_orm::Condition::all()
                            .add(promotion::Column::Status.is_in([
                                Status::Scheduled.as_str(),
                                Status::Active.as_str(),
                                Status::Paused.as_str(),
                            ]))
                            .add(promotion::Column::EndsAt.lte(at)),
                    ),
            )
            .scoped(tenant)
            .all(read)
            .await?;
        let mut changed = 0;
        for row in due {
            let from = Status::parse(&row.status).unwrap_or(Status::Draft);
            let to = if row.ends_at <= at { Status::Ended } else { Status::Active };
            let transaction = tenant.database().begin_write().await?;
            let done = promotion::Entity::update_many()
                .col_expr(promotion::Column::Status, Expr::value(to.as_str()))
                .col_expr(promotion::Column::UpdatedAt, Expr::value(at))
                .filter(promotion::Column::Id.eq(row.id))
                .filter(promotion::Column::Status.eq(from.as_str()))
                .scoped(tenant)
                .exec(&transaction)
                .await?
                .rows_affected;
            if done == 0 {
                continue;
            }
            bump_revision(&transaction, tenant).await?;
            let change =
                Change::after(json!({"id": row.id, "from": from.as_str(), "to": to.as_str()}));
            let subject = events::promotion_subject(row.id);
            // Bắt đầu hay hết hạn khi đang chạy: giá giảm tự động đổi (catalog tính lại giá).
            if from == Status::Active || to == Status::Active {
                let view = super::store::load(&transaction, tenant, row.id).await?;
                if let Some(prices) = view.as_ref().and_then(events::automatic_prices_changed) {
                    self.outbox
                        .record(&transaction, tenant, Actor::system(), subject.clone(), &prices)
                        .await?;
                }
            }
            if to == Status::Active {
                self.outbox
                    .record(
                        &transaction,
                        tenant,
                        Actor::system(),
                        subject,
                        &events::PromotionStarted(change),
                    )
                    .await?;
            } else {
                self.outbox
                    .record(
                        &transaction,
                        tenant,
                        Actor::system(),
                        subject,
                        &events::PromotionEnded(change),
                    )
                    .await?;
            }
            transaction.commit().await?;
            if to == Status::Ended {
                self.entitlements
                    .release(tenant.id(), &ACTIVE_PROMOTIONS, &entitlement_key(row.id))
                    .await?;
            }
            changed += 1;
        }
        if changed > 0 {
            self.compiled.invalidate(tenant.id());
        }
        Ok(changed)
    }
}
