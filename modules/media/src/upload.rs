//! Nhận nội dung: tải lên item mới và thay nội dung của item có sẵn.
//!
//! Nội dung đi theo luồng thẳng tới nơi lưu, không nằm trọn trong bộ nhớ. Trên đường đi
//! nó được đếm (vượt giới hạn thì dừng, 413), băm SHA-256, và giữ lại phần đầu
//! (`sniff::HEAD_BYTES`) để nhận loại thật và đọc kích thước ảnh. Loại khai sai thì dừng
//! ngay khi đủ byte để nhận ra, không chờ hết file (D-089).

use std::sync::{Arc, Mutex};

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::storage::ByteStream;
use chrono::{DateTime, Utc};
use futures_util::StreamExt as _;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, Set};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::config::{Limits, normalize_type};
use crate::entity::item::{self, Column, Entity, storage_key};
use crate::entity::stored_folder;
use crate::events::{self, Change};
use crate::permissions::MEDIA_WRITE;
use crate::service::{
    Item, MediaService, Visibility, check_name, folder_gone, in_trash, item_not_found, now,
    snapshot, too_large, touch_folder,
};
use crate::sniff;

/// Một lần tải lên.
#[derive(Debug, Clone, Default)]
pub(crate) struct NewUpload {
    pub name: String,
    /// `None`: thư mục gốc.
    pub folder_id: Option<Uuid>,
    /// Header `Content-Type` như client gửi; `None` hay `application/octet-stream`: máy
    /// chủ tự nhận loại.
    pub content_type: Option<String>,
    /// Nhãn gắn ngay khi tải lên.
    pub tags: Vec<String>,
}

/// Kết quả tải lên: item mới, và các item đã có cùng nội dung.
#[derive(Debug, Clone)]
pub(crate) struct Uploaded {
    pub item: Item,
    pub duplicates: Vec<Item>,
}

/// Nội dung đã nhận xong và đã kiểm.
struct Received {
    content_type: String,
    size: i64,
    sha256: String,
    width: Option<i32>,
    height: Option<i32>,
}

/// Trạng thái của luồng đang nhận, chung giữa luồng và bên đọc kết quả.
#[derive(Default)]
struct Intake {
    hasher: Sha256,
    size: u64,
    head: Vec<u8>,
    /// Loại đã kiểm xong (khi đã đủ byte).
    verdict: Option<String>,
}

impl MediaService {
    /// Tải lên item mới. Cần `media.write`. Tên và thư mục kiểm TRƯỚC khi đọc nội dung,
    /// mọi lỗi báo một lần.
    pub(crate) async fn upload(
        &self,
        principal: &Principal,
        input: NewUpload,
        content: ByteStream,
    ) -> Result<Uploaded, Error> {
        principal.require(&MEDIA_WRITE)?;
        let mut problems = Vec::new();
        let name = check_name(&input.name, &mut problems);
        if let Some(folder) = input.folder_id {
            self.check_folder_exists(folder, "folder_id", &mut problems).await?;
        }
        let tags = crate::tags::check_tags(&input.tags, "tags", &mut problems);
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let declared = declared_type(input.content_type.as_deref(), &self.config.limits)?;

        let (id, version_id) = (Uuid::now_v7(), Uuid::now_v7());
        let key = storage_key(id, version_id);
        let received = self.receive(&key, declared, content).await?;
        let saved = self
            .save_new(principal, (id, version_id), name, input.folder_id, &tags, received)
            .await;
        if saved.is_err() {
            self.discard(&key).await;
        }
        let item = saved?;
        let duplicates = self.duplicates(&item).await?;
        tracing::info!(media_item_id = %item.id, size = item.size, "media item uploaded");
        metrics::counter!("media_items_uploaded_total").increment(1);
        Ok(Uploaded { item, duplicates })
    }

    /// Thay nội dung của item `id`, GIỮ ID (link và chỗ dùng item vẫn đúng); `url` và
    /// `ETag` đổi. Cần `media.write`. `expected`: phiên bản người gọi đã đọc (`If-Match`).
    ///
    /// Thứ tự: ghi nội dung mới dưới `version_id` mới; cập nhật dòng + event trong một
    /// transaction (điều kiện phiên bản ngay trong câu `UPDATE`); rồi xoá nội dung cũ.
    pub(crate) async fn replace(
        &self,
        principal: &Principal,
        id: Uuid,
        content_type: Option<&str>,
        content: ByteStream,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Item, Error> {
        principal.require(&MEDIA_WRITE)?;
        let current = Entity::find_by_id(id)
            .one(self.database.read())
            .await?
            .ok_or_else(|| item_not_found(id))?;
        if current.trashed_at.is_some() {
            return Err(in_trash(id));
        }
        if expected.is_some_and(|expected| expected != current.updated_at) {
            // Biết trước là sẽ 412: khỏi nhận cả file.
            return Err(autoxide_kernel::version::changed());
        }
        let declared = declared_type(content_type, &self.config.limits)?;
        let version_id = Uuid::now_v7();
        let key = storage_key(id, version_id);
        let received = self.receive(&key, declared, content).await?;

        let replaced = self.save_replacement(principal, id, version_id, received, expected).await;
        let (before, after, old_copies) = match replaced {
            Ok(changed) => changed,
            Err(error) => {
                self.discard(&key).await;
                return Err(error);
            }
        };
        // Nội dung cũ (và ảnh thu nhỏ của nó) không còn ai trỏ tới; xoá hỏng thì job dọn mồ
        // côi xoá sau.
        self.discard(&before.storage_key()).await;
        for copy in &old_copies {
            self.discard(&copy.storage_key()).await;
        }
        tracing::info!(media_item_id = %id, size = after.size, "media item content replaced");
        Ok(after)
    }

    async fn save_new(
        &self,
        principal: &Principal,
        (id, version_id): (Uuid, Uuid),
        name: String,
        folder: Option<Uuid>,
        tags: &[String],
        received: Received,
    ) -> Result<Item, Error> {
        let now = now();
        let folder = stored_folder(folder);
        let transaction = self.database.begin_write().await?;
        if !touch_folder(&transaction, folder).await? {
            // Thư mục vừa bị xoá giữa lúc kiểm và lúc ghi.
            return Err(folder_gone("folder_id"));
        }
        let item = item::ActiveModel {
            id: Set(id),
            folder_id: Set(folder),
            name_key: Set(autoxide_kernel::search::normalize(&name)),
            name: Set(name),
            content_type: Set(received.content_type),
            size: Set(received.size),
            sha256: Set(received.sha256),
            width: Set(received.width),
            height: Set(received.height),
            alt: Set(String::new()),
            title: Set(String::new()),
            caption: Set(String::new()),
            visibility: Set(Visibility::Private.as_str().to_owned()),
            version_id: Set(version_id),
            uploaded_by: Set(Some(principal.user_id())),
            created_at: Set(now),
            updated_at: Set(now),
            trashed_at: Set(None),
        }
        .insert(&transaction)
        .await?;
        let tags = if tags.is_empty() {
            Vec::new()
        } else {
            crate::tags::set_item_tags(&transaction, id, tags).await?
        };
        self.enqueue_variants(&transaction, &item).await?;
        let after = crate::service::with_tags(snapshot(&item), &tags);
        let event = events::ItemUploaded(Change { before: None, after: Some(after) });
        let actor = Actor::from(principal);
        self.outbox.record(&transaction, actor, events::item_subject(id), &event).await?;
        transaction.commit().await?;
        Ok(item)
    }

    async fn save_replacement(
        &self,
        principal: &Principal,
        id: Uuid,
        version_id: Uuid,
        received: Received,
        expected: Option<DateTime<Utc>>,
    ) -> Result<(Item, Item, Vec<crate::variants::Variant>), Error> {
        let transaction = self.database.begin_write().await?;
        let before =
            Entity::find_by_id(id).one(&transaction).await?.ok_or_else(|| item_not_found(id))?;
        let mut row = before.clone().into_active_model();
        row.content_type = Set(received.content_type);
        row.size = Set(received.size);
        row.sha256 = Set(received.sha256);
        row.width = Set(received.width);
        row.height = Set(received.height);
        row.version_id = Set(version_id);
        row.updated_at = Set(now());
        let mut update = Entity::update(row).validate()?.filter(Column::TrashedAt.is_null());
        // Điều kiện "phiên bản đã đọc" (`If-Match`), hay ít nhất "chưa ai thay nội dung từ
        // lúc đọc ở trên" — hai lệnh thay cùng lúc thì đúng một lệnh thắng.
        update = match expected {
            Some(expected) => update.filter(Column::UpdatedAt.eq(expected)),
            None => update.filter(Column::VersionId.eq(before.version_id)),
        };
        let after = match update.exec(&transaction).await {
            Ok(after) => after,
            Err(sea_orm::DbErr::RecordNotUpdated) => {
                return Err(self.why_not_changed(id, &transaction).await);
            }
            Err(error) => return Err(error.into()),
        };
        use crate::entity::variant;
        let old_copies = variant::Entity::find()
            .filter(variant::Column::ItemId.eq(id))
            .all(&transaction)
            .await?;
        variant::Entity::delete_many()
            .filter(variant::Column::ItemId.eq(id))
            .exec(&transaction)
            .await?;
        self.enqueue_variants(&transaction, &after).await?;
        let change = Change { before: Some(snapshot(&before)), after: Some(snapshot(&after)) };
        let event = events::ItemContentReplaced(change);
        let actor = Actor::from(principal);
        self.outbox.record(&transaction, actor, events::item_subject(id), &event).await?;
        transaction.commit().await?;
        Ok((before, after, old_copies))
    }

    /// Ghi `content` vào `key`, kiểm trên đường đi; lỗi thì không để lại gì ở nơi lưu.
    async fn receive(
        &self,
        key: &str,
        declared: Option<String>,
        content: ByteStream,
    ) -> Result<Received, Error> {
        let limits = self.config.limits.clone();
        let intake = Arc::new(Mutex::new(Intake::default()));
        let state = intake.clone();
        let (max, expected_type) = (limits.max_bytes, declared.clone());
        let checked: ByteStream = content
            .map(move |chunk| {
                let chunk = chunk?;
                let mut state = state.lock().map_err(|_| Error::internal("media intake lock"))?;
                state.size += chunk.len() as u64;
                if state.size > max {
                    return Err(too_large(max));
                }
                state.hasher.update(&chunk);
                let room = sniff::HEAD_BYTES.saturating_sub(state.head.len());
                state.head.extend_from_slice(&chunk[..chunk.len().min(room)]);
                if state.verdict.is_none() && state.head.len() >= sniff::SNIFF_BYTES {
                    // Đủ byte để nhận loại: khai sai thì dừng ngay, không nhận tiếp.
                    let verdict = verify(expected_type.as_deref(), &state.head, &limits)?;
                    state.verdict = Some(verdict);
                }
                Ok(chunk)
            })
            .boxed();
        self.storage.put(key, checked).await?;

        let finished = {
            let mut state = intake.lock().map_err(|_| Error::internal("media intake lock"))?;
            let state = std::mem::take(&mut *state);
            finish(state, declared.as_deref(), &self.config.limits)
        };
        if finished.is_err() {
            self.discard(key).await;
        }
        finished
    }

    /// Xoá nội dung ở `key`; hỏng thì chỉ ghi log (job dọn mồ côi xoá sau).
    async fn discard(&self, key: &str) {
        if let Err(error) = self.storage.delete(key).await {
            tracing::warn!(key, error = %error, "cannot remove unused media content");
        }
    }
}

/// Kiểm phần còn lại sau khi nhận hết: file nhỏ hơn `SNIFF_BYTES` chưa được kiểm loại,
/// file rỗng, kích thước ảnh.
fn finish(state: Intake, declared: Option<&str>, limits: &Limits) -> Result<Received, Error> {
    if state.size == 0 {
        return Err(Error::validation(vec![FieldError::new(
            "content",
            "empty",
            "the body is empty; send the content of the file",
        )]));
    }
    let content_type = match state.verdict {
        Some(verdict) => verdict,
        None => verify(declared, &state.head, limits)?,
    };
    let (width, height) = if sniff::is_raster_image(&content_type) {
        let (width, height) = sniff::dimensions(&state.head).ok_or_else(|| {
            Error::new(
                ErrorKind::UnsupportedMediaType,
                "media.invalid_image",
                "The image could not be read: it is damaged, or not a `{type}` file.",
            )
            .with_param("type", &content_type)
        })?;
        (Some(width), Some(height))
    } else {
        (None, None)
    };
    Ok(Received {
        content_type,
        size: i64::try_from(state.size).map_err(Error::internal)?,
        sha256: state.hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect(),
        width,
        height,
    })
}

/// Loại client khai (header `Content-Type`), đã chuẩn hoá và đã kiểm là được phép —
/// trước khi đọc nội dung. `None`: không khai (hay `application/octet-stream`), máy chủ
/// tự nhận loại.
fn declared_type(raw: Option<&str>, limits: &Limits) -> Result<Option<String>, Error> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let declared = normalize_type(raw).ok_or_else(|| {
        Error::bad_request(
            "media.invalid_content_type",
            "The `Content-Type` header must be a media type like `image/png`.",
        )
    })?;
    if declared == "application/octet-stream" {
        return Ok(None);
    }
    let declared = sniff::canonical(&declared).to_owned();
    check_allowed(&declared, limits)?;
    Ok(Some(declared))
}

/// Loại cuối cùng của nội dung có phần đầu `head`: loại khai nếu khớp loại thật, hay loại
/// thật nếu không khai.
fn verify(declared: Option<&str>, head: &[u8], limits: &Limits) -> Result<String, Error> {
    let detected = sniff::detect(head);
    match (declared, detected) {
        (None, Some(detected)) => {
            check_allowed(detected, limits)?;
            Ok(detected.to_owned())
        }
        (None, None) => Err(Error::new(
            ErrorKind::UnsupportedMediaType,
            "media.unknown_type",
            "The type of the content could not be recognised. Send its media type in \
             `Content-Type`.",
        )),
        (Some(declared), Some(detected)) if sniff::compatible(declared, detected) => {
            Ok(declared.to_owned())
        }
        (Some(declared), None) if !sniff::has_signature(declared) => Ok(declared.to_owned()),
        (Some(declared), detected) => Err(Error::new(
            ErrorKind::UnsupportedMediaType,
            "media.content_mismatch",
            "The content is not a `{declared}` file (it looks like `{detected}`). Send the \
             real media type in `Content-Type`.",
        )
        .with_param("declared", declared)
        .with_param("detected", detected.unwrap_or("unknown"))),
    }
}

fn check_allowed(content_type: &str, limits: &Limits) -> Result<(), Error> {
    if limits.allows(content_type) {
        return Ok(());
    }
    Err(Error::new(
        ErrorKind::UnsupportedMediaType,
        "media.type_not_allowed",
        "Media of type `{type}` cannot be uploaded here. Allowed types: {allowed}.",
    )
    .with_param("type", content_type)
    .with_param("allowed", limits.allowed_types.join(", ")))
}
