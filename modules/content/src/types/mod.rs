//! Loại nội dung tự định nghĩa (bước 67, D-235, D-20261122-content-types): khách tạo loại (đội ngũ,
//! dự án, chi nhánh…) mà không sửa code. Trường riêng của loại là trường tự định nghĩa của kernel với
//! đích `content.type.<key>` (không có bộ trường thứ hai). Mục của loại là `content_entries` với
//! `kind = custom` và `type_id` — dùng nguyên nháp, lịch sử, duyệt, hẹn giờ, bản dịch, SEO, chuyển
//! hướng của `content`. Loại có trang thì mục công khai ở `/<path_prefix>/<slug>`.

pub(crate) mod public;

use std::collections::BTreeMap;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, root_of};
use autoxide_kernel::time::Timestamp;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, PaginatorTrait, QueryFilter, Set,
    SqlErr,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::{content_type, entry};
use crate::entries::Entry;
use crate::events::{self, Change};
use crate::permissions::{CONTENT_READ, CONTENT_TYPES};
use crate::service::{ContentService, now};

/// Đích trường tự định nghĩa của loại `key`.
pub(crate) fn field_target(key: &str) -> String {
    format!("{}.{key}", TARGET)
}
/// Đích cha (khai `with_subtypes`).
pub(crate) const TARGET: &str = "content.type";
pub(crate) const BODIES: [&str; 3] = ["none", "rich", "layout"];
pub(crate) const ICONS: [&str; 12] = [
    "people",
    "briefcase",
    "map_pin",
    "star",
    "question",
    "calendar",
    "image",
    "document",
    "tag",
    "building",
    "gift",
    "chat",
];
/// Số loại tối đa mỗi khách (trần máy chủ).
pub(crate) const TYPES_MAX: u64 = 50;
/// Đường dẫn dành riêng của `content` (bài viết, đọc công khai…), không làm tiền tố của loại.
const RESERVED_PREFIXES: [&str; 3] = ["api", "public", "preview"];

pub(crate) type ContentType = content_type::Model;

/// Dữ liệu tạo / sửa một loại; `None` giữ nguyên (khi tạo: mặc định).
#[derive(Debug, Clone, Default)]
pub(crate) struct TypeInput {
    pub key: Option<String>,
    pub labels: Option<Value>,
    pub icon: Option<String>,
    pub routable: Option<bool>,
    pub path_prefix: Option<String>,
    pub body: Option<String>,
    pub has_seo: Option<bool>,
    pub listable: Option<bool>,
    pub active: Option<bool>,
    pub org_id: Option<Uuid>,
}

pub(crate) fn type_not_found(key: &str) -> Error {
    Error::not_found("content.type_not_found", "There is no content type `{key}`.")
        .with_param("key", key)
}

fn prefix_taken(prefix: &str) -> Error {
    Error::conflict(
        "content.path_prefix_taken",
        "The address `/{prefix}` is used by another content type of this website.",
    )
    .with_param("prefix", prefix)
}

impl ContentService {
    /// Một trang loại nội dung. Cần `content.read`.
    pub(crate) async fn list_types(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<ContentType>, Error> {
        principal.require(&CONTENT_READ)?;
        let select = content_type::Entity::find().scoped(tenant);
        fetch_page(select, content_type::Column::Id, |row| row.id, page, tenant.database().read())
            .await
    }

    /// Loại `key`. Cần `content.read`.
    pub(crate) async fn content_type(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        key: &str,
    ) -> Result<ContentType, Error> {
        principal.require(&CONTENT_READ)?;
        self.find_type(tenant, key).await
    }

    pub(crate) async fn find_type(
        &self,
        tenant: &TenantContext,
        key: &str,
    ) -> Result<ContentType, Error> {
        content_type::Entity::find()
            .scoped(tenant)
            .filter(content_type::Column::Key.eq(key))
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| type_not_found(key))
    }

    /// Loại của mục `entry` (mục `kind = custom`), đọc trên `connection`.
    pub(crate) async fn type_of<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        entry: &Entry,
    ) -> Result<Option<ContentType>, Error> {
        match entry.type_id {
            Some(id) => {
                Ok(content_type::Entity::find_by_id(id).scoped(tenant).one(connection).await?)
            }
            None => Ok(None),
        }
    }

    /// Đích trường tự định nghĩa của mục `entry`: `content.entry`, hay `content.type.<key>` cho mục
    /// của loại tự định nghĩa.
    pub(crate) async fn entry_field_target(
        &self,
        tenant: &TenantContext,
        entry: &Entry,
    ) -> Result<String, Error> {
        Ok(match self.type_of(tenant.database().read(), tenant, entry).await? {
            Some(row) => field_target(&row.key),
            None => crate::resource::ENTRY.to_owned(),
        })
    }

    /// Tạo loại. Cần `content.types.manage` ở cả khách; tối đa [`TYPES_MAX`] loại.
    pub(crate) async fn create_type(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: TypeInput,
    ) -> Result<ContentType, Error> {
        principal.require(&CONTENT_TYPES)?;
        let org = input.org_id.map_or(root_of(tenant.id()), OrgId::from_uuid);
        let at = now();
        let mut row = ContentType {
            id: Uuid::now_v7(),
            tenant_id: tenant.id().as_uuid(),
            org_id: org.as_uuid(),
            key: String::new(),
            labels: "{}".to_owned(),
            icon: "document".to_owned(),
            routable: false,
            path_prefix: None,
            body: "rich".to_owned(),
            has_seo: true,
            listable: true,
            active: true,
            created_by: Some(principal.user_id()),
            created_at: at,
            updated_at: at,
        };
        apply(&mut row, input, true)?;
        let transaction = tenant.database().begin_write().await?;
        principal.require_write_at(&transaction, &CONTENT_TYPES, OrgTarget::new(org, None)).await?;
        let count = content_type::Entity::find().scoped(tenant).count(&transaction).await?;
        if count >= TYPES_MAX {
            return Err(Error::conflict(
                "content.types_limit",
                "This tenant already has {max} content types; delete one first.",
            )
            .with_param("max", TYPES_MAX));
        }
        let active: content_type::ActiveModel = row.clone().into();
        let saved = active.insert(&transaction).await.map_err(|error| unique(error, &row))?;
        self.record_type(&transaction, tenant, principal, None, Some(&saved)).await?;
        transaction.commit().await?;
        Ok(saved)
    }

    /// Sửa loại `key` (chỉ trường gửi lên). Đổi `key` khi loại đã có mục → 409
    /// `content.type_key_locked`. Tắt loại: mục ngừng công khai (không mất dữ liệu).
    pub(crate) async fn update_type(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        key: &str,
        input: TypeInput,
        expected: Option<Timestamp>,
    ) -> Result<ContentType, Error> {
        principal.require(&CONTENT_TYPES)?;
        let transaction = tenant.database().begin_write().await?;
        let before = content_type::Entity::find()
            .scoped(tenant)
            .filter(content_type::Column::Key.eq(key))
            .one(&transaction)
            .await?
            .ok_or_else(|| type_not_found(key))?;
        principal
            .require_write_at(
                &transaction,
                &CONTENT_TYPES,
                OrgTarget::new(OrgId::from_uuid(before.org_id), None),
            )
            .await?;
        if expected.is_some_and(|expected| expected != before.updated_at) {
            return Err(autoxide_kernel::version::changed());
        }
        let mut row = before.clone();
        apply(&mut row, input, false)?;
        if row.key != before.key && entries_of(&transaction, tenant, before.id).await? > 0 {
            return Err(Error::conflict(
                "content.type_key_locked",
                "Content type `{key}` already has entries; its key cannot change.",
            )
            .with_param("key", &before.key));
        }
        row.updated_at = now();
        let mut active: content_type::ActiveModel = before.clone().into();
        active.key = Set(row.key.clone());
        active.labels = Set(row.labels.clone());
        active.icon = Set(row.icon.clone());
        active.routable = Set(row.routable);
        active.path_prefix = Set(row.path_prefix.clone());
        active.body = Set(row.body.clone());
        active.has_seo = Set(row.has_seo);
        active.listable = Set(row.listable);
        active.active = Set(row.active);
        active.updated_at = Set(row.updated_at);
        let saved = active.update(&transaction).await.map_err(|error| unique(error, &row))?;
        self.record_type(&transaction, tenant, principal, Some(&before), Some(&saved)).await?;
        transaction.commit().await?;
        Ok(saved)
    }

    /// Xoá loại `key`; còn mục → 409 `content.type_in_use` kèm số mục.
    pub(crate) async fn delete_type(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        key: &str,
    ) -> Result<(), Error> {
        principal.require(&CONTENT_TYPES)?;
        let transaction = tenant.database().begin_write().await?;
        let before = content_type::Entity::find()
            .scoped(tenant)
            .filter(content_type::Column::Key.eq(key))
            .one(&transaction)
            .await?
            .ok_or_else(|| type_not_found(key))?;
        principal
            .require_write_at(
                &transaction,
                &CONTENT_TYPES,
                OrgTarget::new(OrgId::from_uuid(before.org_id), None),
            )
            .await?;
        let count = entries_of(&transaction, tenant, before.id).await?;
        if count > 0 {
            return Err(Error::conflict(
                "content.type_in_use",
                "Content type `{key}` still has {count} entries; delete or move them first.",
            )
            .with_param("key", key)
            .with_param("count", count));
        }
        content_type::Entity::delete_many()
            .scoped(tenant)
            .filter(content_type::Column::Id.eq(before.id))
            .exec(&transaction)
            .await?;
        self.record_type(&transaction, tenant, principal, Some(&before), None).await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn record_type<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        principal: &Principal,
        before: Option<&ContentType>,
        after: Option<&ContentType>,
    ) -> Result<(), Error> {
        let view = |row: &ContentType| json!({"id": row.id, "key": row.key, "active": row.active, "routable": row.routable, "path_prefix": row.path_prefix});
        let id = after.or(before).map_or(Uuid::nil(), |row| row.id);
        let event = events::TypeChanged(Change {
            before: before.map(view),
            after: after.map(view),
            published: None,
        });
        self.outbox
            .record(connection, tenant, Actor::from(principal), events::subject("type", id), &event)
            .await?;
        Ok(())
    }
}

async fn entries_of<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    type_id: Uuid,
) -> Result<u64, Error> {
    Ok(entry::Entity::find()
        .scoped(tenant)
        .filter(entry::Column::TypeId.eq(type_id))
        .count(connection)
        .await?)
}

fn unique(error: sea_orm::DbErr, row: &ContentType) -> Error {
    match error.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(message)) if message.contains("prefix") => {
            prefix_taken(row.path_prefix.as_deref().unwrap_or_default())
        }
        Some(SqlErr::UniqueConstraintViolation(_)) => Error::conflict(
            "content.type_key_taken",
            "A content type with the key `{key}` already exists.",
        )
        .with_param("key", &row.key),
        _ => error.into(),
    }
}

/// Áp `input` lên `row` và kiểm, mọi lỗi một lần.
fn apply(row: &mut ContentType, input: TypeInput, creating: bool) -> Result<(), Error> {
    let mut problems = Vec::new();
    match input.key {
        Some(key) if is_key(&key) => row.key = key,
        Some(_) => problems.push(FieldError::new(
            "key",
            "invalid",
            "must be 1 to 64 lower-case letters, digits or `_`, starting with a letter",
        )),
        None if creating => problems.push(FieldError::new("key", "required", "is required")),
        None => {}
    }
    if let Some(labels) = input.labels {
        match check_labels(&labels) {
            Ok(text) => row.labels = text,
            Err(error) => problems.push(error),
        }
    } else if creating {
        problems.push(FieldError::new("labels", "required", "is required"));
    }
    if let Some(icon) = input.icon {
        if ICONS.contains(&icon.as_str()) {
            row.icon = icon;
        } else {
            problems.push(
                FieldError::new("icon", "invalid", "must be one of {allowed}")
                    .with_param("allowed", ICONS.join(", ")),
            );
        }
    }
    if let Some(body) = input.body {
        if BODIES.contains(&body.as_str()) {
            row.body = body;
        } else {
            problems.push(
                FieldError::new("body", "invalid", "must be one of {allowed}")
                    .with_param("allowed", BODIES.join(", ")),
            );
        }
    }
    if let Some(routable) = input.routable {
        row.routable = routable;
    }
    if let Some(prefix) = input.path_prefix {
        let prefix = prefix.trim().trim_matches('/').to_owned();
        if autoxide_kernel::slug::is_valid(&prefix) && !RESERVED_PREFIXES.contains(&prefix.as_str())
        {
            row.path_prefix = Some(prefix);
        } else {
            problems.push(FieldError::new("path_prefix", "invalid", "must be a lower-case address segment such as `du-an`, not `api`, `public` or `preview`"));
        }
    }
    if row.routable && row.path_prefix.is_none() {
        problems.push(FieldError::new(
            "path_prefix",
            "required",
            "a content type with pages needs an address prefix",
        ));
    }
    if !row.routable {
        row.path_prefix = None;
    }
    for (value, field) in [
        (input.has_seo, &mut row.has_seo),
        (input.listable, &mut row.listable),
        (input.active, &mut row.active),
    ] {
        if let Some(value) = value {
            *field = value;
        }
    }
    if problems.is_empty() { Ok(()) } else { Err(Error::validation(problems)) }
}

fn is_key(key: &str) -> bool {
    (1..=64).contains(&key.len())
        && key.starts_with(|c: char| c.is_ascii_lowercase())
        && key.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Nhãn: `{"one": {lang: chữ}, "many": {lang: chữ}}`, mỗi nhãn 1–80 ký tự, có ít nhất một ngôn ngữ.
fn check_labels(labels: &Value) -> Result<String, FieldError> {
    let invalid = || {
        FieldError::new(
            "labels",
            "invalid",
            "must be {\"one\": {language: text}, \"many\": {language: text}} with texts of 1 to 80 characters",
        )
    };
    let object = labels.as_object().ok_or_else(invalid)?;
    let mut clean: BTreeMap<&str, BTreeMap<String, String>> = BTreeMap::new();
    for part in ["one", "many"] {
        let texts = object.get(part).and_then(Value::as_object).ok_or_else(invalid)?;
        let mut kept = BTreeMap::new();
        for (language, text) in texts {
            let text = text
                .as_str()
                .map(str::trim)
                .filter(|text| (1..=80).contains(&text.chars().count()))
                .ok_or_else(invalid)?;
            kept.insert(language.clone(), text.to_owned());
        }
        if kept.is_empty() {
            return Err(invalid());
        }
        clean.insert(part, kept);
    }
    if object.len() != 2 {
        return Err(invalid());
    }
    serde_json::to_string(&clean).map_err(|_| invalid())
}
