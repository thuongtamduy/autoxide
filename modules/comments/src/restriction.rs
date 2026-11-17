//! Bình luận bị ẩn tạm vì vụ việc (bước 37, D-363). `compliance` không đọc bảng của `comments`:
//!
//! - Sổ đối tượng: `comments` đăng ký loại `comments.comment` (bình luận có thật không, đang hiện
//!   không, ai viết) để vụ việc kiểm đích và biết bên bị phản ánh.
//! - Event `compliance.target_restricted` / `compliance.target_restored` (hợp đồng
//!   `autoxide_kernel::moderation`) cho `comments.comment:<id>`: ẩn bình luận (trạng thái
//!   `rejected`, lý do giữ `compliance`), nhớ trạng thái cũ trong `restricted_status`; hiện lại trả
//!   đúng trạng thái cũ. Idempotent — event có thể tới lại.
//! - Người kiểm duyệt không đổi được trạng thái bình luận đang bị ẩn vì vụ việc
//!   (`comments.restricted`, 409): việc đó thuộc về người xử lý vụ việc.

use std::collections::HashMap;
use std::sync::Arc;

use autoxide_kernel::error::Error;
use autoxide_kernel::event::{Actor, Delivered, EventHandler};
use autoxide_kernel::moderation::{RestrictionChange, restriction};
use autoxide_kernel::resource::{Resource, ResourceResolver};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{EntityTrait, IntoActiveModel, Set};
use uuid::Uuid;

use crate::entity::comment;
use crate::events::{self, Change};
use crate::posting::authoring::update_if_unchanged;
use crate::service::{CommentsService, Status, now, shift_counts, summary};

/// Loại đối tượng của bình luận trong sổ đối tượng.
pub(crate) const COMMENT: &str = "comments.comment";

pub(crate) fn restricted() -> Error {
    Error::conflict(
        "comments.restricted",
        "This comment is hidden while a case of the compliance module is handled; the case decides \
         when it shows again.",
    )
}

/// Bộ tra `comments.comment` của sổ đối tượng.
pub(crate) struct CommentResolver;

#[async_trait::async_trait]
impl ResourceResolver for CommentResolver {
    async fn resolve(&self, tenant: &TenantContext, id: &str) -> Result<Option<Resource>, Error> {
        let Ok(id) = id.parse::<Uuid>() else { return Ok(None) };
        let found = comment::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .filter(|row| row.status != Status::Deleted.as_str());
        Ok(found.map(|row| {
            let owners = row.user_id.into_iter().collect();
            if row.status == Status::Published.as_str() {
                let title: String = row.body.chars().take(80).collect();
                Resource::new(title).public(true).locale(row.locale).owner_user_ids(owners)
            } else {
                Resource::new(String::new()).owner_user_ids(owners)
            }
        }))
    }

    async fn resolve_many(
        &self,
        tenant: &TenantContext,
        ids: &[String],
        _locale: Option<&str>,
    ) -> Result<HashMap<String, Resource>, Error> {
        let mut found = HashMap::new();
        for id in ids.iter().take(100) {
            if let Some(resource) = self.resolve(tenant, id).await? {
                found.insert(id.clone(), resource);
            }
        }
        Ok(found)
    }
}

/// Bên nghe event ẩn / hiện của `compliance`.
pub(crate) struct Listener(pub Arc<CommentsService>);

#[async_trait::async_trait]
impl EventHandler for Listener {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some((change, notice)) = restriction(event) else {
            return Ok(());
        };
        let Some(id) = notice.id_in(COMMENT).and_then(|id| id.parse::<Uuid>().ok()) else {
            return Ok(());
        };
        let restrict = match change {
            RestrictionChange::Restricted => true,
            RestrictionChange::Restored => false,
            _ => return Ok(()),
        };
        self.0.set_restricted(&event.tenant, id, restrict).await
    }
}

impl CommentsService {
    /// Ẩn (`true`) / hiện lại (`false`) bình luận `id` vì vụ việc. Không có, đã xoá, đã ở đúng trạng
    /// thái: không làm gì.
    pub(crate) async fn set_restricted(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        restrict: bool,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        let Some(before) = comment::Entity::find_by_id(id).scoped(tenant).one(&transaction).await?
        else {
            return Ok(());
        };
        let from = Status::of(&before);
        if from == Status::Deleted || before.restricted_status.is_some() == restrict {
            return Ok(());
        }
        let to = if restrict {
            Status::Rejected
        } else {
            // Người kiểm duyệt không đổi được bình luận đang ẩn, nên nó vẫn là `rejected`.
            before.restricted_status.as_deref().and_then(Status::parse).unwrap_or(from)
        };
        let mut row = before.clone().into_active_model();
        row.status = Set(to.as_str().to_owned());
        row.updated_at = Set(now());
        if restrict {
            row.restricted_status = Set(Some(from.as_str().to_owned()));
            row.hold_reason = Set(Some("compliance".to_owned()));
        } else {
            row.restricted_status = Set(None);
            if to == Status::Published {
                row.hold_reason = Set(None);
            }
            if to == Status::Published && before.published_at.is_none() {
                row.published_at = Set(Some(now()));
            }
        }
        let after = update_if_unchanged(&transaction, tenant, row, &before).await?;
        if from != to {
            shift_counts(&transaction, tenant, &after, Some(from), Some(to)).await?;
        }
        let change = Change { before: Some(summary(&before)), after: Some(summary(&after)) };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::system(),
                events::comment_subject(id),
                &events::CommentStatusChanged(change),
            )
            .await?;
        transaction.commit().await?;
        tracing::info!(comment_id = %id, restricted = restrict, "comment restriction changed");
        Ok(())
    }
}
