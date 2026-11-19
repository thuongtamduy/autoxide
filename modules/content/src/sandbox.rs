//! Trang trong sandbox (bước 38, D-232, D-20261119-step38-sandbox-copy).
//!
//! - Sao: mọi trang (không phải bài viết, không phải trang đã lưu trữ) của khách thật sang sandbox,
//!   trang cha trước trang con, cùng slug; trang đang đăng thì đăng ở sandbox. Không sao ảnh (file
//!   của khách không sang sandbox), trường tự định nghĩa, bản dịch nối nhau, lịch sử revision.
//! - Đẩy (`content.page`): trang sandbox thành BẢN NHÁP ở khách thật qua service như REST, với quyền
//!   của người gọi. Trang sao từ khách thật sửa bản nháp của trang gốc (trang đang đăng giữ nguyên,
//!   mang `has_unpublished_changes`); trang tạo mới trong sandbox thành trang nháp mới.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::tenancy::sandbox::{
    CopyContext, Promotable, PromoteApply, PromoteRequest, Promoted, SandboxCopy, copied,
    link_copy, promote_record_not_found, promoted_target,
};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::entry;
use crate::entries::{Entry, EntryFields, Kind, NewEntry};
use crate::service::ContentService;

/// Loại bản ghi của trang (bảng nối của kernel, API promote).
pub(crate) const PAGE_KIND: &str = "content.page";

/// Nội dung trang đem sao / đẩy (không ảnh, không trường tự định nghĩa).
fn fields_of(row: &Entry) -> EntryFields {
    EntryFields {
        title: Some(row.title.clone()),
        slug: Some(row.slug.clone()),
        excerpt: Some(row.excerpt.clone()),
        body: serde_json::from_str(&row.body).ok(),
        featured_media_id: None,
        featured_media_alt: None,
        meta_title: Some(row.meta_title.clone()),
        meta_description: Some(row.meta_description.clone()),
        og_media_id: None,
        canonical_url: Some(row.canonical_url.clone()),
        noindex: Some(row.noindex),
        ai_generated: Some(row.ai_generated),
        ai_assisted: Some(row.ai_assisted),
        translated_from: None,
        custom_fields: None,
    }
}

fn fingerprint_of(row: &Entry, parent: Option<Uuid>) -> Result<String, Error> {
    let body: Value = serde_json::from_str(&row.body).unwrap_or(Value::Null);
    autoxide_kernel::reference::fingerprint(&json!({
        "title": row.title,
        "slug": row.slug,
        "excerpt": row.excerpt,
        "body": body,
        "meta_title": row.meta_title,
        "meta_description": row.meta_description,
        "canonical_url": row.canonical_url,
        "noindex": row.noindex,
        "locale": row.locale,
        "parent": parent,
    }))
}

/// Lỗi của một trang mà lần sao bỏ qua (dữ liệu khách không hợp ở sandbox) thay vì dừng cả job.
fn skippable(error: &Error) -> bool {
    matches!(error.kind(), ErrorKind::Validation | ErrorKind::Conflict)
}

pub(crate) struct Pages {
    pub(crate) service: Arc<ContentService>,
}

impl Pages {
    async fn find(tenant: &TenantContext, id: Uuid) -> Result<Option<Entry>, Error> {
        Ok(entry::Entity::find_by_id(id)
            .filter(entry::Column::Kind.eq(Kind::Page.as_str()))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?)
    }

    async fn copy_one(
        &self,
        context: &CopyContext<'_>,
        row: &Entry,
        parent: Option<Uuid>,
    ) -> Result<Uuid, Error> {
        let new = NewEntry {
            kind: Kind::Page,
            locale: Some(row.locale.clone()),
            parent_id: parent,
            org_id: None,
            fields: fields_of(row),
        };
        let made = self.service.create(context.sandbox, context.principal, new).await?;
        link_copy(context.sandbox, PAGE_KIND, made.id, row.id).await?;
        if row.status == "published" {
            let reviewed = row.ai_generated || row.ai_assisted;
            self.service
                .publish(context.sandbox, context.principal, made.id, None, None, reviewed)
                .await?;
        }
        Ok(made.id)
    }
}

#[async_trait::async_trait]
impl SandboxCopy for Pages {
    async fn copy(&self, context: &CopyContext<'_>) -> Result<u64, Error> {
        let rows = entry::Entity::find()
            .filter(entry::Column::Kind.eq(Kind::Page.as_str()))
            .filter(entry::Column::Status.ne("archived"))
            .scoped(context.source)
            .order_by_asc(entry::Column::Id)
            .all(context.source.database().read())
            .await?;
        // Trang cha trước trang con: mỗi lượt sao các trang mà cha đã có ở sandbox (hay không có
        // cha); cây sâu tối đa 5 cấp nên vài lượt là xong.
        let mut pending = rows;
        let mut count = 0;
        loop {
            let mut waiting = Vec::new();
            let mut progressed = false;
            for row in pending {
                if copied(context.sandbox, PAGE_KIND, row.id).await?.is_some() {
                    progressed = true;
                    continue;
                }
                let parent = match row.parent_id {
                    None => None,
                    Some(source_parent) => {
                        match copied(context.sandbox, PAGE_KIND, source_parent).await? {
                            Some(parent) => Some(parent),
                            None => {
                                waiting.push(row);
                                continue;
                            }
                        }
                    }
                };
                progressed = true;
                match self.copy_one(context, &row, parent).await {
                    Ok(_) => count += 1,
                    Err(error) if skippable(&error) => {
                        tracing::warn!(entry_id = %row.id, code = error.code(), "page not copied into the sandbox");
                    }
                    Err(error) => return Err(error),
                }
            }
            if waiting.is_empty() || !progressed {
                break;
            }
            pending = waiting;
        }
        Ok(count)
    }
}

#[async_trait::async_trait]
impl PromoteApply for Pages {
    async fn promote(
        &self,
        request: &PromoteRequest<'_>,
        previous: Option<&str>,
    ) -> Result<(Promoted, Option<String>), Error> {
        let row = Self::find(request.sandbox, request.id)
            .await?
            .ok_or_else(|| promote_record_not_found(PAGE_KIND, request.id))?;
        // Trang cha ở khách thật: bản gốc / bản đã đẩy của trang cha trong sandbox, nếu còn.
        let parent = match row.parent_id {
            Some(parent) => match promoted_target(request.sandbox, PAGE_KIND, parent).await? {
                Some(target) if Self::find(request.target, target).await?.is_some() => Some(target),
                _ => None,
            },
            None => None,
        };
        let fingerprint = fingerprint_of(&row, parent)?;
        let existing = match request.existing {
            Some(id) => Self::find(request.target, id).await?.map(|row| row.id),
            None => None,
        };
        let principal: &Principal = request.principal;
        let outcome = match existing {
            Some(target) if previous == Some(fingerprint.as_str()) => {
                return Ok((Promoted::Unchanged(target), Some(fingerprint)));
            }
            Some(target) => self
                .service
                .update(request.target, principal, target, fields_of(&row), true, None)
                .await
                .map(|entry| Promoted::UpdatedDraft(entry.id)),
            None => {
                let new = NewEntry {
                    kind: Kind::Page,
                    locale: Some(row.locale.clone()),
                    parent_id: parent,
                    org_id: None,
                    fields: fields_of(&row),
                };
                self.service
                    .create(request.target, principal, new)
                    .await
                    .map(|entry| Promoted::CreatedDraft(entry.id))
            }
        };
        match outcome {
            Ok(promoted) => Ok((promoted, Some(fingerprint))),
            Err(error) if error.code() == "content.slug_taken" => Ok((
                Promoted::Conflict { field: "slug".to_owned(), message: error.to_string() },
                None,
            )),
            Err(error) => Err(error),
        }
    }
}

/// Loại đẩy được của `content`.
pub(crate) fn promotable(service: Arc<ContentService>) -> Vec<Promotable> {
    vec![Promotable::new(
        PAGE_KIND,
        "A page: becomes a draft page in the real tenant (or a pending change of the page it was \
         copied from); never published by the promotion.",
        Arc::new(Pages { service }),
    )]
}
