//! Đường dây thoại của một nút (`conversations_voice_lines`, một dòng mỗi nút): nhà cung cấp,
//! hotline, giờ nhận gọi lại, ghi âm (tắt mặc định). Token trong đường dẫn webhook chỉ hiện một lần
//! lúc tạo hay khi xin token mới; database giữ bản băm.

use std::sync::Arc;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::secret::{random_token, sha256};
use autoxide_kernel::sms::PhoneNumber;
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext};
use autoxide_kernel::voice::VoiceProvider;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, Set};
use serde_json::json;
use uuid::Uuid;

use super::hours::{DEFAULT_OFFSET_MINUTES, Range, default_hours, valid};
use crate::entity::voice_line;
use crate::events::{self, Change};
use crate::permissions::VOICE_MANAGE;
use crate::service::{ConversationsService, finish, now, org_exists};

pub(crate) type Line = voice_line::Model;

/// Cấu hình đường dây cần đặt.
#[derive(Debug, Clone)]
pub(crate) struct LineInput {
    pub provider: String,
    pub hotline: String,
    pub recording_enabled: bool,
    pub callback_hours: Option<Vec<Range>>,
    pub utc_offset_minutes: Option<i32>,
    pub active: bool,
    /// Cấp token webhook mới (token cũ hết dùng).
    pub rotate_token: bool,
}

impl ConversationsService {
    /// Nhà cung cấp tên `name` đã có bộ nối.
    pub(crate) fn provider(&self, name: &str) -> Option<Arc<dyn VoiceProvider>> {
        self.parts.voice.iter().find(|provider| provider.provider() == name).cloned()
    }

    /// Đường dây đang bật của nút `org`.
    pub(crate) async fn line_of(
        &self,
        tenant: &TenantContext,
        org: Uuid,
    ) -> Result<Option<Line>, Error> {
        Ok(voice_line::Entity::find()
            .filter(voice_line::Column::OrgId.eq(org))
            .filter(voice_line::Column::Active.eq(true))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?)
    }

    /// Đường dây của nút `org` (`conversations.voice.manage`); chưa đặt → `None`.
    pub(crate) async fn get_line(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Uuid,
    ) -> Result<Option<Line>, Error> {
        let target = OrgTarget::new(OrgId::from_uuid(org), None);
        principal.require_at(tenant.database().read(), &VOICE_MANAGE, target).await?;
        Ok(voice_line::Entity::find()
            .filter(voice_line::Column::OrgId.eq(org))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?)
    }

    /// Đặt đường dây của nút `org`; trả đường dây và token webhook khi vừa cấp.
    pub(crate) async fn put_line(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Uuid,
        input: LineInput,
    ) -> Result<(Line, Option<String>), Error> {
        let target = OrgTarget::new(OrgId::from_uuid(org), None);
        principal.require_at(tenant.database().read(), &VOICE_MANAGE, target).await?;
        let mut problems = Vec::new();
        if !org_exists(tenant, OrgId::from_uuid(org)).await? {
            problems.push(FieldError::new("org_id", "unknown", "is not a unit of this shop"));
        }
        let provider = self.provider(&input.provider);
        if provider.is_none() {
            problems.push(FieldError::new(
                "provider",
                "unknown",
                "is not a voice provider installed on this server",
            ));
        }
        let hotline = PhoneNumber::parse(&input.hotline, "84");
        if hotline.is_none() {
            problems.push(FieldError::new("hotline", "invalid", "is not a phone number"));
        }
        let hours = input.callback_hours.clone().unwrap_or_else(default_hours);
        if !valid(&hours) {
            problems.push(FieldError::new(
                "callback_hours",
                "invalid",
                "list at most 50 ranges: day 1 (Monday) to 7, times HH:MM, closing after opening",
            ));
        }
        let offset = input.utc_offset_minutes.unwrap_or(DEFAULT_OFFSET_MINUTES);
        if !(-720..=840).contains(&offset) {
            problems.push(FieldError::new(
                "utc_offset_minutes",
                "range",
                "must be between -720 and 840",
            ));
        }
        finish(problems)?;
        if input.recording_enabled
            && let Some(provider) = &provider
        {
            let capabilities = provider.capabilities();
            if !(capabilities.recording && capabilities.recording_notice) {
                return Err(recording_unsupported());
            }
        }
        let hotline = hotline.map(|phone| phone.as_str().to_owned()).unwrap_or_default();
        let existing = voice_line::Entity::find()
            .filter(voice_line::Column::OrgId.eq(org))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?;
        let token =
            (existing.is_none() || input.rotate_token).then(|| random_token(32)).transpose()?;
        let at = now();
        let transaction = tenant.database().begin_write().await?;
        let hours_json = serde_json::to_string(&hours).map_err(Error::internal)?;
        let after = match existing.clone() {
            Some(row) => {
                let mut active = row.into_active_model();
                active.provider = Set(input.provider.clone());
                active.hotline = Set(hotline);
                active.recording_enabled = Set(input.recording_enabled);
                active.callback_hours = Set(hours_json);
                active.utc_offset_minutes = Set(offset);
                active.active = Set(input.active);
                if let Some(token) = &token {
                    active.line_token_hash = Set(sha256(token));
                }
                active.updated_at = Set(at);
                active.update(&transaction).await?
            }
            None => {
                voice_line::ActiveModel {
                    id: Set(Uuid::now_v7()),
                    tenant_id: Set(tenant.id().as_uuid()),
                    org_id: Set(org),
                    provider: Set(input.provider.clone()),
                    hotline: Set(hotline),
                    line_token_hash: Set(token.as_deref().map(sha256).unwrap_or_default()),
                    recording_enabled: Set(input.recording_enabled),
                    callback_hours: Set(hours_json),
                    utc_offset_minutes: Set(offset),
                    active: Set(input.active),
                    updated_at: Set(at),
                }
                .insert(&transaction)
                .await?
            }
        };
        let snapshot = |line: &Line| json!({"org_id": line.org_id, "provider": line.provider, "recording_enabled": line.recording_enabled, "active": line.active});
        let event = events::VoiceLineChanged(Change {
            before: existing.as_ref().map(snapshot),
            after: Some(snapshot(&after)),
            audience: Vec::new(),
        });
        let subject = Some(format!("conversations.voice_line:{}", after.id));
        self.parts
            .outbox
            .record(&transaction, tenant, Actor::from(principal), subject, &event)
            .await?;
        transaction.commit().await?;
        Ok((after, token))
    }
}

/// Giờ nhận gọi lại của đường dây.
pub(crate) fn hours_of(line: &Line) -> Vec<Range> {
    serde_json::from_str(&line.callback_hours).unwrap_or_else(|_| default_hours())
}

pub(crate) fn voice_unavailable() -> Error {
    Error::not_found("conversations.voice_unavailable", "This shop has not set up calls.")
}

pub(crate) fn recording_unsupported() -> Error {
    Error::new(
        ErrorKind::Validation,
        "conversations.recording_unsupported",
        "The voice provider cannot play a recording notice, so recording cannot be switched on.",
    )
}
