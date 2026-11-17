//! Gửi thông báo qua REST / MCP (cùng permission) và xem nhật ký gửi.

use autoxide_compliance::api::ConsentKey;
use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext, resolve_link_token};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use uuid::Uuid;

use crate::api::{Category, Channel, Notification, Outcome};
use crate::entity::message;
use crate::permissions::{MESSAGES_READ, SEND, SEND_MARKETING};
use crate::service::{NotificationsService, consent_subject, kind_unknown, link_invalid};

/// Bộ lọc nhật ký gửi.
#[derive(Debug, Clone, Default)]
pub(crate) struct MessageFilter {
    pub kind: Option<String>,
    pub status: Option<String>,
    pub channel: Option<String>,
    pub user_id: Option<Uuid>,
}

const STATUSES: [&str; 6] = ["queued", "deferred", "sent", "failed", "skipped", "canceled"];

impl NotificationsService {
    /// Gửi theo yêu cầu của người gọi: `notifications.send`; tin quảng cáo cần thêm
    /// `notifications.send_marketing`. Permission được kiểm TRƯỚC khi tra loại.
    pub(crate) async fn send_manual(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        notification: Notification,
    ) -> Result<Outcome, Error> {
        principal.require(&SEND)?;
        let kind =
            self.kinds.get(&notification.kind).ok_or_else(|| kind_unknown(&notification.kind))?;
        if kind.category == Category::Marketing {
            principal.require(&SEND_MARKETING)?;
        }
        self.notify(tenant, notification).await
    }

    pub(crate) async fn list_messages(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: MessageFilter,
        page: &PageRequest,
    ) -> Result<Page<message::Model>, Error> {
        principal.require(&MESSAGES_READ)?;
        let mut problems = Vec::new();
        let mut select = message::Entity::find().scoped(tenant);
        if let Some(status) = &filter.status {
            if !STATUSES.contains(&status.as_str()) {
                problems.push(
                    FieldError::new("status", "invalid", "must be one of {values}")
                        .with_param("values", STATUSES.join(", ")),
                );
            }
            select = select.filter(message::Column::Status.eq(status.as_str()));
        }
        if let Some(channel) = &filter.channel {
            if Channel::parse(channel).is_none() {
                problems.push(FieldError::new(
                    "channel",
                    "invalid",
                    "must be `email`, `sms`, `zns`, `web_push`, `in_app` or `mobile_push`",
                ));
            }
            select = select.filter(message::Column::Channel.eq(channel.as_str()));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        if let Some(kind) = &filter.kind {
            select = select.filter(message::Column::Kind.eq(kind.as_str()));
        }
        if let Some(user) = filter.user_id {
            select = select.filter(message::Column::UserId.eq(user));
        }
        fetch_page(select, message::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    pub(crate) async fn get_message(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<message::Model, Error> {
        principal.require(&MESSAGES_READ)?;
        message::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(crate::service::message_not_found)
    }

    /// Rút đồng ý quảng cáo bằng link trong email (`List-Unsubscribe`): token mang khách; đồng ý của
    /// đúng (mục đích, kênh) của tin bị rút, tin quảng cáo đang chờ bị huỷ (qua event của
    /// `compliance`). Gọi lại vô hại. Token lạ, hay tin đã quá hạn lưu địa chỉ →
    /// `notifications.link_invalid`.
    pub(crate) async fn unsubscribe(
        &self,
        request: Option<&TenantContext>,
        token: &str,
    ) -> Result<(), Error> {
        let Some((tenant, raw)) = resolve_link_token(&self.databases, request, token).await? else {
            return Err(link_invalid());
        };
        let row = message::Entity::find()
            .scoped(&tenant)
            .filter(message::Column::UnsubscribeTokenHash.eq(autoxide_kernel::secret::sha256(raw)))
            .one(tenant.database().read())
            .await?
            .ok_or_else(link_invalid)?;
        let channel = Channel::parse(&row.channel).ok_or_else(link_invalid)?;
        let subject = consent_subject(row.user_id, channel, row.address.as_deref())
            .ok_or_else(link_invalid)?;
        let ledger = self.consents.as_ref().ok_or_else(link_invalid)?;
        let key = ConsentKey::new(row.purpose.clone(), "", channel.consent_channel());
        ledger
            .withdraw(
                tenant.id(),
                subject,
                key,
                "unsubscribe_link",
                &format!("notifications.unsubscribe:{}", row.id),
            )
            .await?;
        tracing::info!(message_id = %row.id, "advertising consent withdrawn with an unsubscribe link");
        Ok(())
    }
}
