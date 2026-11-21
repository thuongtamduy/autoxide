//! Ẩn tạm / hiện lại mục vì vụ việc (bước 37, D-363): nghe event `compliance.target_restricted` /
//! `compliance.target_restored` (hợp đồng `autoxide_kernel::moderation`) cho đích `content.entry:<id>`.
//! Không đọc bảng của `compliance`: chỉ dùng payload của event.
//!
//! Idempotent: ẩn mục đã ẩn, hiện mục không ẩn đều không đổi gì (event có thể tới lại). Ẩn ghi
//! `restricted_at` ở bản nháp và bản công khai: trang công khai trả 451 `content.restricted`, sitemap,
//! feed, danh sách và tìm kiếm bỏ qua; đăng lại vẫn giữ ẩn.

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Delivered, EventHandler};
use autoxide_kernel::moderation::{RestrictionChange, restriction};
use autoxide_kernel::tenancy::ScopeExt;
use autoxide_kernel::time::Timestamp;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::entity::{entry, published};
use crate::service::now;

/// Lỗi của trang công khai đang bị ẩn.
pub(crate) fn restricted() -> Error {
    Error::new(
        autoxide_kernel::error::ErrorKind::UnavailableForLegalReasons,
        "content.restricted",
        "This content is hidden for now while a report of an infringement is handled.",
    )
}

pub(crate) struct Listener;

#[async_trait::async_trait]
impl EventHandler for Listener {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some((change, notice)) = restriction(event) else {
            return Ok(());
        };
        let Some(id) = notice.id_in(crate::resource::ENTRY).and_then(|id| id.parse::<Uuid>().ok())
        else {
            return Ok(());
        };
        let value: Option<Timestamp> = match change {
            RestrictionChange::Restricted => Some(now()),
            RestrictionChange::Restored => None,
            _ => return Ok(()),
        };
        let tenant = &event.tenant;
        let transaction = tenant.database().begin_write().await?;
        let mut entries = entry::Entity::update_many()
            .col_expr(entry::Column::RestrictedAt, Expr::value(value))
            .filter(entry::Column::Id.eq(id))
            .scoped(tenant);
        let mut live = published::Entity::update_many()
            .col_expr(published::Column::RestrictedAt, Expr::value(value))
            .filter(published::Column::EntryId.eq(id))
            .scoped(tenant);
        // Chỉ đổi dòng đang ở trạng thái ngược lại: nhận lại event không dời mốc đã ghi.
        if value.is_some() {
            entries = entries.filter(entry::Column::RestrictedAt.is_null());
            live = live.filter(published::Column::RestrictedAt.is_null());
        } else {
            entries = entries.filter(entry::Column::RestrictedAt.is_not_null());
            live = live.filter(published::Column::RestrictedAt.is_not_null());
        }
        let changed = entries.exec(&transaction).await?.rows_affected;
        live.exec(&transaction).await?;
        transaction.commit().await?;
        if changed > 0 {
            tracing::info!(entry_id = %id, case_id = %notice.case_id, restricted = value.is_some(), "entry restriction changed");
        }
        Ok(())
    }
}
