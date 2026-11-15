//! Service của thư viện media: MỌI logic nằm ở đây và ở các file theo tính năng — `library/`
//! (tải lên, loại thật, thư mục, nhãn), `images/` (ảnh thu nhỏ, alt), `jobs.rs` (dọn định kỳ).
//! Handler REST và tool MCP chỉ lấy dữ liệu vào rồi gọi service — hai đường cho cùng kết
//! quả, cùng permission (luật kiến trúc 8). Mỗi lệnh ghi đi cùng một event trong CÙNG
//! transaction (luật kiến trúc 3).
//!
//! Nội dung nằm ở nơi lưu chung (`Storage`, khoá `media/<id>/<version_id>`), thông tin nằm
//! trong `media_item`. Thứ tự ghi để không bao giờ có dòng trỏ tới nội dung không có:
//! ghi nội dung trước, rồi dòng; xoá dòng trước, rồi nội dung. Nội dung còn sót lại (xoá
//! hỏng, máy chủ tắt giữa chừng) do job dọn mồ côi xoá sau (`jobs.rs`).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::database::Database;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::{Actor, Delivered, EventHandler, Outbox};
use autoxide_kernel::job::JobQueue;
use autoxide_kernel::pagination::{
    Page, PageRequest, Pagination, SortValue, fetch_page, fetch_page_by,
};
use autoxide_kernel::privacy::{PersonalData, deleted_user};
use autoxide_kernel::rows;
use autoxide_kernel::storage::{Storage, StoredFile};
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::config::MediaConfig;
use crate::entity::item::{self, Column, Entity};
use crate::entity::{api_folder, folder, stored_folder};
use crate::events::{self, Change};
use crate::permissions::{MEDIA_PURGE, MEDIA_READ, MEDIA_WRITE};

/// Một item của thư viện.
pub(crate) type Item = item::Model;

/// Độ dài tối đa, theo ký tự.
pub(crate) const NAME_MAX_LENGTH: usize = 255;
pub(crate) const ALT_MAX_LENGTH: usize = 1000;
const TITLE_MAX_LENGTH: usize = 255;
const CAPTION_MAX_LENGTH: usize = 2000;
/// Số item trùng nội dung báo lại khi tải lên.
const DUPLICATES_SHOWN: u64 = 20;

/// Who may load the content without signing in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Visibility {
    /// Only users with `media.read`.
    Private,
    /// Anyone with the link, signed in or not (while the item is not in the trash).
    Public,
}

impl Visibility {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Visibility::Private => "private",
            Visibility::Public => "public",
        }
    }

    pub(crate) fn of(item: &Item) -> Visibility {
        if item.visibility == "public" { Visibility::Public } else { Visibility::Private }
    }
}

/// Item nằm ở thư mục nào khi lọc danh sách.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum FolderFilter {
    /// Mọi thư mục.
    #[default]
    Any,
    /// Chỉ thư mục gốc.
    Root,
    /// Chỉ thư mục này (không gồm thư mục con).
    Id(Uuid),
}

// Sắp danh sách theo gì.
/// Field to sort the list by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ItemSort {
    /// Upload time (the default).
    #[default]
    CreatedAt,
    /// Name, ignoring case.
    Name,
    /// Size in bytes.
    Size,
}

/// Bộ lọc của danh sách item.
#[derive(Debug, Clone, Default)]
pub(crate) struct ItemFilter {
    pub folder: FolderFilter,
    /// `image/png` hoặc `image/*`.
    pub content_type: Option<String>,
    pub visibility: Option<Visibility>,
    /// `true`: chỉ item trong thùng rác; `false` (mặc định): chỉ item ngoài thùng rác.
    pub trashed: bool,
    /// Tên chứa chuỗi này (không phân biệt hoa thường).
    pub search: Option<String>,
    /// Chỉ item mang nhãn này (không phân biệt hoa thường).
    pub tag: Option<String>,
    pub sort: ItemSort,
}

/// Thay đổi thông tin một item; trường `None` giữ nguyên.
#[derive(Debug, Clone, Default)]
pub(crate) struct ItemChanges {
    pub name: Option<String>,
    pub alt: Option<String>,
    pub title: Option<String>,
    pub caption: Option<String>,
    /// `Some(None)`: chuyển về thư mục gốc.
    pub folder_id: Option<Option<Uuid>>,
    pub visibility: Option<Visibility>,
    /// Thay cả bộ nhãn.
    pub tags: Option<Vec<String>>,
}

/// Một item kèm nhãn, bản dịch `alt` và ảnh thu nhỏ của nó — những gì API trả về.
#[derive(Debug, Clone)]
pub(crate) struct ItemDetails {
    pub item: Item,
    pub tags: Vec<String>,
    /// `alt` theo ngôn ngữ (ngôn ngữ → alt), D-202.
    pub alts: std::collections::BTreeMap<String, String>,
    /// Ảnh thu nhỏ theo cỡ, hẹp trước.
    pub variants: Vec<crate::images::variants::VariantPair>,
}

pub(crate) struct MediaService {
    pub(crate) database: Database,
    pub(crate) storage: Arc<dyn Storage>,
    pub(crate) outbox: Outbox,
    pub(crate) jobs: JobQueue,
    /// Mô hình AI của module `ai` (bản "đã tắt" khi chưa cấu hình): gợi ý alt text (D-109).
    pub(crate) ai: Arc<dyn autoxide_kernel::ai::TextModel>,
    pub(crate) config: MediaConfig,
}

impl MediaService {
    pub(crate) fn new(
        database: Database,
        storage: Arc<dyn Storage>,
        outbox: Outbox,
        jobs: JobQueue,
        ai: Arc<dyn autoxide_kernel::ai::TextModel>,
        config: MediaConfig,
    ) -> Self {
        MediaService { database, storage, outbox, jobs, ai, config }
    }

    /// Một trang item theo bộ lọc. Cần `media.read`.
    pub(crate) async fn list(
        &self,
        principal: &Principal,
        filter: &ItemFilter,
        page: &PageRequest,
    ) -> Result<Page<Item>, Error> {
        principal.require(&MEDIA_READ)?;
        let mut select = Entity::find();
        select = if filter.trashed {
            select.filter(Column::TrashedAt.is_not_null())
        } else {
            select.filter(Column::TrashedAt.is_null())
        };
        select = match filter.folder {
            FolderFilter::Any => select,
            FolderFilter::Root => select.filter(Column::FolderId.eq(crate::entity::ROOT)),
            FolderFilter::Id(id) => select.filter(Column::FolderId.eq(id)),
        };
        if let Some(pattern) = &filter.content_type {
            select = match pattern.strip_suffix("/*") {
                Some(kind) => select.filter(Column::ContentType.starts_with(format!("{kind}/"))),
                None => select.filter(Column::ContentType.eq(pattern.as_str())),
            };
        }
        if let Some(visibility) = filter.visibility {
            select = select.filter(Column::Visibility.eq(visibility.as_str()));
        }
        if let Some(search) = filter.search.as_deref().filter(|search| !search.trim().is_empty()) {
            select = select.filter(autoxide_kernel::search::contains(Column::NameKey, search));
        }
        if let Some(tag) = filter.tag.as_deref().filter(|tag| !tag.trim().is_empty()) {
            let Some(tag_id) = crate::library::tags::find_tag(self.database.read(), tag).await?
            else {
                // Không có nhãn đó: không item nào mang nó.
                let pagination = Pagination::new(page.limit(), None);
                return Ok(Page { data: Vec::new(), pagination });
            };
            select = select.filter(crate::library::tags::has_tag(tag_id));
        }
        let read = self.database.read();
        match filter.sort {
            ItemSort::CreatedAt => fetch_page(select, Column::Id, |row| row.id, page, read).await,
            ItemSort::Name => {
                let key = |row: &Item| SortValue::Text(row.name_key.clone());
                fetch_page_by(select, Column::NameKey, key, Column::Id, |row| row.id, page, read)
                    .await
            }
            ItemSort::Size => {
                let key = |row: &Item| SortValue::Integer(row.size);
                fetch_page_by(select, Column::Size, key, Column::Id, |row| row.id, page, read).await
            }
        }
    }

    /// Kèm nhãn, bản dịch `alt` và ảnh thu nhỏ cho từng item (thêm ba, bốn câu query cho cả
    /// danh sách).
    pub(crate) async fn describe(&self, items: Vec<Item>) -> Result<Vec<ItemDetails>, Error> {
        let read = self.database.read();
        let ids: Vec<Uuid> = items.iter().map(|item| item.id).collect();
        let mut tags = crate::library::tags::tags_of(read, &ids).await?;
        let mut alts = crate::images::alts::alts_of(read, &ids).await?;
        let mut variants = crate::images::variants::variants_of(read, &items).await?;
        Ok(items
            .into_iter()
            .map(|item| ItemDetails {
                tags: tags.remove(&item.id).unwrap_or_default(),
                alts: alts.remove(&item.id).unwrap_or_default(),
                variants: crate::images::variants::pair(
                    variants.remove(&item.id).unwrap_or_default(),
                ),
                item,
            })
            .collect())
    }

    /// Như [`describe`](Self::describe), cho một item.
    pub(crate) async fn describe_one(&self, item: Item) -> Result<ItemDetails, Error> {
        let details = self.describe(vec![item]).await?.into_iter().next();
        details.ok_or_else(|| Error::internal("describing a media item returned nothing"))
    }

    /// Item `id` (kể cả trong thùng rác). Cần `media.read`.
    pub(crate) async fn get(&self, principal: &Principal, id: Uuid) -> Result<Item, Error> {
        principal.require(&MEDIA_READ)?;
        Entity::find_by_id(id).one(self.database.read()).await?.ok_or_else(|| item_not_found(id))
    }

    /// Item `id` nếu người gọi được tải nội dung của nó: item công khai ngoài thùng rác thì
    /// ai cũng được (kể cả không đăng nhập); còn lại cần `media.read`. Không được thì báo
    /// như không có — không để lộ item riêng tư có tồn tại.
    pub(crate) async fn viewable(
        &self,
        principal: Option<&Principal>,
        id: Uuid,
    ) -> Result<Item, Error> {
        let item = Entity::find_by_id(id).one(self.database.read()).await?;
        let reader = principal.is_some_and(|principal| principal.has(&MEDIA_READ));
        item.filter(|item| {
            reader || (Visibility::of(item) == Visibility::Public && item.trashed_at.is_none())
        })
        .ok_or_else(|| item_not_found(id))
    }

    /// Nội dung hiện tại của `item`, cả file hoặc đoạn `range`.
    pub(crate) async fn read(
        &self,
        item: &Item,
        range: Option<std::ops::Range<u64>>,
    ) -> Result<StoredFile, Error> {
        self.read_key(&item.storage_key(), range).await
    }

    /// Nội dung ở khoá `key` (của module), cả file hoặc đoạn `range`.
    pub(crate) async fn read_key(
        &self,
        key: &str,
        range: Option<std::ops::Range<u64>>,
    ) -> Result<StoredFile, Error> {
        let stored = match range {
            None => self.storage.get(key).await?,
            Some(range) => self.storage.get_range(key, range).await?,
        };
        // Có dòng mà không có nội dung: nơi lưu bị đổi từ bên ngoài.
        stored.ok_or_else(|| {
            Error::internal(format!("media content `{key}` is missing from storage"))
        })
    }

    /// Đổi thông tin item `id`. Cần `media.write`; item trong thùng rác thì phải khôi phục
    /// trước. `expected`: phiên bản người gọi đã đọc (`If-Match`).
    pub(crate) async fn update(
        &self,
        principal: &Principal,
        id: Uuid,
        changes: ItemChanges,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Item, Error> {
        principal.require(&MEDIA_WRITE)?;
        let mut problems = Vec::new();
        let name = changes.name.as_deref().map(|name| check_name(name, &mut problems));
        for (field, value, max) in [
            ("alt", &changes.alt, ALT_MAX_LENGTH),
            ("title", &changes.title, TITLE_MAX_LENGTH),
            ("caption", &changes.caption, CAPTION_MAX_LENGTH),
        ] {
            check_length(field, value.as_deref(), max, &mut problems);
        }
        if let Some(Some(folder)) = changes.folder_id {
            self.check_folder_exists(folder, "folder_id", &mut problems).await?;
        }
        let tags = changes
            .tags
            .as_deref()
            .map(|tags| crate::library::tags::check_tags(tags, "tags", &mut problems));
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }

        let transaction = self.database.begin_write().await?;
        let before =
            Entity::find_by_id(id).one(&transaction).await?.ok_or_else(|| item_not_found(id))?;
        if before.trashed_at.is_some() {
            return Err(in_trash(id));
        }
        let mut row = before.clone().into_active_model();
        if let Some(name) = name {
            row.name_key = Set(autoxide_kernel::search::fold(&name));
            row.name = Set(name);
        }
        for (column, value) in [
            (&mut row.alt, changes.alt),
            (&mut row.title, changes.title),
            (&mut row.caption, changes.caption),
        ] {
            if let Some(value) = value {
                *column = Set(value.trim().to_owned());
            }
        }
        if let Some(folder) = changes.folder_id {
            let folder = stored_folder(folder);
            if folder != before.folder_id && !touch_folder(&transaction, folder).await? {
                return Err(folder_gone("folder_id"));
            }
            row.folder_id = Set(folder);
        }
        if let Some(visibility) = changes.visibility {
            row.visibility = Set(visibility.as_str().to_owned());
        }
        row.updated_at = Set(now());
        let mut update = Entity::update(row).validate()?.filter(Column::TrashedAt.is_null());
        if let Some(expected) = expected {
            // Kiểm phiên bản ngay trong câu `UPDATE`: người khác vừa sửa thì 0 dòng (D-084).
            update = update.filter(Column::UpdatedAt.eq(expected));
        }
        let after = match update.exec(&transaction).await {
            Ok(after) => after,
            Err(sea_orm::DbErr::RecordNotUpdated) => {
                return Err(self.why_not_changed(id, &transaction).await);
            }
            Err(error) => return Err(error.into()),
        };
        let tags_before = crate::library::tags::tags_of(&transaction, &[id])
            .await?
            .remove(&id)
            .unwrap_or_default();
        let tags_after = match &tags {
            Some(tags) => crate::library::tags::set_item_tags(&transaction, id, tags).await?,
            None => tags_before.clone(),
        };
        let event = events::ItemUpdated(Change {
            before: Some(with_tags(snapshot(&before), &tags_before)),
            after: Some(with_tags(snapshot(&after), &tags_after)),
        });
        let actor = Actor::from(principal);
        self.outbox.record(&transaction, actor, events::item_subject(id), &event).await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Chuyển item `id` vào thùng rác. Cần `media.write`. Item vẫn đọc được với
    /// `media.read`, link công khai thôi chạy; khôi phục được trong
    /// `AUTOXIDE_MEDIA_TRASH_DAYS` ngày.
    pub(crate) async fn trash(&self, principal: &Principal, id: Uuid) -> Result<Item, Error> {
        principal.require(&MEDIA_WRITE)?;
        self.set_trashed(principal, id, true).await
    }

    /// Lấy item `id` ra khỏi thùng rác. Cần `media.write`.
    pub(crate) async fn restore(&self, principal: &Principal, id: Uuid) -> Result<Item, Error> {
        principal.require(&MEDIA_WRITE)?;
        self.set_trashed(principal, id, false).await
    }

    async fn set_trashed(
        &self,
        principal: &Principal,
        id: Uuid,
        trash: bool,
    ) -> Result<Item, Error> {
        let transaction = self.database.begin_write().await?;
        let before =
            Entity::find_by_id(id).one(&transaction).await?.ok_or_else(|| item_not_found(id))?;
        let now = now();
        let mut row = before.clone().into_active_model();
        row.trashed_at = Set(trash.then_some(now));
        row.updated_at = Set(now);
        // Điều kiện trong câu `UPDATE`: hai request cùng lúc thì một bên nhận 409.
        let state =
            if trash { Column::TrashedAt.is_null() } else { Column::TrashedAt.is_not_null() };
        let after = match Entity::update(row).validate()?.filter(state).exec(&transaction).await {
            Ok(after) => after,
            Err(sea_orm::DbErr::RecordNotUpdated) => {
                return Err(if trash { in_trash(id) } else { not_in_trash(id) });
            }
            Err(error) => return Err(error.into()),
        };
        let change = Change { before: Some(snapshot(&before)), after: Some(snapshot(&after)) };
        let actor = Actor::from(principal);
        let subject = events::item_subject(id);
        if trash {
            self.outbox.record(&transaction, actor, subject, &events::ItemTrashed(change)).await?;
        } else {
            self.outbox.record(&transaction, actor, subject, &events::ItemRestored(change)).await?;
        }
        transaction.commit().await?;
        tracing::info!(media_item_id = %id, trashed = trash, "media item trash state changed");
        Ok(after)
    }

    /// Xoá hẳn item `id` đang trong thùng rác. Cần `media.purge`.
    pub(crate) async fn purge(&self, principal: &Principal, id: Uuid) -> Result<(), Error> {
        principal.require(&MEDIA_PURGE)?;
        let item = Entity::find_by_id(id)
            .one(self.database.read())
            .await?
            .ok_or_else(|| item_not_found(id))?;
        if item.trashed_at.is_none() {
            return Err(not_in_trash(id));
        }
        if !self.purge_item(Actor::from(principal), &item).await? {
            return Err(item_not_found(id));
        }
        Ok(())
    }

    /// Xoá dòng (kèm event) rồi nội dung của `item` nếu nó còn trong thùng rác. `false`:
    /// không còn gì để xoá (một request khác vừa xoá, hay vừa khôi phục).
    pub(crate) async fn purge_item(&self, actor: Actor, item: &Item) -> Result<bool, Error> {
        let transaction = self.database.begin_write().await?;
        let deleted = Entity::delete_many()
            .filter(Column::Id.eq(item.id))
            .filter(Column::TrashedAt.is_not_null())
            .exec(&transaction)
            .await?;
        if deleted.rows_affected == 0 {
            transaction.rollback().await?;
            return Ok(false);
        }
        crate::entity::item_tag::Entity::delete_many()
            .filter(crate::entity::item_tag::Column::ItemId.eq(item.id))
            .exec(&transaction)
            .await?;
        crate::images::alts::delete_alts(&transaction, item.id).await?;
        let copies = crate::entity::variant::Entity::find()
            .filter(crate::entity::variant::Column::ItemId.eq(item.id))
            .all(&transaction)
            .await?;
        crate::entity::variant::Entity::delete_many()
            .filter(crate::entity::variant::Column::ItemId.eq(item.id))
            .exec(&transaction)
            .await?;
        let event = events::ItemPurged(Change { before: Some(snapshot(item)), after: None });
        self.outbox.record(&transaction, actor, events::item_subject(item.id), &event).await?;
        transaction.commit().await?;
        let keys =
            std::iter::once(item.storage_key()).chain(copies.iter().map(|copy| copy.storage_key()));
        for key in keys {
            if let Err(error) = self.storage.delete(&key).await {
                // Job dọn mồ côi sẽ xoá sau.
                tracing::warn!(media_item_id = %item.id, error = %error, "media item purged but its content remains");
            }
        }
        tracing::info!(media_item_id = %item.id, "media item purged");
        metrics::counter!("media_items_purged_total").increment(1);
        Ok(true)
    }

    /// Item có cùng nội dung (SHA-256) với `item`, ngoài thùng rác, trừ chính nó.
    pub(crate) async fn duplicates(&self, item: &Item) -> Result<Vec<Item>, Error> {
        Ok(Entity::find()
            .filter(Column::Sha256.eq(item.sha256.as_str()))
            .filter(Column::Id.ne(item.id))
            .filter(Column::TrashedAt.is_null())
            .order_by_asc(Column::Id)
            .limit(DUPLICATES_SHOWN)
            .all(self.database.write())
            .await?)
    }

    /// Lý do câu `UPDATE` có điều kiện không đổi dòng nào: item không còn, đang trong
    /// thùng rác, hay đã bị người khác sửa.
    pub(crate) async fn why_not_changed<C: ConnectionTrait>(
        &self,
        id: Uuid,
        connection: &C,
    ) -> Error {
        match Entity::find_by_id(id).one(connection).await {
            Ok(None) => item_not_found(id),
            Ok(Some(item)) if item.trashed_at.is_some() => in_trash(id),
            Ok(Some(_)) => autoxide_kernel::version::changed(),
            Err(error) => error.into(),
        }
    }

    /// Thêm lỗi `field` nếu không có thư mục `id`.
    pub(crate) async fn check_folder_exists(
        &self,
        id: Uuid,
        field: &str,
        problems: &mut Vec<FieldError>,
    ) -> Result<(), Error> {
        if folder::Entity::find_by_id(id).one(self.database.read()).await?.is_none() {
            problems.push(FieldError::new(field, "not_found", "no media folder has this id"));
        }
        Ok(())
    }
}

/// Thông tin item của một người dùng đã tải lên, cho bản xuất dữ liệu cá nhân.
#[async_trait::async_trait]
impl PersonalData for MediaService {
    async fn export(&self, user_id: Uuid) -> Result<Value, Error> {
        let items = Entity::find()
            .filter(Column::UploadedBy.eq(user_id))
            .order_by_asc(Column::Id)
            .all(self.database.read())
            .await?;
        Ok(Value::Array(
            items
                .iter()
                .map(|item| {
                    json!({
                        "id": item.id,
                        "name": item.name,
                        "content_type": item.content_type,
                        "size": item.size,
                        "created_at": item.created_at,
                    })
                })
                .collect(),
        ))
    }
}

/// Tài khoản bị xoá: item người đó tải lên KHÔNG bị xoá — thư viện là của chung, không
/// của người tải (D-086) — chỉ bỏ liên kết tới người đó (`uploaded_by` thành `NULL`).
#[async_trait::async_trait]
impl EventHandler for MediaService {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let Some(user_id) = deleted_user(event) else {
            return Ok(());
        };
        let updated = Entity::update_many()
            .col_expr(Column::UploadedBy, Expr::value(Option::<Uuid>::None))
            .filter(Column::UploadedBy.eq(user_id))
            .exec(self.database.write())
            .await?;
        tracing::info!(user_id = %user_id, items = updated.rows_affected, "media items of a deleted user unlinked");
        Ok(())
    }
}

/// Khoá dòng thư mục `folder` tới hết `transaction` (`rows::lock_one`, D-181), nên lệnh xoá thư mục chạy cùng lúc phải chờ rồi thấy item / thư mục con
/// mới (D-086). `false` nếu thư mục không còn. Thư mục gốc luôn có.
pub(crate) async fn touch_folder<C: ConnectionTrait>(
    transaction: &C,
    folder: Uuid,
) -> Result<bool, Error> {
    if folder == crate::entity::ROOT {
        return Ok(true);
    }
    Ok(rows::lock_one(transaction, folder::Entity::find_by_id(folder)).await?.is_some())
}

/// Kiểm tên (bỏ khoảng trắng hai đầu; 1 tới 255 ký tự; không `/`, `\` hay ký tự điều
/// khiển), thêm lỗi vào `problems`. Trả tên đã chuẩn hoá.
pub(crate) fn check_name(name: &str, problems: &mut Vec<FieldError>) -> String {
    let name = name.trim();
    let length = name.chars().count();
    let valid = (1..=NAME_MAX_LENGTH).contains(&length)
        && !name.chars().any(|c| c == '/' || c == '\\' || c.is_control());
    if !valid {
        problems.push(FieldError::new(
            "name",
            "invalid",
            format!(
                "must be 1 to {NAME_MAX_LENGTH} characters, without `/`, `\\` or control characters"
            ),
        ));
    }
    name.to_owned()
}

fn check_length(field: &str, value: Option<&str>, max: usize, problems: &mut Vec<FieldError>) {
    if value.is_some_and(|value| value.trim().chars().count() > max) {
        problems.push(FieldError::new(
            field,
            "length",
            format!("must be at most {max} characters long"),
        ));
    }
}

/// Đường dẫn tải nội dung của `item`, kèm phiên bản: đổi khi nội dung đổi, nên trình
/// duyệt và CDN giữ được lâu (D-086).
pub(crate) fn content_url(item: &Item) -> String {
    format!("/api/v1/media/{}/content?v={}", item.id, item.version_id)
}

/// `snapshot` kèm danh sách nhãn.
pub(crate) fn with_tags(mut snapshot: Value, tags: &[String]) -> Value {
    if let Some(object) = snapshot.as_object_mut() {
        object.insert("tags".to_owned(), json!(tags));
    }
    snapshot
}

/// Những gì audit cần biết về một item.
pub(crate) fn snapshot(item: &Item) -> Value {
    json!({
        "name": item.name,
        "folder_id": api_folder(item.folder_id),
        "content_type": item.content_type,
        "size": item.size,
        "sha256": item.sha256,
        "width": item.width,
        "height": item.height,
        "alt": item.alt,
        "title": item.title,
        "caption": item.caption,
        "visibility": item.visibility,
        "version_id": item.version_id,
        "trashed_at": item.trashed_at,
    })
}

pub(crate) fn item_not_found(id: Uuid) -> Error {
    Error::not_found("media.item_not_found", "Media item `{id}` does not exist.")
        .with_param("id", id)
}

pub(crate) fn in_trash(id: Uuid) -> Error {
    Error::conflict(
        "media.item_in_trash",
        "Media item `{id}` is in the trash. Restore it first with `POST /api/v1/media/{id}/restore`.",
    )
    .with_param("id", id)
}

fn not_in_trash(id: Uuid) -> Error {
    Error::conflict(
        "media.item_not_in_trash",
        "Media item `{id}` is not in the trash. Move it there first with `DELETE /api/v1/media/{id}`.",
    )
    .with_param("id", id)
}

/// Thư mục đích vừa bị xoá giữa lúc kiểm và lúc ghi.
pub(crate) fn folder_gone(field: &str) -> Error {
    Error::validation(vec![FieldError::new(field, "not_found", "no media folder has this id")])
}

/// Bây giờ, cắt tới micro giây để hai database trả cùng giá trị.
pub(crate) fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

/// Lỗi khi nội dung lớn hơn giới hạn.
pub(crate) fn too_large(max: u64) -> Error {
    Error::new(
        ErrorKind::PayloadTooLarge,
        "media.too_large",
        "The content is larger than {max} bytes, the most this server accepts for a media item.",
    )
    .with_param("max", max)
}

#[async_trait::async_trait]
impl crate::api::Media for MediaService {
    async fn find_many_in(
        &self,
        ids: &[Uuid],
        locale: &str,
    ) -> Result<Vec<crate::api::MediaRef>, Error> {
        let mut found = self.find_many(ids).await?;
        // Ngôn ngữ không được khai thì không có bản dịch nào: giữ `alt` mặc định.
        let Ok(locale) = self.alt_locale(locale) else {
            return Ok(found);
        };
        let ids: Vec<Uuid> = found.iter().map(|item| item.id).collect();
        let mut translated =
            crate::images::alts::alts_in(self.database.read(), &ids, &locale).await?;
        for item in &mut found {
            if let Some(alt) = translated.remove(&item.id) {
                item.alt = alt;
            }
        }
        Ok(found)
    }

    async fn find_many(&self, ids: &[Uuid]) -> Result<Vec<crate::api::MediaRef>, Error> {
        use crate::api::{MAX_IDS, MediaRef, MediaVisibility};

        if ids.len() > MAX_IDS {
            return Err(Error::internal(format!(
                "Media::find_many takes at most {MAX_IDS} ids, got {}",
                ids.len()
            )));
        }
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = Entity::find()
            .filter(Column::Id.is_in(ids.to_vec()))
            .filter(Column::TrashedAt.is_null())
            .all(self.database.read())
            .await?;
        let mut copies = crate::images::variants::variants_of(self.database.read(), &rows).await?;
        let mut found: std::collections::HashMap<Uuid, Item> =
            rows.into_iter().map(|item| (item.id, item)).collect();
        Ok(ids
            .iter()
            .filter_map(|id| found.remove(id))
            .map(|item| {
                let url = content_url(&item);
                let variants =
                    crate::images::variants::pair(copies.remove(&item.id).unwrap_or_default())
                        .into_iter()
                        .map(|copy| crate::api::MediaVariant {
                            url: format!("{url}&w={}", copy.main.width),
                            webp_url: copy
                                .webp
                                .map(|webp| format!("{url}&w={}&format=webp", webp.width)),
                            width: copy.main.width,
                            height: copy.main.height,
                        })
                        .collect();
                MediaRef {
                    id: item.id,
                    visibility: match Visibility::of(&item) {
                        Visibility::Public => MediaVisibility::Public,
                        Visibility::Private => MediaVisibility::Private,
                    },
                    url,
                    variants,
                    content_type: item.content_type,
                    width: item.width,
                    height: item.height,
                    alt: item.alt,
                }
            })
            .collect())
    }
}
