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
use autoxide_kernel::db::{Expr, Queries as _};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::{ClientInfo, Schema};
use autoxide_kernel::pagination::{Page, PageRequest};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::throttle::Throttle;
use serde::{Deserialize, Serialize};
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Schema)]
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
        let transaction = tenant.db().begin_write().await?;
        let inserted = transaction
            .insert(report::Model {
                id: Uuid::now_v7(),
                tenant_id: autoxide_kernel::db::TenantKey::new(tenant.id()),
                comment_id: id,
                reporter_key,
                reporter_user_id: principal.map(Principal::user_id),
                reason: reason.as_str().to_owned(),
                note: note.map(str::to_owned),
                created_at: now,
            })
            .on_conflict_do_nothing([
                report::Model::TENANT_ID.key(),
                report::Model::COMMENT_ID.key(),
                report::Model::REPORTER_KEY.key(),
            ])
            .exec()
            .await?;
        if inserted == 0 {
            transaction.commit().await?;
            return Ok(());
        }
        transaction
            .update::<comment::Model>()
            .set_expr(
                comment::Model::REPORT_COUNT,
                comment::Model::REPORT_COUNT.expr() + Expr::value(1),
            )
            .filter(comment::Model::ID.eq(id))
            .exec()
            .await?;
        let threshold = i32::try_from(self.config.report_threshold).map_err(Error::internal)?;
        let held = transaction
            .update::<comment::Model>()
            .set(comment::Model::STATUS, Status::Pending.as_str().to_owned())
            .set(comment::Model::HOLD_REASON, Some("reported".to_owned()))
            .set(comment::Model::REPORTS_HELD_AT, Some(now))
            .set(comment::Model::UPDATED_AT, now)
            .filter(comment::Model::ID.eq(id))
            .filter(comment::Model::STATUS.eq(Status::Published.as_str().to_owned()))
            .filter(comment::Model::REPORTS_HELD_AT.is_null())
            .filter(comment::Model::REPORT_COUNT.gte(threshold))
            .exec()
            .await?;
        if held == 1 {
            let after = transaction
                .find::<comment::Model>()
                .filter(comment::Model::ID.eq(id))
                .one()
                .await?
                .ok_or_else(|| comment_not_found(id))?;
            shift_counts(&transaction, &after, Some(Status::Published), Some(Status::Pending))
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
        tenant
            .db()
            .read()
            .find::<report::Model>()
            .filter(report::Model::COMMENT_ID.eq(id))
            .page(report::Model::ID, page)
            .await
    }
}

/// Trang đầu, cũ trước (để test đọc đủ).
#[cfg(test)]
pub(crate) fn all_oldest() -> PageRequest {
    PageRequest::first(200, autoxide_kernel::pagination::Order::Asc)
}
