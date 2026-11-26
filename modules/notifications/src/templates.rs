//! Dựng nội dung tin: mẫu khách tự soạn đang dùng (D-206, [`crate::custom`]), rồi câu chữ thay thế cũ
//! của khách (`notifications_templates`, chỉ chỗ điền `{tên}`), rồi mẫu mặc định theo i18n.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::i18n::Language;
use autoxide_kernel::pagination::{Page, PageRequest, SortValue, fetch_page_by};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, Set};
use serde_json::json;

use crate::api::{Channel, NotificationKind};
use crate::entity::{message, template};
use crate::events;
use crate::permissions::TEMPLATES_MANAGE;
use crate::service::{NotificationsService, kind_unknown, now};

/// Nội dung đã dựng của một tin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rendered {
    pub title: String,
    pub body: String,
    pub short: String,
    /// Phần HTML do khách soạn (đã dựng, đã thoát giá trị; chưa đặt vào khung) — chỉ email.
    pub html: Option<String>,
    /// Mẫu của khách không dựng được lúc gửi: đã dùng mẫu mặc định (`template_fallback`).
    pub fallback: bool,
}

/// Điền `params` vào `template` một lượt, từ trái sang phải (giá trị đã điền không bị thay tiếp).
/// Thiếu giá trị cho một chỗ điền → `None`.
pub(crate) fn fill(template: &str, params: &[(String, String)]) -> Option<String> {
    let mut text = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        text.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let name = after.split_once('}').map(|(name, _)| name).filter(|name| {
            !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
        match name {
            Some(name) => {
                let (_, value) = params.iter().find(|(key, _)| key == name)?;
                text.push_str(value);
                rest = &after[name.len() + 1..];
            }
            None => {
                text.push('{');
                rest = after;
            }
        }
    }
    text.push_str(rest);
    Some(text)
}

/// Tên các chỗ điền `{tên}` trong `template`.
pub(crate) fn placeholders(template: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        if let Some((name, _)) = after.split_once('}')
            && !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            if !names.iter().any(|known: &String| known == name) {
                names.push(name.to_owned());
            }
            rest = &after[name.len() + 1..];
        } else {
            rest = after;
        }
    }
    names
}

pub(crate) fn template_invalid(params: &[String]) -> Error {
    Error::new(
        ErrorKind::Validation,
        "notifications.template_invalid",
        "The template uses values the notification kind does not have: {params}.",
    )
    .with_param("params", params.join(", "))
}

pub(crate) fn template_not_found() -> Error {
    Error::not_found(
        "notifications.template_not_found",
        "The tenant has not replaced this text; the default text is in use.",
    )
}

/// Câu chữ thay thế gửi lên.
#[derive(Debug, Clone)]
pub(crate) struct TemplateInput {
    pub title: String,
    pub body: String,
    pub short: String,
    pub zns_template_id: Option<String>,
}

impl NotificationsService {
    /// Nội dung của tin `row`: câu thay thế của khách (đúng loại, kênh, ngôn ngữ) nếu điền được, rồi bản
    /// dịch của mẫu mặc định, rồi câu tiếng Anh của mẫu mặc định.
    pub(crate) async fn render(
        &self,
        tenant: &TenantContext,
        kind: &NotificationKind,
        row: &message::Model,
    ) -> Result<Rendered, Error> {
        // Mẫu khách tự soạn đang dùng (ngoài transaction). Lỗi dựng → mẫu mặc định, không bỏ tin.
        let mut fallback = false;
        if let Some(content) =
            self.live_content(tenant, &row.kind, &row.channel, &row.locale).await?
        {
            let context = crate::custom::send_context(&row.params, row.link.as_deref());
            match self.render_parts(&content, &row.locale, &context) {
                Ok(rendered) => return Ok(rendered),
                Err(error) => {
                    tracing::warn!(
                        kind = row.kind.as_str(),
                        channel = row.channel.as_str(),
                        error_code = error.code(),
                        "tenant template failed, using the default text"
                    );
                    fallback = true;
                }
            }
        }
        let custom = template::Entity::find_by_id((
            tenant.id().as_uuid(),
            row.kind.clone(),
            row.channel.clone(),
            row.locale.clone(),
        ))
        .one(tenant.database().read())
        .await?;
        let mut params = NotificationsService::params_of(&row.params);
        if let Some(link) = &row.link {
            params.push(("link".to_owned(), link.clone()));
        }
        let language = Language::from_tag(&row.locale).unwrap_or(Language::En);
        let pick = |part: &str, english: &str, custom: Option<&str>| {
            custom
                .filter(|text| !text.is_empty())
                .and_then(|text| fill(text, &params))
                .or_else(|| {
                    self.translations
                        .text(&format!("{}.{part}", kind.key()), language)
                        .and_then(|text| fill(text, &params))
                })
                .or_else(|| fill(english, &params))
                .unwrap_or_else(|| english.to_owned())
        };
        let template = kind.template;
        Ok(Rendered {
            title: pick("title", template.title, custom.as_ref().map(|row| row.title.as_str())),
            body: pick("body", template.body, custom.as_ref().map(|row| row.body.as_str())),
            short: pick("short", template.short, custom.as_ref().map(|row| row.short.as_str())),
            html: None,
            fallback,
        })
    }

    /// Dựng các phần của mẫu khách tự soạn bằng bộ dựng của kernel.
    fn render_parts(
        &self,
        content: &crate::custom::Content,
        locale: &str,
        context: &serde_json::Value,
    ) -> Result<Rendered, Error> {
        use autoxide_kernel::template::Output;
        let engine = &self.templates;
        Ok(Rendered {
            title: engine.render(&content.subject, Output::Text, locale, context)?,
            body: engine.render(&content.body_text, Output::Text, locale, context)?,
            short: engine.render(&content.short, Output::Text, locale, context)?,
            html: match &content.body_html {
                Some(source) => Some(engine.render(source, Output::Html, locale, context)?),
                None => None,
            },
            fallback: false,
        })
    }

    /// Một trang câu thay thế của khách: theo loại, rồi ID (UUIDv7 cấp lúc ghi lần đầu; dòng cũ được cấp
    /// theo kênh rồi ngôn ngữ ở migration `m0011`), keyset trong database (index
    /// `notifications_templates_page`).
    pub(crate) async fn list_templates(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        page: &PageRequest,
    ) -> Result<Page<template::Model>, Error> {
        principal.require(&TEMPLATES_MANAGE)?;
        fetch_page_by(
            template::Entity::find().scoped(tenant),
            template::Column::Kind,
            |row| SortValue::Text(row.kind.clone()),
            template::Column::Id,
            |row| row.id,
            page,
            tenant.database().read(),
        )
        .await
    }

    /// Kiểm (loại, kênh, ngôn ngữ) của đường dẫn mẫu; trả loại và ngôn ngữ chuẩn.
    pub(crate) fn template_target(
        &self,
        kind: &str,
        channel: &str,
        locale: &str,
    ) -> Result<(NotificationKind, Channel, &'static str), Error> {
        let found = self.kinds.get(kind).ok_or_else(|| kind_unknown(kind))?;
        let mut problems = Vec::new();
        let channel = Channel::parse(channel).filter(|channel| found.channels.contains(channel));
        if channel.is_none() {
            problems.push(
                FieldError::new("channel", "invalid", "must be one of {channels}").with_param(
                    "channels",
                    found
                        .channels
                        .iter()
                        .map(|channel| channel.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                ),
            );
        }
        let language = Language::from_tag(locale).filter(|language| language.tag() == locale);
        if language.is_none() {
            problems.push(FieldError::new("locale", "invalid", "must be `en` or `vi`"));
        }
        match (channel, language) {
            (Some(channel), Some(language)) if problems.is_empty() => {
                Ok((found, channel, language.tag()))
            }
            _ => Err(Error::validation(problems)),
        }
    }

    pub(crate) async fn put_template(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        target: (&str, &str, &str),
        input: TemplateInput,
    ) -> Result<template::Model, Error> {
        principal.require(&TEMPLATES_MANAGE)?;
        let (kind, channel, locale) = self.template_target(target.0, target.1, target.2)?;
        let mut problems = Vec::new();
        for (field, value, max) in [
            ("title", &input.title, 300),
            ("body", &input.body, 20_000),
            ("short", &input.short, 500),
        ] {
            if value.trim().is_empty() || value.chars().count() > max {
                problems.push(
                    FieldError::new(field, "length", "must be 1 to {max} characters")
                        .with_param("max", max),
                );
            }
        }
        if let Some(id) = &input.zns_template_id
            && (id.is_empty() || id.len() > 64)
        {
            problems.push(FieldError::new(
                "zns_template_id",
                "length",
                "must be 1 to 64 characters",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        // Chỉ dùng được chỗ điền mà mẫu mặc định có (cùng tham số module gửi kèm).
        let mut known = placeholders(kind.template.title);
        known.extend(placeholders(kind.template.body));
        known.extend(placeholders(kind.template.short));
        known.push("link".to_owned());
        let mut unknown: Vec<String> = [&input.title, &input.body, &input.short]
            .into_iter()
            .flat_map(|text| placeholders(text))
            .filter(|name| !known.contains(name))
            .collect();
        unknown.dedup();
        if !unknown.is_empty() {
            return Err(template_invalid(&unknown));
        }
        let transaction = tenant.database().begin_write().await?;
        template::Entity::insert(template::ActiveModel {
            // Ghi lại cùng (loại, kênh, ngôn ngữ) đi nhánh `ON CONFLICT` bên dưới, không đổi `id`.
            id: Set(uuid::Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            kind: Set(kind.key().to_owned()),
            channel: Set(channel.as_str().to_owned()),
            locale: Set(locale.to_owned()),
            title: Set(input.title),
            body: Set(input.body),
            short: Set(input.short),
            zns_template_id: Set(input.zns_template_id),
            updated_by: Set(Some(principal.user_id())),
            updated_at: Set(now()),
        })
        .on_conflict(
            OnConflict::columns([
                template::Column::TenantId,
                template::Column::Kind,
                template::Column::Channel,
                template::Column::Locale,
            ])
            .update_columns([
                template::Column::Title,
                template::Column::Body,
                template::Column::Short,
                template::Column::ZnsTemplateId,
                template::Column::UpdatedBy,
                template::Column::UpdatedAt,
            ])
            .to_owned(),
        )
        .exec_without_returning(&transaction)
        .await?;
        let key = (
            tenant.id().as_uuid(),
            kind.key().to_owned(),
            channel.as_str().to_owned(),
            locale.to_owned(),
        );
        let event = events::TemplateUpdated(
            json!({"kind": kind.key(), "channel": channel.as_str(), "locale": locale}),
        );
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                Some(format!("notifications.template:{}", kind.key())),
                &event,
            )
            .await?;
        let saved = template::Entity::find_by_id(key)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| Error::internal("a notification template vanished"))?;
        transaction.commit().await?;
        Ok(saved)
    }

    pub(crate) async fn delete_template(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        target: (&str, &str, &str),
    ) -> Result<(), Error> {
        principal.require(&TEMPLATES_MANAGE)?;
        let (kind, channel, locale) = self.template_target(target.0, target.1, target.2)?;
        let transaction = tenant.database().begin_write().await?;
        let deleted = template::Entity::delete_many()
            .filter(template::Column::Kind.eq(kind.key()))
            .filter(template::Column::Channel.eq(channel.as_str()))
            .filter(template::Column::Locale.eq(locale))
            .scoped(tenant)
            .exec(&transaction)
            .await?
            .rows_affected;
        if deleted == 0 {
            return Err(template_not_found());
        }
        let event = events::TemplateDeleted(
            json!({"kind": kind.key(), "channel": channel.as_str(), "locale": locale}),
        );
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                Some(format!("notifications.template:{}", kind.key())),
                &event,
            )
            .await?;
        transaction.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dien_mot_luot_va_bao_thieu() {
        let params = vec![("code".to_owned(), "{x}".to_owned()), ("x".to_owned(), "!".to_owned())];
        assert_eq!(fill("Order {code} {x}", &params).as_deref(), Some("Order {x} !"));
        assert_eq!(fill("Order {missing}", &params), None);
        assert_eq!(fill("a { b } {}", &params).as_deref(), Some("a { b } {}"));
        assert_eq!(placeholders("{a} {b} {a} { c }"), ["a", "b"]);
    }
}
