//! Báo cáo vi phạm (công khai). Mỗi người báo một bình luận một lần: người dùng theo `user_id`,
//! khách theo dấu vết IP — khối /24 (IPv4) hay /48 (IPv6) — (unique `(comment_id, reporter_key)`,
//! chèn `ON CONFLICT DO NOTHING`). Dấu vết trong `reporter_key` bị bỏ sau
//! `AUTOXIDE_COMMENTS_IP_RETENTION_DAYS` (`retention.rs`).
//!
//! Đủ `AUTOXIDE_COMMENTS_REPORT_THRESHOLD` báo cáo thì bình luận đang hiện về chờ duyệt (lý do
//! `reported`) — ĐÚNG MỘT LẦN, kể cả khi nhiều báo cáo tới cùng lúc: bộ đếm tăng bằng
//! `UPDATE … SET report_count = report_count + 1`, rồi một `UPDATE` có điều kiện (`status =
//! 'published' AND reports_held_at IS NULL AND report_count >= ngưỡng`) — chỉ một transaction
//! thắng, trên cả hai database. Người kiểm duyệt duyệt lại thì báo cáo sau không ẩn nó nữa. Bình
//! luận bị giữ vì báo cáo đã từng hiện nên job dọn không bao giờ tự xoá nó (`retention.rs`).

use std::net::{IpAddr, Ipv6Addr};
use std::time::Duration;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::ClientInfo;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::throttle::Throttle;
use sea_orm::sea_query::{Expr, ExprTrait as _, OnConflict};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::{comment, report};
use crate::events::{self, Change};
use crate::permissions::COMMENTS_READ;
use crate::retention::GUEST_REPORTER;
use crate::service::{
    CommentsService, MODULE, Status, comment_not_found, now, shift_counts, summary,
};

const HOUR: Duration = Duration::from_secs(3600);

/// Why a comment is reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = CommentReportReason)]
pub(crate) enum ReportReason {
    /// Advertising or junk.
    Spam,
    /// Insults, harassment, hate.
    Abuse,
    /// Breaks the law (e.g. a takedown request).
    Illegal,
    /// Anything else; say what in `note`.
    Other,
}

impl ReportReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ReportReason::Spam => "spam",
            ReportReason::Abuse => "abuse",
            ReportReason::Illegal => "illegal",
            ReportReason::Other => "other",
        }
    }
}

/// Khối địa chỉ gom khách khi đếm báo cáo: IPv6 theo /48 (một nhà mạng thường cấp cho mỗi khách
/// cả một /48 hay /56, nên một người đổi /64 trong khối của mình là có "người báo" mới); IPv4 giữ
/// nguyên (`ip_hash` gom theo /24). Đưa địa chỉ đầu khối /48 vào `ip_hash::fingerprint` (gom /64)
/// thì mọi địa chỉ trong cùng /48 ra cùng một bản băm.
fn reporter_block(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) if v6.to_ipv4_mapped().is_none() => {
            let s = v6.segments();
            IpAddr::V6(Ipv6Addr::new(s[0], s[1], s[2], 0, 0, 0, 0, 0))
        }
        other => other,
    }
}

impl CommentsService {
    /// Báo cáo bình luận `id` (đang hiện). Báo lại, hay vượt giới hạn số lần, trả lời như thường
    /// mà không lưu gì.
    pub(crate) async fn report(
        &self,
        tenant: &TenantContext,
        principal: Option<&Principal>,
        client: &ClientInfo,
        id: Uuid,
        reason: ReportReason,
        note: Option<&str>,
    ) -> Result<(), Error> {
        let note = note.map(str::trim).filter(|note| !note.is_empty());
        if note.is_some_and(|note| note.chars().count() > 500 || note.chars().any(char::is_control))
        {
            return Err(Error::validation(vec![FieldError::new(
                "note",
                "length",
                "must be at most 500 characters, on one line",
            )]));
        }
        let row = self.find(tenant, id).await?;
        if Status::of(&row) != Status::Published
            || self.target(tenant, &row.target_type, &row.target_id).await?.hidden
        {
            return Err(comment_not_found(id));
        }
        let reporter_key = match (principal, client.ip) {
            (Some(principal), _) => format!("user:{}", principal.user_id()),
            (None, Some(ip)) => format!(
                "{GUEST_REPORTER}{}",
                autoxide_kernel::ip_hash::fingerprint(&self.secret, MODULE, reporter_block(ip))
                    .await?
            ),
            (None, None) => format!("{GUEST_REPORTER}unknown"),
        };
        let throttle = Throttle::new(MODULE, self.config.reports_per_hour, HOUR)?;
        if !throttle.hit(tenant, &format!("report:{reporter_key}")).await?.is_allowed() {
            return Ok(());
        }

        let now = now();
        let transaction = tenant.database().begin_write().await?;
        let inserted = report::Entity::insert(report::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            comment_id: Set(id),
            reporter_key: Set(reporter_key),
            reporter_user_id: Set(principal.map(Principal::user_id)),
            reason: Set(reason.as_str().to_owned()),
            note: Set(note.map(str::to_owned)),
            created_at: Set(now),
        })
        .on_conflict(
            OnConflict::columns([
                report::Column::TenantId,
                report::Column::CommentId,
                report::Column::ReporterKey,
            ])
            .do_nothing()
            .to_owned(),
        )
        .exec_without_returning(&transaction)
        .await?;
        if inserted == 0 {
            transaction.commit().await?;
            return Ok(());
        }
        comment::Entity::update_many()
            .scoped(tenant)
            .col_expr(comment::Column::ReportCount, Expr::col(comment::Column::ReportCount).add(1))
            .filter(comment::Column::Id.eq(id))
            .exec(&transaction)
            .await?;
        let threshold = i32::try_from(self.config.report_threshold).map_err(Error::internal)?;
        let held = comment::Entity::update_many()
            .scoped(tenant)
            .col_expr(comment::Column::Status, Expr::value(Status::Pending.as_str()))
            .col_expr(comment::Column::HoldReason, Expr::value(Some("reported".to_owned())))
            .col_expr(comment::Column::ReportsHeldAt, Expr::value(Some(now)))
            .col_expr(comment::Column::UpdatedAt, Expr::value(now))
            .filter(comment::Column::Id.eq(id))
            .filter(comment::Column::Status.eq(Status::Published.as_str()))
            .filter(comment::Column::ReportsHeldAt.is_null())
            .filter(comment::Column::ReportCount.gte(threshold))
            .exec(&transaction)
            .await?
            .rows_affected;
        if held == 1 {
            let after = comment::Entity::find_by_id(id)
                .scoped(tenant)
                .one(&transaction)
                .await?
                .ok_or_else(|| comment_not_found(id))?;
            shift_counts(
                &transaction,
                tenant,
                &after,
                Some(Status::Published),
                Some(Status::Pending),
            )
            .await?;
            let before =
                comment::Model { status: Status::Published.as_str().to_owned(), ..after.clone() };
            let change = Change { before: Some(summary(&before)), after: Some(summary(&after)) };
            let actor = principal.map_or_else(Actor::public, Actor::from);
            self.outbox
                .record(
                    &transaction,
                    tenant,
                    actor,
                    events::comment_subject(id),
                    &events::CommentStatusChanged(change),
                )
                .await?;
            tracing::info!(comment_id = %id, "comment held after reports");
        }
        transaction.commit().await?;
        Ok(())
    }

    /// Báo cáo của bình luận `id`, mới trước. Cần `comments.read`.
    pub(crate) async fn reports_of(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<report::Model>, Error> {
        principal.require(&COMMENTS_READ)?;
        self.find(tenant, id).await?;
        let select = report::Entity::find().scoped(tenant).filter(report::Column::CommentId.eq(id));
        fetch_page(select, report::Column::Id, |row| row.id, page, tenant.database().read()).await
    }
}

/// Trang đầu, cũ trước (để test đọc đủ).
#[cfg(test)]
pub(crate) fn all_oldest() -> PageRequest {
    PageRequest::first(200, autoxide_kernel::pagination::Order::Asc)
}
