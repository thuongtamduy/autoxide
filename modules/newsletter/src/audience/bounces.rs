//! Danh sách chặn và bounce.
//!
//! - Địa chỉ trong danh sách chặn không bao giờ nhận mail chiến dịch (kiểm ngay trước khi gửi).
//! - Hard bounce hay khiếu nại một lần là chặn; soft bounce chặn khi tới
//!   `AUTOXIDE_NEWSLETTER_SOFT_BOUNCE_LIMIT` lần.
//! - Nguồn: event `mail.delivery_failed` của module `mail` (máy chủ nhận từ chối hẳn — coi là
//!   hard bounce), nghe **bằng tên** (`autoxide_kernel::mail::DELIVERY_FAILED`), và endpoint chung
//!   `POST /api/v1/newsletter/bounces` cho dịch vụ gửi mail bên ngoài gọi.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Delivered, EventHandler};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::TenantContext;
use sea_orm::sea_query::{Expr, ExprTrait as _, OnConflict};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::campaigns::sending::DeliveryStatus;
use crate::entity::{delivery, subscriber, suppression};
use crate::events::{self, Change};
use crate::permissions::{NEWSLETTER_MANAGE, NEWSLETTER_READ};
use crate::service::{NewsletterService, now, suppression_not_found};
use crate::text;

/// Why an address is suppressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SuppressionReason {
    /// The receiving server refused the address for good.
    HardBounce,
    /// Temporary failures reached `AUTOXIDE_NEWSLETTER_SOFT_BOUNCE_LIMIT`.
    SoftBounce,
    /// The person marked a mail as spam.
    Complaint,
    /// Added by an admin.
    Manual,
}

impl SuppressionReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            SuppressionReason::HardBounce => "hard_bounce",
            SuppressionReason::SoftBounce => "soft_bounce",
            SuppressionReason::Complaint => "complaint",
            SuppressionReason::Manual => "manual",
        }
    }
}

/// Kind of a bounce report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum BounceKind {
    /// Refused for good (unknown mailbox…): suppressed at once.
    Hard,
    /// Refused for now (mailbox full…): suppressed after a number of them.
    Soft,
    /// Marked as spam by the person: suppressed at once.
    Complaint,
}

/// Bộ lọc danh sách chặn.
#[derive(Debug, Clone, Default)]
pub(crate) struct SuppressionFilter {
    /// Một phần địa chỉ email.
    pub email: Option<String>,
}

/// What a bounce report did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub(crate) struct BounceOutcome {
    /// The address is (now or already) suppressed.
    pub suppressed: bool,
}

impl NewsletterService {
    /// Một trang danh sách chặn, mới trước (mặc định). Cần `newsletter.read`.
    pub(crate) async fn list_suppressions(
        &self,
        principal: &Principal,
        filter: &SuppressionFilter,
        page: &PageRequest,
    ) -> Result<Page<suppression::Model>, Error> {
        principal.require(&NEWSLETTER_READ)?;
        let mut select = suppression::Entity::find();
        if let Some(email) =
            filter.email.as_deref().map(str::trim).filter(|email| !email.is_empty())
        {
            select =
                select.filter(autoxide_kernel::search::contains(suppression::Column::Email, email));
        }
        fetch_page(select, suppression::Column::Id, |row| row.id, page, self.database.read()).await
    }

    /// Chặn `email` bằng tay. Đã có thì trả dòng cũ. Cần `newsletter.manage`.
    pub(crate) async fn add_suppression(
        &self,
        principal: &Principal,
        email: &str,
        note: Option<&str>,
    ) -> Result<suppression::Model, Error> {
        principal.require(&NEWSLETTER_MANAGE)?;
        let mut problems = Vec::new();
        let email = autoxide_kernel::email::checked(email, "email", &mut problems);
        let note = text::line(note, "note", 255, &mut problems);
        let Some(email) = email.filter(|_| problems.is_empty()) else {
            return Err(Error::validation(problems));
        };
        let transaction = self.database.begin_write().await?;
        let row = self
            .suppress(&transaction, Actor::from(principal), &email, SuppressionReason::Manual, note)
            .await?;
        transaction.commit().await?;
        Ok(row)
    }

    /// Bỏ chặn dòng `id`; đếm soft bounce của địa chỉ về 0. Cần `newsletter.manage`.
    pub(crate) async fn remove_suppression(
        &self,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        principal.require(&NEWSLETTER_MANAGE)?;
        let transaction = self.database.begin_write().await?;
        let row = suppression::Entity::find_by_id(id)
            .one(&transaction)
            .await?
            .ok_or_else(|| suppression_not_found(id))?;
        suppression::Entity::delete_by_id(id).exec(&transaction).await?;
        subscriber::Entity::update_many()
            .col_expr(subscriber::Column::SoftBounces, Expr::value(0))
            .filter(subscriber::Column::Email.eq(row.email.as_str()))
            .exec(&transaction)
            .await?;
        let event = events::SuppressionRemoved(Change {
            before: Some(json!({"id": row.id, "reason": row.reason})),
            after: None,
        });
        self.outbox
            .record(
                &transaction,
                &TenantContext::legacy_default(&self.database),
                Actor::from(principal),
                events::suppression_subject(id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Báo một bounce hay khiếu nại cho `email` (dịch vụ gửi mail ngoài gọi). Cần
    /// `newsletter.manage`.
    pub(crate) async fn report_bounce(
        &self,
        principal: &Principal,
        email: &str,
        kind: BounceKind,
        note: Option<&str>,
    ) -> Result<BounceOutcome, Error> {
        principal.require(&NEWSLETTER_MANAGE)?;
        let mut problems = Vec::new();
        let email = autoxide_kernel::email::checked(email, "email", &mut problems);
        let note = text::line(note, "reason", 255, &mut problems);
        let Some(email) = email.filter(|_| problems.is_empty()) else {
            return Err(Error::validation(problems));
        };
        self.bounce(Actor::from(principal), &email, kind, note).await
    }

    /// Ghi một bounce: hard / khiếu nại thì chặn; soft thì đếm, tới ngưỡng thì chặn.
    pub(crate) async fn bounce(
        &self,
        actor: Actor,
        email: &str,
        kind: BounceKind,
        note: Option<String>,
    ) -> Result<BounceOutcome, Error> {
        let transaction = self.database.begin_write().await?;
        let reason = match kind {
            BounceKind::Hard => Some(SuppressionReason::HardBounce),
            BounceKind::Complaint => Some(SuppressionReason::Complaint),
            BounceKind::Soft => {
                subscriber::Entity::update_many()
                    .col_expr(
                        subscriber::Column::SoftBounces,
                        Expr::col(subscriber::Column::SoftBounces).add(1),
                    )
                    .filter(subscriber::Column::Email.eq(email))
                    .exec(&transaction)
                    .await?;
                let count = subscriber::Entity::find()
                    .filter(subscriber::Column::Email.eq(email))
                    .one(&transaction)
                    .await?
                    .map_or(0, |row| row.soft_bounces);
                let limit = i32::try_from(self.config.soft_bounce_limit).unwrap_or(i32::MAX);
                (count >= limit).then_some(SuppressionReason::SoftBounce)
            }
        };
        if let Some(reason) = reason {
            self.suppress(&transaction, actor, email, reason, note).await?;
        }
        let suppressed = suppression::Entity::find()
            .filter(suppression::Column::Email.eq(email))
            .one(&transaction)
            .await?
            .is_some();
        transaction.commit().await?;
        Ok(BounceOutcome { suppressed })
    }

    /// Thêm `email` vào danh sách chặn (đã có thì giữ dòng cũ), kèm event khi thêm mới.
    async fn suppress<C: ConnectionTrait>(
        &self,
        connection: &C,
        actor: Actor,
        email: &str,
        reason: SuppressionReason,
        note: Option<String>,
    ) -> Result<suppression::Model, Error> {
        let fresh = suppression::ActiveModel {
            id: Set(Uuid::now_v7()),
            email: Set(email.to_owned()),
            reason: Set(reason.as_str().to_owned()),
            note: Set(note),
            created_at: Set(now()),
        };
        let inserted = suppression::Entity::insert(fresh)
            .on_conflict(OnConflict::column(suppression::Column::Email).do_nothing().to_owned())
            .exec_without_returning(connection)
            .await?;
        let row = suppression::Entity::find()
            .filter(suppression::Column::Email.eq(email))
            .one(connection)
            .await?
            .ok_or_else(|| Error::internal("suppression row vanished"))?;
        if inserted > 0 {
            let event = events::SuppressionAdded(Change {
                before: None,
                after: Some(json!({"id": row.id, "reason": row.reason})),
            });
            self.outbox
                .record(
                    connection,
                    &TenantContext::legacy_default(&self.database),
                    actor,
                    events::suppression_subject(row.id),
                    &event,
                )
                .await?;
            tracing::info!(suppression_id = %row.id, reason = reason.as_str(), "address suppressed");
        }
        Ok(row)
    }
}

/// Bên nhận event `mail.delivery_failed`: mail của bản tin (hay gửi tới một người đăng ký) bị
/// từ chối hẳn → lượt gửi thành `failed`, địa chỉ bị chặn (hard bounce). Nhận lại cùng event
/// không đổi gì thêm.
pub(crate) struct DeliveryFailures(pub Arc<NewsletterService>);

#[async_trait::async_trait]
impl EventHandler for DeliveryFailures {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        if event.name != autoxide_kernel::mail::DELIVERY_FAILED {
            return Ok(());
        }
        let service = &self.0;
        let to = event.payload.get("to").and_then(|to| to.as_str()).unwrap_or_default();
        // `Ada <ada@example.com>` → `ada@example.com`.
        let address = match (to.rfind('<'), to.rfind('>')) {
            (Some(start), Some(end)) if start < end => &to[start + 1..end],
            _ => to,
        };
        let email = autoxide_kernel::email::normalize(address);
        let code = event.payload.get("code").and_then(|code| code.as_str()).map(str::to_owned);
        let delivery_id = event
            .payload
            .get("dedupe_key")
            .and_then(|key| key.as_str())
            .and_then(|key| key.strip_prefix("newsletter.delivery:"))
            .and_then(|id| id.parse::<Uuid>().ok());
        if let Some(id) = delivery_id {
            delivery::Entity::update_many()
                .col_expr(delivery::Column::Status, Expr::value(DeliveryStatus::Failed.as_str()))
                .col_expr(delivery::Column::Error, Expr::value(Some("hard_bounce")))
                .filter(delivery::Column::Id.eq(id))
                .exec(service.database.write())
                .await?;
        }
        let known = subscriber::Entity::find()
            .filter(subscriber::Column::Email.eq(email.as_str()))
            .one(service.database.read())
            .await?
            .is_some();
        if (delivery_id.is_some() || known) && autoxide_kernel::email::is_valid(&email) {
            let note = code.map(|code| format!("smtp {code}"));
            service.bounce(Actor::system(), &email, BounceKind::Hard, note).await?;
        }
        Ok(())
    }
}
