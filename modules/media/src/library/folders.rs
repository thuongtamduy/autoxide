//! Thư mục lồng nhau của thư viện media.
//!
//! Đúng trên cả hai database, kể cả khi nhiều request chạy cùng lúc (D-086):
//!
//! - tên không trùng trong cùng thư mục cha: unique index `(parent_id, name_key)`; thư mục
//!   gốc là UUID toàn số 0 nên luật này áp cả ở gốc;
//! - chỉ xoá thư mục trống: xoá dòng trước rồi đếm con trong cùng transaction; ai thêm
//!   item / thư mục con vào thì "chạm" thư mục đó trước (`touch_folder`), nên hai bên chờ
//!   nhau và bên sau thấy kết quả của bên trước;
//! - không chuyển thư mục vào chính nó hay thư mục con của nó: mọi lệnh chuyển cập nhật
//!   dòng `media_lock.folder_tree` của khách trước, nên chạy lần lượt và thấy cây đã đổi của
//!   nhau. Mỗi khách một cây, một dòng khoá (gieo khi tạo khách, [`seed_lock`]).
//!
//! Theo khách (bước 33): mọi truy vấn lọc theo khách; tên thư mục chỉ không trùng trong cùng khách
//! (unique `(tenant_id, parent_id, name_key)`, thư mục gốc vẫn là UUID toàn số 0).

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, SortValue, fetch_page, fetch_page_by};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use chrono::{DateTime, Utc};
use sea_orm::sea_query::{Expr, ExprTrait as _, OnConflict};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, PaginatorTrait,
    QueryFilter, Set, SqlErr,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::folder::{self, Column, Entity};
use crate::entity::{ROOT, api_folder, item, lock, stored_folder};
use crate::events::{self, Change};
use crate::permissions::{MEDIA_READ, MEDIA_WRITE};
use crate::service::{FolderFilter, MediaService, check_name, folder_gone, now, touch_folder};

/// Một thư mục.
pub(crate) type Folder = folder::Model;

/// Tên dòng khoá cây thư mục trong `media_lock`.
const FOLDER_TREE: &str = "folder_tree";

/// Độ sâu tối đa của cây thư mục; sâu hơn coi như dữ liệu hỏng (vòng lặp).
const MAX_DEPTH: usize = 1000;

// Sắp danh sách thư mục theo gì.
/// Field to sort the folder list by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FolderSort {
    /// Creation time (the default).
    #[default]
    CreatedAt,
    /// Name, ignoring case.
    Name,
}

/// Thay đổi một thư mục; trường `None` giữ nguyên.
#[derive(Debug, Clone, Default)]
pub(crate) struct FolderChanges {
    pub name: Option<String>,
    /// `Some(None)`: chuyển về gốc.
    pub parent_id: Option<Option<Uuid>>,
}

impl MediaService {
    /// Tạo thư mục `name` trong `parent` (`None`: gốc). Cần `media.write`.
    pub(crate) async fn create_folder(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        name: &str,
        parent: Option<Uuid>,
    ) -> Result<Folder, Error> {
        principal.require(&MEDIA_WRITE)?;
        let mut problems = Vec::new();
        let name = check_name(name, &mut problems);
        if let Some(parent) = parent {
            self.check_folder_exists(tenant, parent, "parent_id", &mut problems).await?;
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let parent = stored_folder(parent);
        let now = now();
        let transaction = tenant.database().begin_write().await?;
        if !touch_folder(&transaction, tenant, parent).await? {
            return Err(folder_gone("parent_id"));
        }
        let folder = folder::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            parent_id: Set(parent),
            name_key: Set(autoxide_kernel::search::normalize(&name)),
            name: Set(name.clone()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&transaction)
        .await
        .map_err(|error| name_conflict(error, &name))?;
        let event = events::FolderCreated(Change { before: None, after: Some(snapshot(&folder)) });
        let actor = Actor::from(principal);
        self.outbox
            .record(&transaction, tenant, actor, events::folder_subject(folder.id), &event)
            .await?;
        transaction.commit().await?;
        Ok(folder)
    }

    /// Một trang thư mục. Cần `media.read`.
    pub(crate) async fn list_folders(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        parent: FolderFilter,
        sort: FolderSort,
        page: &PageRequest,
    ) -> Result<Page<Folder>, Error> {
        principal.require(&MEDIA_READ)?;
        let select = Entity::find().scoped(tenant);
        let select = match parent {
            FolderFilter::Any => select,
            FolderFilter::Root => select.filter(Column::ParentId.eq(ROOT)),
            FolderFilter::Id(id) => select.filter(Column::ParentId.eq(id)),
        };
        let read = tenant.database().read();
        match sort {
            FolderSort::CreatedAt => fetch_page(select, Column::Id, |row| row.id, page, read).await,
            FolderSort::Name => {
                let key = |row: &Folder| SortValue::Text(row.name_key.clone());
                fetch_page_by(select, Column::NameKey, key, Column::Id, |row| row.id, page, read)
                    .await
            }
        }
    }

    /// Thư mục `id`. Cần `media.read`.
    pub(crate) async fn get_folder(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Folder, Error> {
        principal.require(&MEDIA_READ)?;
        Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| folder_not_found(id))
    }

    /// Đổi tên hay chuyển thư mục `id`. Cần `media.write`.
    pub(crate) async fn update_folder(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        changes: FolderChanges,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Folder, Error> {
        principal.require(&MEDIA_WRITE)?;
        let mut problems = Vec::new();
        let name = changes.name.as_deref().map(|name| check_name(name, &mut problems));
        if let Some(Some(parent)) = changes.parent_id {
            self.check_folder_exists(tenant, parent, "parent_id", &mut problems).await?;
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }

        let transaction = tenant.database().begin_write().await?;
        if changes.parent_id.is_some() {
            // Lệnh chuyển (của cùng khách) chạy lần lượt (xem đầu file).
            lock_tree(&transaction, tenant).await?;
        }
        let before = Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| folder_not_found(id))?;
        let mut row = before.clone().into_active_model();
        if let Some(name) = &name {
            row.name = Set(name.clone());
            row.name_key = Set(autoxide_kernel::search::normalize(name));
        }
        if let Some(parent) = changes.parent_id {
            let parent = stored_folder(parent);
            if parent != before.parent_id {
                if contains(&transaction, tenant, id, parent).await? {
                    return Err(Error::conflict(
                        "media.folder_cycle",
                        "A folder cannot be moved into itself or into one of its subfolders.",
                    ));
                }
                if !touch_folder(&transaction, tenant, parent).await? {
                    return Err(folder_gone("parent_id"));
                }
            }
            row.parent_id = Set(parent);
        }
        row.updated_at = Set(now());
        let mut update = Entity::update(row).validate()?.scoped(tenant);
        if let Some(expected) = expected {
            update = update.filter(Column::UpdatedAt.eq(expected));
        }
        let after = match update.exec(&transaction).await {
            Ok(after) => after,
            Err(sea_orm::DbErr::RecordNotUpdated) => {
                return Err(autoxide_kernel::version::changed());
            }
            Err(error) => {
                return Err(name_conflict(error, name.as_deref().unwrap_or(&before.name)));
            }
        };
        let change = Change { before: Some(snapshot(&before)), after: Some(snapshot(&after)) };
        let actor = Actor::from(principal);
        let event = events::FolderUpdated(change);
        self.outbox.record(&transaction, tenant, actor, events::folder_subject(id), &event).await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Xoá thư mục `id` nếu trống (không thư mục con, không item — kể cả item trong thùng
    /// rác). Cần `media.write`.
    pub(crate) async fn delete_folder(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        principal.require(&MEDIA_WRITE)?;
        let transaction = tenant.database().begin_write().await?;
        let folder = Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| folder_not_found(id))?;
        // Xoá dòng TRƯỚC khi đếm: dòng bị khoá tới hết transaction, ai đang thêm vào thư
        // mục này phải chờ (xem đầu file).
        let deleted = Entity::delete_by_id(id).scoped(tenant).exec(&transaction).await?;
        if deleted.rows_affected == 0 {
            transaction.rollback().await?;
            return Err(folder_not_found(id));
        }
        let folders = Entity::find()
            .scoped(tenant)
            .filter(Column::ParentId.eq(id))
            .count(&transaction)
            .await?;
        let items = item::Entity::find()
            .scoped(tenant)
            .filter(item::Column::FolderId.eq(id))
            .count(&transaction)
            .await?;
        if folders + items > 0 {
            transaction.rollback().await?;
            return Err(Error::conflict(
                "media.folder_not_empty",
                "The folder still holds {folders} folders and {items} items (items in the trash \
                 count too). Move or delete them first.",
            )
            .with_param("folders", folders)
            .with_param("items", items));
        }
        let event = events::FolderDeleted(Change { before: Some(snapshot(&folder)), after: None });
        let actor = Actor::from(principal);
        self.outbox.record(&transaction, tenant, actor, events::folder_subject(id), &event).await?;
        transaction.commit().await?;
        Ok(())
    }
}

/// Thư mục `ancestor` có phải `folder` hay một thư mục cha (ông…) của nó không.
async fn contains<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    ancestor: Uuid,
    folder: Uuid,
) -> Result<bool, Error> {
    let mut current = folder;
    for _ in 0..MAX_DEPTH {
        if current == ancestor {
            return Ok(true);
        }
        if current == ROOT {
            return Ok(false);
        }
        match Entity::find_by_id(current).scoped(tenant).one(connection).await? {
            Some(row) => current = row.parent_id,
            None => return Ok(false),
        }
    }
    Err(Error::internal("the media folder tree is deeper than expected; it may contain a cycle"))
}

/// Gieo dòng khoá cây thư mục của khách `tenant` (khi tạo khách). Chạy lại vô hại.
pub(crate) async fn seed_lock<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
) -> Result<(), Error> {
    let row = lock::ActiveModel {
        tenant_id: Set(tenant.id().as_uuid()),
        name: Set(FOLDER_TREE.to_owned()),
        counter: Set(0),
    };
    lock::Entity::insert(row)
        .on_conflict(
            OnConflict::columns([lock::Column::TenantId, lock::Column::Name])
                .do_nothing()
                .to_owned(),
        )
        .exec_without_returning(connection)
        .await?;
    Ok(())
}

/// Cập nhật dòng khoá cây thư mục của khách `tenant` (khoá tới hết `transaction`). Khách chưa có
/// dòng (tạo trước khi module có bộ gieo) thì gieo ngay rồi khoá.
async fn lock_tree<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
) -> Result<(), Error> {
    for _ in 0..2 {
        let locked = lock::Entity::update_many()
            .col_expr(lock::Column::Counter, Expr::col(lock::Column::Counter).add(1))
            .filter(lock::Column::Name.eq(FOLDER_TREE))
            .scoped(tenant)
            .exec(transaction)
            .await?;
        if locked.rows_affected > 0 {
            return Ok(());
        }
        seed_lock(transaction, tenant).await?;
    }
    Err(Error::internal("the media folder lock row cannot be created"))
}

/// Vi phạm index unique (thư mục cha, tên) thành lỗi 409 dễ hiểu.
fn name_conflict(error: sea_orm::DbErr, name: &str) -> Error {
    match error.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(_)) => Error::conflict(
            "media.folder_name_taken",
            "This folder already holds a folder named `{name}` (names ignore case). Choose \
             another name.",
        )
        .with_param("name", name),
        _ => Error::from(error),
    }
}

pub(crate) fn folder_not_found(id: Uuid) -> Error {
    Error::not_found("media.folder_not_found", "Media folder `{id}` does not exist.")
        .with_param("id", id)
}

fn snapshot(folder: &Folder) -> Value {
    json!({"name": folder.name, "parent_id": api_folder(folder.parent_id)})
}
