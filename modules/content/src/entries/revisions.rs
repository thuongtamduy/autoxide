//! Revision: ảnh chụp đầy đủ các trường sửa được của một mục (D-102). Tạo ở service, nên
//! REST và MCP như nhau. Cây trang (cha), term và tác giả không nằm trong revision — chúng
//! có hiệu lực ngay, không qua nháp.

use autoxide_kernel::error::Error;
use chrono::{DateTime, Utc};
use sea_orm::{ActiveModelTrait, ConnectionTrait, Set};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::entity::{entry, revision};

/// Vì sao có revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reason {
    Initial,
    ExplicitSave,
    BackgroundSave,
    Published,
    Unpublished,
    Restored,
    Scheduled,
}

impl Reason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Reason::Initial => "initial",
            Reason::ExplicitSave => "explicit_save",
            Reason::BackgroundSave => "background_save",
            Reason::Published => "published",
            Reason::Unpublished => "unpublished",
            Reason::Restored => "restored",
            Reason::Scheduled => "scheduled",
        }
    }
}

/// Các trường sửa được của một mục, như lưu trong `content_revisions.snapshot`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Snapshot {
    pub title: String,
    pub slug: String,
    pub excerpt: String,
    pub body: Value,
    pub featured_media_id: Option<Uuid>,
    pub featured_media_alt: String,
    pub meta_title: String,
    pub meta_description: String,
    pub og_media_id: Option<Uuid>,
    pub canonical_url: String,
    pub noindex: bool,
    /// Revision cũ (trước D-109) không có trường này: coi là `false`.
    #[serde(default)]
    pub ai_generated: bool,
}

impl Snapshot {
    pub(crate) fn of(entry: &entry::Model) -> Self {
        Snapshot {
            title: entry.title.clone(),
            slug: entry.slug.clone(),
            excerpt: entry.excerpt.clone(),
            body: serde_json::from_str(&entry.body)
                .unwrap_or_else(|_| crate::entries::body::empty()),
            featured_media_id: entry.featured_media_id,
            featured_media_alt: entry.featured_media_alt.clone(),
            meta_title: entry.meta_title.clone(),
            meta_description: entry.meta_description.clone(),
            og_media_id: entry.og_media_id,
            canonical_url: entry.canonical_url.clone(),
            noindex: entry.noindex,
            ai_generated: entry.ai_generated,
        }
    }
}

/// Ghi một revision của `entry` (trạng thái hiện tại của nó), trong `connection`.
pub(crate) async fn record<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    entry: &entry::Model,
    reason: Reason,
    by: Option<Uuid>,
    at: DateTime<Utc>,
) -> Result<revision::Model, Error> {
    let snapshot = serde_json::to_string(&Snapshot::of(entry)).map_err(Error::internal)?;
    Ok(revision::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant.id().as_uuid()),
        entry_id: Set(entry.id),
        snapshot: Set(snapshot),
        reason: Set(reason.as_str().to_owned()),
        created_by: Set(by),
        created_at: Set(at),
    }
    .insert(connection)
    .await?)
}

use autoxide_kernel::access::Principal;
use autoxide_kernel::database::Database;
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext, TenantId};
use sea_orm::sea_query::ExprTrait as _;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

use crate::entries::{Entry, EntryFields};
use crate::permissions::{self, CONTENT_READ};
use crate::service::ContentService;

/// Một revision.
pub(crate) type Revision = revision::Model;

pub(crate) const PRUNE: &str = "content.prune_revisions";

impl ContentService {
    /// Revision của mục `entry_id`, mới trước (mặc định). Cần `content.read`.
    pub(crate) async fn list_revisions(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        entry_id: Uuid,
        page: &PageRequest,
    ) -> Result<Page<Revision>, Error> {
        self.get(tenant, principal, entry_id).await?;
        let select =
            revision::Entity::find().scoped(tenant).filter(revision::Column::EntryId.eq(entry_id));
        fetch_page(select, revision::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Revision `id` của mục `entry_id`. Cần `content.read`.
    pub(crate) async fn get_revision(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        entry_id: Uuid,
        id: Uuid,
    ) -> Result<Revision, Error> {
        principal.require(&CONTENT_READ)?;
        revision::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .filter(|row| row.entry_id == entry_id)
            .ok_or_else(|| revision_not_found(id))
    }

    /// Chép revision `id` vào bản nháp của mục `entry_id` (sinh revision `restored`); không tự
    /// đăng. Cần `content.write`.
    pub(crate) async fn restore(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        entry_id: Uuid,
        id: Uuid,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Entry, Error> {
        permissions::require_create(principal)?;
        // Không cần `content.read`: khôi phục là sửa mục, quyền sửa (kể cả `content.write_own`
        // trên mục của mình) kiểm ở `update_inner`; ở đây chỉ kiểm revision thuộc mục.
        let revision = revision::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .filter(|row| row.entry_id == entry_id)
            .ok_or_else(|| revision_not_found(id))?;
        // Quyền sửa mục (kể cả `content.write_own` trên mục của mình) kiểm ở `update_inner`.
        let snapshot: Snapshot =
            serde_json::from_str(&revision.snapshot).map_err(Error::internal)?;
        self.update_inner(
            tenant,
            principal,
            entry_id,
            fields_of(snapshot),
            crate::entries::Revise { forced: Some(Reason::Restored), restored_from: Some(id) },
            expected,
        )
        .await
    }

    /// Bớt revision của mỗi mục trên `database` xuống `AUTOXIDE_CONTENT_MAX_REVISIONS`, cũ đi
    /// trước — trừ revision bản công khai hay một lịch đang chờ còn trỏ tới. Job theo lịch chạy
    /// một lần mỗi database (bước 33): tìm (khách, mục) có quá nhiều revision trên cả database,
    /// rồi xoá theo từng khách. Trả số revision đã xoá.
    pub(crate) async fn prune_revisions(&self, database: &Database) -> Result<u64, Error> {
        let keep = u64::from(self.config.max_revisions);
        // Quét cả database qua index `(tenant_id, entry_id, …)`.
        let crowded: Vec<(Uuid, Uuid)> = revision::Entity::find()
            .select_only()
            .column(revision::Column::TenantId)
            .column(revision::Column::EntryId)
            .group_by(revision::Column::TenantId)
            .group_by(revision::Column::EntryId)
            .having(revision::Column::Id.count().gt(i64::try_from(keep).unwrap_or(i64::MAX)))
            .into_tuple()
            .all(database.read())
            .await?;
        let mut by_tenant: std::collections::BTreeMap<Uuid, Vec<Uuid>> = Default::default();
        for (tenant, entry) in crowded {
            by_tenant.entry(tenant).or_default().push(entry);
        }
        let mut removed = 0;
        for (id, entries) in by_tenant {
            match self.databases.tenant(TenantId::from_uuid(id)).await {
                Ok(tenant) => removed += self.prune_tenant(&tenant, keep, entries).await?,
                // Khách đã bị xoá khỏi sổ: dữ liệu còn lại do việc xoá khách lo, không đoán ở đây.
                Err(error) => {
                    tracing::warn!(tenant_id = %id, error = %error, "content revision pruning skipped a tenant");
                }
            }
        }
        if removed > 0 {
            tracing::info!(revisions = removed, "old content revisions removed");
        }
        Ok(removed)
    }

    /// Phần của khách `tenant` trong [`prune_revisions`](Self::prune_revisions): các mục `crowded`.
    async fn prune_tenant(
        &self,
        tenant: &TenantContext,
        keep: u64,
        crowded: Vec<Uuid>,
    ) -> Result<u64, Error> {
        let mut removed = 0;
        for entry_id in crowded {
            let ids: Vec<Uuid> = revision::Entity::find()
                .scoped(tenant)
                .select_only()
                .column(revision::Column::Id)
                .filter(revision::Column::EntryId.eq(entry_id))
                .order_by_desc(revision::Column::CreatedAt)
                .order_by_desc(revision::Column::Id)
                .into_tuple::<Uuid>()
                .all(tenant.database().read())
                .await?
                .into_iter()
                .skip(usize::try_from(keep).unwrap_or(usize::MAX))
                .collect();
            let mut protected: Vec<Uuid> = crate::entity::published::Entity::find()
                .scoped(tenant)
                .filter(crate::entity::published::Column::EntryId.eq(entry_id))
                .one(tenant.database().read())
                .await?
                .map(|row| row.revision_id)
                .into_iter()
                .collect();
            protected.extend(
                crate::entity::schedule::Entity::find()
                    .scoped(tenant)
                    .filter(crate::entity::schedule::Column::EntryId.eq(entry_id))
                    .filter(crate::entity::schedule::Column::Status.eq("scheduled"))
                    .all(tenant.database().read())
                    .await?
                    .into_iter()
                    .filter_map(|row| row.revision_id),
            );
            let doomed: Vec<Uuid> = ids.into_iter().filter(|id| !protected.contains(id)).collect();
            if doomed.is_empty() {
                continue;
            }
            let deleted = revision::Entity::delete_many()
                .scoped(tenant)
                .filter(revision::Column::Id.is_in(doomed))
                // Lịch có thể vừa ghim một revision trong lúc này: không xoá revision đang được trỏ tới.
                .filter(
                    revision::Column::Id.not_in_subquery(
                        sea_orm::sea_query::Query::select()
                            .column(crate::entity::schedule::Column::RevisionId)
                            .from(crate::entity::schedule::Entity)
                            .and_where(
                                crate::entity::schedule::Column::TenantId.eq(tenant.id().as_uuid()),
                            )
                            .and_where(crate::entity::schedule::Column::Status.eq("scheduled"))
                            .and_where(crate::entity::schedule::Column::RevisionId.is_not_null())
                            .to_owned(),
                    ),
                )
                // Một lịch có thể vừa chạy xong: revision nó ghim giờ là bản công khai.
                .filter(
                    revision::Column::Id.not_in_subquery(
                        sea_orm::sea_query::Query::select()
                            .column(crate::entity::published::Column::RevisionId)
                            .from(crate::entity::published::Entity)
                            .and_where(
                                crate::entity::published::Column::TenantId
                                    .eq(tenant.id().as_uuid()),
                            )
                            .to_owned(),
                    ),
                )
                .exec(tenant.database().write())
                .await?;
            removed += deleted.rows_affected;
        }
        Ok(removed)
    }

    /// Số revision của mục `entry_id` (cho test).
    #[cfg(test)]
    pub(crate) async fn revision_count(&self, tenant: &TenantContext, entry_id: Uuid) -> u64 {
        use sea_orm::PaginatorTrait as _;
        revision::Entity::find()
            .scoped(tenant)
            .filter(revision::Column::EntryId.eq(entry_id))
            .count(tenant.database().read())
            .await
            .unwrap_or(0)
    }
}

/// Job hằng ngày: dọn revision cũ.
pub(crate) struct Prune(pub std::sync::Arc<ContentService>);

#[async_trait::async_trait]
impl JobHandler for Prune {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.prune_revisions(&job.database).await.map(|_| ())
    }
}

/// Các trường của một ảnh chụp, dạng thay đổi cho bản nháp.
pub(crate) fn fields_of(snapshot: Snapshot) -> EntryFields {
    EntryFields {
        title: Some(snapshot.title),
        slug: Some(snapshot.slug),
        excerpt: Some(snapshot.excerpt),
        body: Some(snapshot.body),
        featured_media_id: Some(snapshot.featured_media_id),
        featured_media_alt: Some(snapshot.featured_media_alt),
        meta_title: Some(snapshot.meta_title),
        meta_description: Some(snapshot.meta_description),
        og_media_id: Some(snapshot.og_media_id),
        canonical_url: Some(snapshot.canonical_url),
        noindex: Some(snapshot.noindex),
        ai_generated: Some(snapshot.ai_generated),
        translated_from: None,
    }
}

/// Revision `id`, đọc thẳng (không kiểm quyền — cho job).
pub(crate) async fn revision_entity<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    id: Uuid,
) -> Result<Revision, Error> {
    revision::Entity::find_by_id(id)
        .scoped(tenant)
        .one(connection)
        .await?
        .ok_or_else(|| revision_not_found(id))
}

pub(crate) fn revision_not_found(id: Uuid) -> Error {
    Error::not_found("content.revision_not_found", "This entry has no revision `{id}`.")
        .with_param("id", id)
}
