//! Mẫu thông báo khách tự soạn (D-206, D-365): cú pháp của bộ dựng mẫu kernel (`{{ code }}`, `{% if %}`,
//! `{% for %}`), kiểm biến theo schema của loại khi lưu, bản nháp → xuất bản (bản đã xuất bản không
//! đổi, giữ 20 bản gần nhất) → quay lại bản cũ, xem trước bằng dữ liệu mẫu, gửi thử tới chính người
//! gọi. Không có mẫu đang dùng thì tin dùng câu thay thế cũ (`notifications_templates`) rồi mẫu mặc
//! định theo i18n.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::i18n::Language;
use autoxide_kernel::mail::Mail;
use autoxide_kernel::template::{Output, TemplateSchema, Variable};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::api::{Category, Channel, NotificationKind};
use crate::delivery::UNSUBSCRIBE_FOOTER;
use crate::entity::{inbox, template_draft, template_live, template_version};
use crate::events;
use crate::layout::{escape, mail_html, text_to_html};
use crate::permissions::{TEMPLATES_MANAGE, TEMPLATES_READ};
use crate::service::{NotificationsService, now};

/// Số bản đã xuất bản giữ lại cho mỗi (loại, kênh, ngôn ngữ).
const KEEP_VERSIONS: i32 = 20;

/// Mô tả biến suy ra từ chỗ điền `{tên}` của mẫu mặc định.
const DERIVED: &str = "A value the module sends with this kind of notification.";
const LINK: &str = "Page opened from the notification (may be empty).";

pub(crate) fn draft_not_found() -> Error {
    Error::not_found(
        "notifications.template_draft_not_found",
        "There is no draft for this template; save one with `PUT …/draft` first.",
    )
}

pub(crate) fn version_not_found() -> Error {
    Error::not_found(
        "notifications.template_version_not_found",
        "There is no such published version of this template (only the last 20 are kept).",
    )
}

pub(crate) fn not_published() -> Error {
    Error::not_found(
        "notifications.template_not_published",
        "This template has no published version in use; the default text applies.",
    )
}

pub(crate) fn test_send_unsupported() -> Error {
    Error::new(
        ErrorKind::Validation,
        "notifications.test_send_unsupported",
        "A test can only be sent on the `email` channel (to your verified email) or the `in_app` \
         channel (to your inbox).",
    )
}

pub(crate) fn test_address_unverified() -> Error {
    Error::new(
        ErrorKind::Validation,
        "notifications.test_address_unverified",
        "Your email is not verified; a test email only goes to your own verified address.",
    )
}

pub(crate) fn test_send_throttled() -> Error {
    Error::too_many_requests(
        "notifications.test_send_throttled",
        "Too many test sends in the last hour; try again later.",
    )
}

/// Nội dung một mẫu (bản nháp hay bản đã xuất bản).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Content {
    /// Tiêu đề: chủ đề email, tiêu đề đẩy và hộp thư.
    pub subject: String,
    /// Bản chữ thuần: email (phần chữ), hộp thư.
    pub body_text: String,
    /// Bản HTML của email (đặt trong khung MJML); `None` → sinh từ `body_text`.
    pub body_html: Option<String>,
    /// SMS, ZNS, đẩy.
    pub short: String,
    pub zns_template_id: Option<String>,
}

impl From<&template_draft::Model> for Content {
    fn from(row: &template_draft::Model) -> Self {
        Content {
            subject: row.subject.clone(),
            body_text: row.body_text.clone(),
            body_html: row.body_html.clone(),
            short: row.short.clone(),
            zns_template_id: row.zns_template_id.clone(),
        }
    }
}

impl From<&template_version::Model> for Content {
    fn from(row: &template_version::Model) -> Self {
        Content {
            subject: row.subject.clone(),
            body_text: row.body_text.clone(),
            body_html: row.body_html.clone(),
            short: row.short.clone(),
            zns_template_id: row.zns_template_id.clone(),
        }
    }
}

/// Bản nào để xem trước / gửi thử.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    Draft,
    Published,
    Version(i32),
}

/// Kết quả dựng (xem trước).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Preview {
    pub subject: String,
    pub text: String,
    /// Mail HTML hoàn chỉnh (khung MJML) — chỉ kênh `email`.
    pub html: Option<String>,
    pub short: String,
}

/// Danh sách bản đã xuất bản và bản đang dùng.
pub(crate) struct Versions {
    pub items: Vec<template_version::Model>,
    pub live: Option<i32>,
}

/// Tên các chỗ điền `{tên}` (giữ thời gian sống của chuỗi gốc: mẫu mặc định là `&'static str`).
fn placeholders(template: &str) -> Vec<&str> {
    let mut names: Vec<&str> = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        match after.split_once('}') {
            Some((name, _))
                if !name.is_empty()
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') =>
            {
                if !names.contains(&name) {
                    names.push(name);
                }
                rest = &after[name.len() + 1..];
            }
            _ => rest = after,
        }
    }
    names
}

/// Schema biến của loại: khai sẵn, hay suy ra từ mẫu mặc định (cộng `link`).
pub(crate) fn schema_of(kind: &NotificationKind) -> TemplateSchema {
    if !kind.variables.is_empty() {
        return TemplateSchema::new(kind.variables.to_vec());
    }
    let template = kind.template;
    let mut variables: Vec<Variable> = Vec::new();
    for text in [template.title, template.body, template.short] {
        for name in placeholders(text) {
            if !variables.iter().any(|known| known.path == name) {
                variables.push(Variable::optional(name, DERIVED));
            }
        }
    }
    if !variables.iter().any(|known| known.path == "link") {
        variables.push(Variable::optional("link", LINK));
    }
    TemplateSchema::new(variables)
}

/// Dữ liệu mẫu của loại: khai sẵn, hay `"[tên]"` cho mỗi biến (đường dẫn chấm thành object lồng).
pub(crate) fn sample_of(kind: &NotificationKind) -> Value {
    if let Some(Value::Object(map)) = kind.sample.and_then(|raw| serde_json::from_str(raw).ok()) {
        return Value::Object(map);
    }
    let mut root = Map::new();
    for variable in schema_of(kind).variables {
        let parts: Vec<&str> = variable.path.split('.').collect();
        let mut node = &mut root;
        for (index, part) in parts.iter().enumerate() {
            if index + 1 == parts.len() {
                let value = if *part == "link" {
                    json!("https://www.example.com/")
                } else {
                    json!(format!("[{part}]"))
                };
                node.entry((*part).to_owned()).or_insert(value);
            } else {
                let next = node.entry((*part).to_owned()).or_insert_with(|| json!({}));
                if !next.is_object() {
                    *next = json!({});
                }
                let Value::Object(child) = next else { break };
                node = child;
            }
        }
    }
    Value::Object(root)
}

/// Khoá gốc `key` có thuộc schema không (là một biến, hay gốc của một biến đã khai).
fn schema_has_root(schema: &TemplateSchema, key: &str) -> bool {
    schema.variables.iter().any(|variable| {
        variable.path == key
            || variable.path.strip_prefix(key).is_some_and(|rest| rest.starts_with('.'))
    })
}

/// Ngữ cảnh dựng lúc gửi: tham số của tin cộng `link`.
pub(crate) fn send_context(params: &str, link: Option<&str>) -> Value {
    let mut map = match serde_json::from_str::<Value>(params) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    };
    map.insert("link".to_owned(), link.map_or(Value::String(String::new()), |link| json!(link)));
    Value::Object(map)
}

fn key_of(
    tenant: &TenantContext,
    kind: &NotificationKind,
    channel: Channel,
    locale: &str,
) -> (Uuid, String, String, String) {
    (tenant.id().as_uuid(), kind.key().to_owned(), channel.as_str().to_owned(), locale.to_owned())
}

impl NotificationsService {
    /// Kiểm nội dung: độ dài, trường bắt buộc theo kênh, cú pháp và biến theo schema — gom mọi lỗi
    /// báo một lần (D-083).
    fn check_content(
        &self,
        kind: &NotificationKind,
        channel: Channel,
        content: &Content,
    ) -> Result<(), Error> {
        let mut problems = Vec::new();
        // Trường bắt buộc theo kênh: email và hộp thư cần tiêu đề + chữ; SMS / ZNS cần tin ngắn;
        // đẩy cần tiêu đề + tin ngắn.
        let (needs_subject, needs_text, needs_short) = match channel {
            Channel::Email | Channel::InApp => (true, true, false),
            Channel::Sms | Channel::Zns => (false, false, true),
            Channel::WebPush | Channel::MobilePush => (true, false, true),
        };
        let html = content.body_html.as_deref().unwrap_or_default();
        for (field, value, max, needed) in [
            ("subject", content.subject.as_str(), 300, needs_subject),
            ("body_text", content.body_text.as_str(), 20_000, needs_text),
            ("body_html", html, 60_000, false),
            ("short", content.short.as_str(), 500, needs_short),
        ] {
            let count = value.chars().count();
            if (needed && value.trim().is_empty()) || count > max {
                problems.push(
                    FieldError::new(field, "length", "must be 1 to {max} characters")
                        .with_param("max", max),
                );
            }
        }
        if content.body_html.is_some() && channel != Channel::Email {
            problems.push(FieldError::new(
                "body_html",
                "invalid",
                "is only used on the `email` channel",
            ));
        }
        if let Some(id) = &content.zns_template_id
            && (id.is_empty() || id.len() > 64)
        {
            problems.push(FieldError::new(
                "zns_template_id",
                "length",
                "must be 1 to 64 characters",
            ));
        }
        let schema = schema_of(kind);
        // Biến bắt buộc chỉ đòi ở phần chính của kênh (email: chữ; SMS / ZNS / đẩy: tin ngắn).
        let optional = TemplateSchema::new(
            schema
                .variables
                .iter()
                .map(|variable| Variable::optional(variable.path, variable.description))
                .collect(),
        );
        let main = if needs_short { "short" } else { "body_text" };
        for (field, value) in [
            ("subject", content.subject.as_str()),
            ("body_text", content.body_text.as_str()),
            ("body_html", html),
            ("short", content.short.as_str()),
        ] {
            if value.is_empty() && field != main {
                continue;
            }
            let rules = if field == main { &schema } else { &optional };
            if let Err(error) = self.templates.validate(field, value, rules) {
                if error.fields().is_empty() {
                    return Err(error);
                }
                problems.extend(error.fields().iter().cloned());
            }
        }
        if problems.is_empty() { Ok(()) } else { Err(Error::validation(problems)) }
    }

    /// Dựng `content` với dữ liệu `context` cho `channel`. Email: HTML hoàn chỉnh trong khung có
    /// thương hiệu của khách, chân thư huỷ nhận cho tin quảng cáo
    /// (xem trước: link `#` — link thật chỉ sinh lúc gửi).
    pub(crate) async fn render_content(
        &self,
        tenant: &TenantContext,
        kind: &NotificationKind,
        channel: Channel,
        locale: &str,
        content: &Content,
        context: &Value,
    ) -> Result<Preview, Error> {
        let engine = &self.templates;
        let subject = engine.render(&content.subject, Output::Text, locale, context)?;
        let text = engine.render(&content.body_text, Output::Text, locale, context)?;
        let short = engine.render(&content.short, Output::Text, locale, context)?;
        let html = if channel == Channel::Email {
            let body = match &content.body_html {
                Some(source) => engine.render(source, Output::Html, locale, context)?,
                None => text_to_html(&text),
            };
            let footer = self.unsubscribe_footer_html(kind, locale, None);
            let brand = self.brand(tenant).await?;
            Some(mail_html(&brand, &subject, &body, footer.as_deref())?)
        } else {
            None
        };
        Ok(Preview { subject, text, html, short })
    }

    /// Chân thư huỷ nhận (HTML) của tin quảng cáo; tin khác: `None`.
    pub(crate) fn unsubscribe_footer_html(
        &self,
        kind: &NotificationKind,
        locale: &str,
        url: Option<&str>,
    ) -> Option<String> {
        if kind.category != Category::Marketing {
            return None;
        }
        let url = url.unwrap_or("#");
        let language = Language::from_tag(locale).unwrap_or(Language::En);
        let footer = UNSUBSCRIBE_FOOTER.render(&self.translations, language, &[("url", url)]);
        Some(format!(
            "<p>{}</p><p><a href=\"{}\">{}</a></p>",
            escape(&footer.text),
            escape(url),
            escape(&footer.subject)
        ))
    }

    /// Bản đang dùng của (loại, kênh, ngôn ngữ), nếu khách đã xuất bản.
    pub(crate) async fn live_content(
        &self,
        tenant: &TenantContext,
        kind: &str,
        channel: &str,
        locale: &str,
    ) -> Result<Option<Content>, Error> {
        let read = tenant.database().read();
        let Some(live) = template_live::Entity::find_by_id((
            tenant.id().as_uuid(),
            kind.to_owned(),
            channel.to_owned(),
            locale.to_owned(),
        ))
        .one(read)
        .await?
        else {
            return Ok(None);
        };
        let version = template_version::Entity::find()
            .scoped(tenant)
            .filter(template_version::Column::Kind.eq(kind))
            .filter(template_version::Column::Channel.eq(channel))
            .filter(template_version::Column::Locale.eq(locale))
            .filter(template_version::Column::Version.eq(live.version))
            .one(read)
            .await?;
        Ok(version.as_ref().map(Content::from))
    }

    pub(crate) async fn get_draft(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        target: (&str, &str, &str),
    ) -> Result<template_draft::Model, Error> {
        require_read(principal)?;
        let (kind, channel, locale) = self.template_target(target.0, target.1, target.2)?;
        template_draft::Entity::find_by_id(key_of(tenant, &kind, channel, locale))
            .one(tenant.database().read())
            .await?
            .ok_or_else(draft_not_found)
    }

    pub(crate) async fn put_draft(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        target: (&str, &str, &str),
        content: Content,
    ) -> Result<template_draft::Model, Error> {
        principal.require(&TEMPLATES_MANAGE)?;
        let (kind, channel, locale) = self.template_target(target.0, target.1, target.2)?;
        self.check_content(&kind, channel, &content)?;
        let transaction = tenant.database().begin_write().await?;
        template_draft::Entity::insert(template_draft::ActiveModel {
            tenant_id: Set(tenant.id().as_uuid()),
            kind: Set(kind.key().to_owned()),
            channel: Set(channel.as_str().to_owned()),
            locale: Set(locale.to_owned()),
            subject: Set(content.subject),
            body_text: Set(content.body_text),
            body_html: Set(content.body_html),
            short: Set(content.short),
            zns_template_id: Set(content.zns_template_id),
            updated_by: Set(Some(principal.user_id())),
            updated_at: Set(now()),
        })
        .on_conflict(
            OnConflict::columns([
                template_draft::Column::TenantId,
                template_draft::Column::Kind,
                template_draft::Column::Channel,
                template_draft::Column::Locale,
            ])
            .update_columns([
                template_draft::Column::Subject,
                template_draft::Column::BodyText,
                template_draft::Column::BodyHtml,
                template_draft::Column::Short,
                template_draft::Column::ZnsTemplateId,
                template_draft::Column::UpdatedBy,
                template_draft::Column::UpdatedAt,
            ])
            .to_owned(),
        )
        .exec_without_returning(&transaction)
        .await?;
        let payload = json!({"kind": kind.key(), "channel": channel.as_str(), "locale": locale, "draft": true});
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                reference(&kind),
                &events::TemplateUpdated(payload),
            )
            .await?;
        let saved = template_draft::Entity::find_by_id(key_of(tenant, &kind, channel, locale))
            .one(&transaction)
            .await?
            .ok_or_else(|| Error::internal("a template draft vanished"))?;
        transaction.commit().await?;
        Ok(saved)
    }

    pub(crate) async fn delete_draft(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        target: (&str, &str, &str),
    ) -> Result<(), Error> {
        principal.require(&TEMPLATES_MANAGE)?;
        let (kind, channel, locale) = self.template_target(target.0, target.1, target.2)?;
        let deleted = template_draft::Entity::delete_by_id(key_of(tenant, &kind, channel, locale))
            .exec(tenant.database().write())
            .await?
            .rows_affected;
        if deleted == 0 { Err(draft_not_found()) } else { Ok(()) }
    }

    /// Xuất bản bản nháp thành phiên bản mới (không đổi được nữa), dùng ngay, bỏ bản nháp.
    pub(crate) async fn publish_template(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        target: (&str, &str, &str),
    ) -> Result<template_version::Model, Error> {
        principal.require(&TEMPLATES_MANAGE)?;
        let (kind, channel, locale) = self.template_target(target.0, target.1, target.2)?;
        let key = key_of(tenant, &kind, channel, locale);
        let transaction = tenant.database().begin_write().await?;
        let draft = template_draft::Entity::find_by_id(key.clone())
            .one(&transaction)
            .await?
            .ok_or_else(draft_not_found)?;
        // Kiểm lại: loại có thể đã đổi schema từ lúc lưu nháp.
        self.check_content(&kind, channel, &Content::from(&draft))?;
        let latest = template_version::Entity::find()
            .scoped(tenant)
            .filter(template_version::Column::Kind.eq(kind.key()))
            .filter(template_version::Column::Channel.eq(channel.as_str()))
            .filter(template_version::Column::Locale.eq(locale))
            .order_by_desc(template_version::Column::Version)
            .one(&transaction)
            .await?
            .map_or(0, |row| row.version);
        let at = now();
        let version = template_version::Model {
            id: Uuid::now_v7(),
            tenant_id: key.0,
            kind: key.1.clone(),
            channel: key.2.clone(),
            locale: key.3.clone(),
            version: latest + 1,
            subject: draft.subject,
            body_text: draft.body_text,
            body_html: draft.body_html,
            short: draft.short,
            zns_template_id: draft.zns_template_id,
            published_by: Some(principal.user_id()),
            published_at: at,
        };
        template_version::Entity::insert(template_version::ActiveModel::from(version.clone()))
            .exec_without_returning(&transaction)
            .await?;
        self.set_live(&transaction, tenant, principal, &key, version.version).await?;
        template_draft::Entity::delete_by_id(key.clone()).exec(&transaction).await?;
        template_version::Entity::delete_many()
            .filter(template_version::Column::Kind.eq(kind.key()))
            .filter(template_version::Column::Channel.eq(channel.as_str()))
            .filter(template_version::Column::Locale.eq(locale))
            .filter(template_version::Column::Version.lte(version.version - KEEP_VERSIONS))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let payload = json!({"kind": kind.key(), "channel": channel.as_str(), "locale": locale, "version": version.version});
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                reference(&kind),
                &events::TemplatePublished(payload),
            )
            .await?;
        transaction.commit().await?;
        Ok(version)
    }

    async fn set_live(
        &self,
        transaction: &sea_orm::DatabaseTransaction,
        _tenant: &TenantContext,
        principal: &Principal,
        key: &(Uuid, String, String, String),
        version: i32,
    ) -> Result<(), Error> {
        template_live::Entity::insert(template_live::ActiveModel {
            tenant_id: Set(key.0),
            kind: Set(key.1.clone()),
            channel: Set(key.2.clone()),
            locale: Set(key.3.clone()),
            version: Set(version),
            updated_by: Set(Some(principal.user_id())),
            updated_at: Set(now()),
        })
        .on_conflict(
            OnConflict::columns([
                template_live::Column::TenantId,
                template_live::Column::Kind,
                template_live::Column::Channel,
                template_live::Column::Locale,
            ])
            .update_columns([
                template_live::Column::Version,
                template_live::Column::UpdatedBy,
                template_live::Column::UpdatedAt,
            ])
            .to_owned(),
        )
        .exec_without_returning(transaction)
        .await?;
        Ok(())
    }

    /// Dùng lại một bản đã xuất bản (bản đó không đổi; không tạo phiên bản mới).
    pub(crate) async fn rollback_template(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        target: (&str, &str, &str),
        version: i32,
    ) -> Result<template_version::Model, Error> {
        principal.require(&TEMPLATES_MANAGE)?;
        let (kind, channel, locale) = self.template_target(target.0, target.1, target.2)?;
        let key = key_of(tenant, &kind, channel, locale);
        let transaction = tenant.database().begin_write().await?;
        let found = template_version::Entity::find()
            .scoped(tenant)
            .filter(template_version::Column::Kind.eq(kind.key()))
            .filter(template_version::Column::Channel.eq(channel.as_str()))
            .filter(template_version::Column::Locale.eq(locale))
            .filter(template_version::Column::Version.eq(version))
            .one(&transaction)
            .await?
            .ok_or_else(version_not_found)?;
        self.set_live(&transaction, tenant, principal, &key, version).await?;
        let payload = json!({"kind": kind.key(), "channel": channel.as_str(), "locale": locale, "version": version});
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                reference(&kind),
                &events::TemplateRolledBack(payload),
            )
            .await?;
        transaction.commit().await?;
        Ok(found)
    }

    /// Thôi dùng mẫu đã xuất bản (các bản vẫn giữ để quay lại): câu thay thế cũ hay mẫu mặc định
    /// áp lại.
    pub(crate) async fn unpublish_template(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        target: (&str, &str, &str),
    ) -> Result<(), Error> {
        principal.require(&TEMPLATES_MANAGE)?;
        let (kind, channel, locale) = self.template_target(target.0, target.1, target.2)?;
        let transaction = tenant.database().begin_write().await?;
        let deleted = template_live::Entity::delete_by_id(key_of(tenant, &kind, channel, locale))
            .exec(&transaction)
            .await?
            .rows_affected;
        if deleted == 0 {
            return Err(not_published());
        }
        let payload = json!({"kind": kind.key(), "channel": channel.as_str(), "locale": locale});
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                reference(&kind),
                &events::TemplateUnpublished(payload),
            )
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub(crate) async fn list_versions(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        target: (&str, &str, &str),
    ) -> Result<Versions, Error> {
        require_read(principal)?;
        let (kind, channel, locale) = self.template_target(target.0, target.1, target.2)?;
        let read = tenant.database().read();
        let items = template_version::Entity::find()
            .scoped(tenant)
            .filter(template_version::Column::Kind.eq(kind.key()))
            .filter(template_version::Column::Channel.eq(channel.as_str()))
            .filter(template_version::Column::Locale.eq(locale))
            .order_by_desc(template_version::Column::Version)
            .all(read)
            .await?;
        let live = template_live::Entity::find_by_id(key_of(tenant, &kind, channel, locale))
            .one(read)
            .await?
            .map(|row| row.version);
        Ok(Versions { items, live })
    }

    /// Nội dung của bản `source`.
    async fn content_of(
        &self,
        tenant: &TenantContext,
        kind: &NotificationKind,
        channel: Channel,
        locale: &str,
        source: Source,
    ) -> Result<Content, Error> {
        let read = tenant.database().read();
        match source {
            Source::Draft => {
                template_draft::Entity::find_by_id(key_of(tenant, kind, channel, locale))
                    .one(read)
                    .await?
                    .as_ref()
                    .map(Content::from)
                    .ok_or_else(draft_not_found)
            }
            Source::Published => self
                .live_content(tenant, kind.key(), channel.as_str(), locale)
                .await?
                .ok_or_else(not_published),
            Source::Version(version) => template_version::Entity::find()
                .scoped(tenant)
                .filter(template_version::Column::Kind.eq(kind.key()))
                .filter(template_version::Column::Channel.eq(channel.as_str()))
                .filter(template_version::Column::Locale.eq(locale))
                .filter(template_version::Column::Version.eq(version))
                .one(read)
                .await?
                .as_ref()
                .map(Content::from)
                .ok_or_else(version_not_found),
        }
    }

    /// Xem trước bản `source` với dữ liệu mẫu của loại, hay `data` của người gọi (chỉ khoá có
    /// trong schema).
    pub(crate) async fn preview_template(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        target: (&str, &str, &str),
        source: Source,
        data: Option<Value>,
    ) -> Result<Preview, Error> {
        require_read(principal)?;
        let (kind, channel, locale) = self.template_target(target.0, target.1, target.2)?;
        let schema = schema_of(&kind);
        let context = match data {
            None | Some(Value::Null) => sample_of(&kind),
            Some(Value::Object(map)) => {
                let unknown: Vec<FieldError> = map
                    .keys()
                    .filter(|key| !schema_has_root(&schema, key))
                    .map(|key| {
                        FieldError::new(
                            format!("data.{key}"),
                            "template.unknown_variable",
                            "is not a variable of this kind of template",
                        )
                    })
                    .collect();
                if !unknown.is_empty() {
                    return Err(Error::validation(unknown));
                }
                Value::Object(map)
            }
            Some(_) => {
                return Err(Error::validation(vec![FieldError::new(
                    "data",
                    "invalid",
                    "must be an object",
                )]));
            }
        };
        let content = self.content_of(tenant, &kind, channel, locale, source).await?;
        self.render_content(tenant, &kind, channel, locale, &content, &context).await
    }

    /// Gửi thử bản `source` (dữ liệu mẫu) tới CHÍNH người gọi: email đã xác minh của họ, hay hộp
    /// thư của họ. Không nhận địa chỉ tuỳ ý — không dùng làm công cụ gửi thư rác; có throttle.
    pub(crate) async fn test_send_template(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        target: (&str, &str, &str),
        source: Source,
    ) -> Result<Channel, Error> {
        principal.require(&TEMPLATES_MANAGE)?;
        let (kind, channel, locale) = self.template_target(target.0, target.1, target.2)?;
        if !matches!(channel, Channel::Email | Channel::InApp) {
            return Err(test_send_unsupported());
        }
        let content = self.content_of(tenant, &kind, channel, locale, source).await?;
        let user = principal.user_id();
        let email = match channel {
            Channel::Email => {
                let mailer = self.mailer.clone().ok_or_else(test_send_unsupported)?;
                let users = self.users.clone().ok_or_else(test_address_unverified)?;
                let found = users.find(tenant, user).await?.ok_or_else(test_address_unverified)?;
                if found.email_verified_at.is_none() || !found.is_active() {
                    return Err(test_address_unverified());
                }
                Some((mailer, found.email))
            }
            _ => None,
        };
        if !self.test_throttle.hit(tenant, &format!("test:{user}")).await?.is_allowed() {
            return Err(test_send_throttled());
        }
        let rendered = self
            .render_content(tenant, &kind, channel, locale, &content, &sample_of(&kind))
            .await?;
        let subject = format!("[Test] {}", rendered.subject);
        match email {
            Some((mailer, address)) => {
                let mut mail = Mail::new(address, subject, rendered.text);
                if let Some(html) = rendered.html {
                    mail = mail.html(html);
                }
                mailer.send(tenant, mail).await?;
            }
            None => {
                inbox::Entity::insert(inbox::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    user_id: Set(user),
                    kind: Set(kind.key().to_owned()),
                    category: Set(kind.category.as_str().to_owned()),
                    title: Set(subject.chars().take(300).collect()),
                    body: Set(rendered.text),
                    link: Set(None),
                    read_at: Set(None),
                    created_at: Set(now()),
                })
                .exec_without_returning(tenant.database().write())
                .await?;
            }
        }
        tracing::info!(
            kind = kind.key(),
            channel = channel.as_str(),
            "notification template test sent"
        );
        Ok(channel)
    }

    /// Ghi `template_fallback` vào tin đã phải dùng mẫu mặc định vì mẫu của khách không dựng được.
    pub(crate) async fn mark_fallback(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<(), Error> {
        crate::entity::message::Entity::update_many()
            .col_expr(
                crate::entity::message::Column::ErrorCode,
                Expr::value(Some("template_fallback")),
            )
            .filter(crate::entity::message::Column::Id.eq(id))
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?;
        Ok(())
    }
}

/// Xem mẫu: `notifications.templates.read` hay `notifications.templates.manage`.
pub(crate) fn require_read(principal: &Principal) -> Result<(), Error> {
    if principal.has(&TEMPLATES_READ) { Ok(()) } else { principal.require(&TEMPLATES_MANAGE) }
}

fn reference(kind: &NotificationKind) -> Option<String> {
    Some(format!("notifications.template:{}", kind.key()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::NotificationTemplate;

    const KIND: NotificationKind = NotificationKind::new(
        Category::Transactional,
        &[Channel::Email],
        NotificationTemplate::new("x.y", "Order {code}", "{code} for {name}", "{code}"),
    );

    #[test]
    fn schema_va_du_lieu_mau_suy_tu_mau_mac_dinh() {
        let schema = schema_of(&KIND);
        let paths: Vec<_> = schema.variables.iter().map(|variable| variable.path).collect();
        assert_eq!(paths, ["code", "name", "link"]);
        assert_eq!(sample_of(&KIND)["code"], "[code]");
        const DECLARED: NotificationKind = KIND
            .variables(&[Variable::required("order.code", "Code")])
            .sample(r#"{"order": {"code": "DH1"}}"#);
        assert!(schema_has_root(&schema_of(&DECLARED), "order"));
        assert!(!schema_has_root(&schema_of(&DECLARED), "secret"));
        assert_eq!(sample_of(&DECLARED)["order"]["code"], "DH1");
        assert_eq!(send_context(r#"{"a": 1}"#, None)["link"], "");
    }
}
