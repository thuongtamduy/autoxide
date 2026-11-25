//! Phần của chính người dùng: hộp thư trong ứng dụng, tuỳ chọn nhận, đăng ký web push, token thiết bị.
//! Chỉ cần đăng nhập; mỗi người chỉ thấy và sửa phần của mình.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};
use serde_json::json;
use uuid::Uuid;

use crate::api::{Category, Channel};
use crate::delivery::{DEVICE_TOKEN_PURPOSE, PUSH_AUTH_PURPOSE};
use crate::entity::{device_token, inbox, preference, push_subscription};
use crate::events;
use crate::service::{NotificationsService, now};

pub(crate) fn inbox_not_found() -> Error {
    Error::not_found(
        "notifications.inbox_not_found",
        "There is no such notification in your inbox.",
    )
}

pub(crate) fn preference_locked() -> Error {
    Error::new(
        ErrorKind::Validation,
        "notifications.preference_locked",
        "Security notifications cannot be turned off.",
    )
}

pub(crate) fn push_subscription_invalid() -> Error {
    Error::new(
        ErrorKind::Validation,
        "notifications.push_subscription_invalid",
        "The push subscription is not valid: send the `endpoint` (https, on a public server) and the \
         `p256dh` and `auth` keys of the browser's PushSubscription.",
    )
}

pub(crate) fn push_subscription_not_found() -> Error {
    Error::not_found(
        "notifications.push_subscription_not_found",
        "There is no such push subscription of yours.",
    )
}

pub(crate) fn device_token_not_found() -> Error {
    Error::not_found("notifications.device_token_not_found", "There is no such device of yours.")
}

/// Một thay đổi tuỳ chọn: `target` là `kind:<khoá>` hay `category:<nhóm>`.
#[derive(Debug, Clone)]
pub(crate) struct PreferenceChange {
    pub target: String,
    pub channel: String,
    pub enabled: bool,
}

/// Một đăng ký web push gửi lên.
#[derive(Debug, Clone)]
pub(crate) struct NewPushSubscription {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
    pub user_agent: Option<String>,
}

/// Một token thiết bị gửi lên.
#[derive(Debug, Clone)]
pub(crate) struct NewDeviceToken {
    pub platform: String,
    pub token: String,
    pub app_id: String,
    pub locale: Option<String>,
}

/// Chuỗi base64url (không đệm) dài `min..=max` ký tự.
fn base64url(text: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&text.len())
        && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl NotificationsService {
    // ---- Hộp thư ---------------------------------------------------------------------------------

    pub(crate) async fn inbox(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        unread_only: bool,
        page: &PageRequest,
    ) -> Result<Page<inbox::Model>, Error> {
        let mut select = inbox::Entity::find()
            .scoped(tenant)
            .filter(inbox::Column::UserId.eq(principal.user_id()));
        if unread_only {
            select = select.filter(inbox::Column::ReadAt.is_null());
        }
        fetch_page(select, inbox::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    pub(crate) async fn unread_count(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
    ) -> Result<u64, Error> {
        Ok(inbox::Entity::find()
            .scoped(tenant)
            .filter(inbox::Column::UserId.eq(principal.user_id()))
            .filter(inbox::Column::ReadAt.is_null())
            .count(tenant.database().read())
            .await?)
    }

    /// Đánh dấu đã đọc (gọi lại vô hại); trả dòng sau khi đổi.
    pub(crate) async fn mark_read(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<inbox::Model, Error> {
        let write = tenant.database().write();
        inbox::Entity::update_many()
            .col_expr(inbox::Column::ReadAt, Expr::value(Some(now())))
            .filter(inbox::Column::Id.eq(id))
            .filter(inbox::Column::UserId.eq(principal.user_id()))
            .filter(inbox::Column::ReadAt.is_null())
            .scoped(tenant)
            .exec(write)
            .await?;
        inbox::Entity::find_by_id(id)
            .scoped(tenant)
            .filter(inbox::Column::UserId.eq(principal.user_id()))
            .one(write)
            .await?
            .ok_or_else(inbox_not_found)
    }

    pub(crate) async fn mark_all_read(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
    ) -> Result<u64, Error> {
        Ok(inbox::Entity::update_many()
            .col_expr(inbox::Column::ReadAt, Expr::value(Some(now())))
            .filter(inbox::Column::UserId.eq(principal.user_id()))
            .filter(inbox::Column::ReadAt.is_null())
            .scoped(tenant)
            .exec(tenant.database().write())
            .await?
            .rows_affected)
    }

    // ---- Tuỳ chọn nhận ---------------------------------------------------------------------------

    pub(crate) async fn my_preferences(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
    ) -> Result<Vec<preference::Model>, Error> {
        Ok(preference::Entity::find()
            .scoped(tenant)
            .filter(preference::Column::UserId.eq(principal.user_id()))
            .all(tenant.database().read())
            .await?)
    }

    /// Đổi tuỳ chọn nhận; mọi lỗi của đầu vào báo một lần. Tắt thông báo bảo mật →
    /// `notifications.preference_locked`.
    pub(crate) async fn set_preferences(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        changes: Vec<PreferenceChange>,
    ) -> Result<Vec<preference::Model>, Error> {
        let mut problems = Vec::new();
        let mut locked = false;
        if changes.is_empty() || changes.len() > 200 {
            problems.push(FieldError::new("items", "length", "must have 1 to 200 items"));
        }
        for (index, change) in changes.iter().enumerate() {
            let category = if let Some(key) = change.target.strip_prefix("kind:") {
                self.kinds.get(key).map(|kind| kind.category)
            } else {
                change.target.strip_prefix("category:").and_then(Category::parse)
            };
            match category {
                None => problems.push(FieldError::new(
                    format!("items[{index}].target"),
                    "invalid",
                    "must be `kind:<a declared kind>` or `category:<security|transactional|marketing>`",
                )),
                Some(Category::Security) if !change.enabled => locked = true,
                Some(_) => {}
            }
            if Channel::parse(&change.channel).is_none() {
                problems.push(FieldError::new(
                    format!("items[{index}].channel"),
                    "invalid",
                    "must be `email`, `sms`, `zns`, `web_push`, `in_app` or `mobile_push`",
                ));
            }
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        if locked {
            return Err(preference_locked());
        }
        let transaction = tenant.database().begin_write().await?;
        let at = now();
        for change in &changes {
            preference::Entity::insert(preference::ActiveModel {
                tenant_id: Set(tenant.id().as_uuid()),
                user_id: Set(principal.user_id()),
                target: Set(change.target.clone()),
                channel: Set(change.channel.clone()),
                enabled: Set(change.enabled),
                updated_at: Set(at),
            })
            .on_conflict(
                OnConflict::columns([
                    preference::Column::TenantId,
                    preference::Column::UserId,
                    preference::Column::Target,
                    preference::Column::Channel,
                ])
                .update_columns([preference::Column::Enabled, preference::Column::UpdatedAt])
                .to_owned(),
            )
            .exec_without_returning(&transaction)
            .await?;
        }
        let event = events::PreferencesUpdated(
            json!({"user_id": principal.user_id(), "changes": changes.len()}),
        );
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                Some(format!("notifications.preferences:{}", principal.user_id())),
                &event,
            )
            .await?;
        transaction.commit().await?;
        self.my_preferences(tenant, principal).await
    }

    // ---- Web push --------------------------------------------------------------------------------

    pub(crate) async fn add_push_subscription(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewPushSubscription,
    ) -> Result<push_subscription::Model, Error> {
        let endpoint = input.endpoint.trim().to_owned();
        // Địa chỉ do trình duyệt (tức người dùng) gửi; bộ gửi sẽ gọi tới nó: chỉ nhận máy chủ công khai
        // (PT-10) — không `localhost`, IP nội bộ, link-local (metadata của cloud).
        let valid = endpoint.to_ascii_lowercase().starts_with("https://")
            && autoxide_kernel::net::check_target_url(&endpoint, &["https"], false).is_ok()
            && endpoint.len() <= 2000
            && !endpoint.chars().any(char::is_control)
            && base64url(&input.p256dh, 80, 100)
            && base64url(&input.auth, 16, 32);
        if !valid {
            return Err(push_subscription_invalid());
        }
        let secret = self.secret.as_ref().ok_or_else(autoxide_kernel::push::web_push_disabled)?;
        let sealer = secret.sealer().await?;
        let key = autoxide_kernel::secret::sha256(&endpoint);
        let user_agent = input.user_agent.map(|agent| agent.chars().take(300).collect::<String>());
        let transaction = tenant.database().begin_write().await?;
        let existing = push_subscription::Entity::find()
            .scoped(tenant)
            .filter(push_subscription::Column::EndpointKey.eq(key.as_str()))
            .one(&transaction)
            .await?;
        let candidate = existing.as_ref().map_or_else(Uuid::now_v7, |row| row.id);
        let sealed = sealer.seal(
            tenant.id(),
            PUSH_AUTH_PURPOSE,
            &candidate.to_string(),
            input.auth.as_bytes(),
        )?;
        let at = now();
        // Cùng trình duyệt đăng ký lại (có thể bởi người khác trên cùng máy): cập nhật dòng cũ.
        push_subscription::Entity::insert(push_subscription::ActiveModel {
            id: Set(candidate),
            tenant_id: Set(tenant.id().as_uuid()),
            user_id: Set(principal.user_id()),
            endpoint: Set(endpoint),
            endpoint_key: Set(key.clone()),
            p256dh: Set(input.p256dh),
            auth_secret: Set(sealed),
            user_agent: Set(user_agent),
            failures: Set(0),
            created_at: Set(at),
            last_used_at: Set(None),
        })
        .on_conflict(
            OnConflict::columns([
                push_subscription::Column::TenantId,
                push_subscription::Column::EndpointKey,
            ])
            .update_columns([
                push_subscription::Column::UserId,
                push_subscription::Column::P256dh,
                push_subscription::Column::AuthSecret,
                push_subscription::Column::UserAgent,
                push_subscription::Column::Failures,
            ])
            .to_owned(),
        )
        .exec_without_returning(&transaction)
        .await?;
        // Dòng thật của endpoint này (D-181). Hai lần đăng ký cùng trình duyệt chạy cùng lúc đều
        // thấy "chưa có", mỗi bên sinh một ID; trên Postgres bên sau rơi vào nhánh cập nhật của dòng
        // bên trước — dòng giữ ID bên trước nhưng bản mã gắn ID bên sau, gửi push hỏng mãi. Bản mã
        // gắn ID nên niêm lại theo ID của dòng.
        let row = autoxide_kernel::rows::lock_one(
            &transaction,
            push_subscription::Entity::find()
                .scoped(tenant)
                .filter(push_subscription::Column::EndpointKey.eq(key.as_str())),
        )
        .await?
        .ok_or_else(|| Error::internal("a push subscription vanished"))?;
        let id = row.id;
        if id != candidate {
            let sealed = sealer.seal(
                tenant.id(),
                PUSH_AUTH_PURPOSE,
                &id.to_string(),
                input.auth.as_bytes(),
            )?;
            push_subscription::Entity::update_many()
                .col_expr(push_subscription::Column::AuthSecret, Expr::value(sealed))
                .filter(push_subscription::Column::Id.eq(id))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
        }
        let event = events::PushSubscriptionAdded(
            json!({"id": id, "user_id": principal.user_id(), "channel": Channel::WebPush.as_str()}),
        );
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                Some(format!("notifications.push_subscription:{id}")),
                &event,
            )
            .await?;
        let saved = push_subscription::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| Error::internal("a push subscription vanished"))?;
        transaction.commit().await?;
        Ok(saved)
    }

    /// Xoá đăng ký `id` của `user` (người dùng gỡ, hay dịch vụ đẩy báo đã mất). Không có: `false`.
    pub(crate) async fn remove_push_subscription(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
        user: Uuid,
        reason: &str,
    ) -> Result<bool, Error> {
        let transaction = tenant.database().begin_write().await?;
        let deleted = push_subscription::Entity::delete_many()
            .filter(push_subscription::Column::Id.eq(id))
            .filter(push_subscription::Column::UserId.eq(user))
            .scoped(tenant)
            .exec(&transaction)
            .await?
            .rows_affected;
        if deleted > 0 {
            let event = events::PushSubscriptionRemoved(
                json!({"id": id, "user_id": user, "channel": Channel::WebPush.as_str(), "reason": reason}),
            );
            self.outbox
                .record(
                    &transaction,
                    tenant,
                    actor,
                    Some(format!("notifications.push_subscription:{id}")),
                    &event,
                )
                .await?;
        }
        transaction.commit().await?;
        Ok(deleted > 0)
    }

    // ---- Thiết bị di động ------------------------------------------------------------------------

    pub(crate) async fn add_device_token(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewDeviceToken,
    ) -> Result<device_token::Model, Error> {
        let mut problems = Vec::new();
        if !["ios", "android"].contains(&input.platform.as_str()) {
            problems.push(FieldError::new("platform", "invalid", "must be `ios` or `android`"));
        }
        if input.token.is_empty()
            || input.token.len() > 4096
            || input.token.chars().any(char::is_control)
        {
            problems.push(FieldError::new("token", "length", "must be 1 to 4096 characters"));
        }
        if input.app_id.is_empty() || input.app_id.len() > 200 {
            problems.push(FieldError::new("app_id", "length", "must be 1 to 200 characters"));
        }
        let locale = match input.locale.as_deref() {
            None => None,
            Some(raw) => match autoxide_kernel::i18n::Language::from_tag(raw) {
                Some(language) => Some(language.tag().to_owned()),
                None => {
                    problems.push(FieldError::new("locale", "invalid", "must be `en` or `vi`"));
                    None
                }
            },
        };
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let secret =
            self.secret.as_ref().ok_or_else(autoxide_kernel::push::mobile_push_disabled)?;
        let sealer = secret.sealer().await?;
        let key = autoxide_kernel::secret::sha256(&input.token);
        let transaction = tenant.database().begin_write().await?;
        let existing = device_token::Entity::find()
            .scoped(tenant)
            .filter(device_token::Column::TokenKey.eq(key.as_str()))
            .one(&transaction)
            .await?;
        let candidate = existing.as_ref().map_or_else(Uuid::now_v7, |row| row.id);
        let sealed = sealer.seal(
            tenant.id(),
            DEVICE_TOKEN_PURPOSE,
            &candidate.to_string(),
            input.token.as_bytes(),
        )?;
        device_token::Entity::insert(device_token::ActiveModel {
            id: Set(candidate),
            tenant_id: Set(tenant.id().as_uuid()),
            user_id: Set(principal.user_id()),
            platform: Set(input.platform),
            token: Set(sealed),
            token_key: Set(key.clone()),
            app_id: Set(input.app_id),
            locale: Set(locale),
            failures: Set(0),
            created_at: Set(now()),
            last_used_at: Set(None),
        })
        .on_conflict(
            OnConflict::columns([device_token::Column::TenantId, device_token::Column::TokenKey])
                .update_columns([
                    device_token::Column::UserId,
                    device_token::Column::Platform,
                    device_token::Column::Token,
                    device_token::Column::AppId,
                    device_token::Column::Locale,
                    device_token::Column::Failures,
                ])
                .to_owned(),
        )
        .exec_without_returning(&transaction)
        .await?;
        // Dòng thật của token này: như đăng ký web push ở trên, hai lần đăng ký cùng lúc thì bản
        // mã phải gắn ID của dòng, không phải ID bên này đã sinh.
        let row = autoxide_kernel::rows::lock_one(
            &transaction,
            device_token::Entity::find()
                .scoped(tenant)
                .filter(device_token::Column::TokenKey.eq(key.as_str())),
        )
        .await?
        .ok_or_else(|| Error::internal("a device token vanished"))?;
        let id = row.id;
        if id != candidate {
            let sealed = sealer.seal(
                tenant.id(),
                DEVICE_TOKEN_PURPOSE,
                &id.to_string(),
                input.token.as_bytes(),
            )?;
            device_token::Entity::update_many()
                .col_expr(device_token::Column::Token, Expr::value(sealed))
                .filter(device_token::Column::Id.eq(id))
                .scoped(tenant)
                .exec(&transaction)
                .await?;
        }
        let event = events::PushSubscriptionAdded(
            json!({"id": id, "user_id": principal.user_id(), "channel": Channel::MobilePush.as_str()}),
        );
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                Some(format!("notifications.device_token:{id}")),
                &event,
            )
            .await?;
        let saved = device_token::Entity::find_by_id(id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| Error::internal("a device token vanished"))?;
        transaction.commit().await?;
        Ok(saved)
    }

    /// Thiết bị di động người gọi đã đăng ký, mới nhất trước (không trả token).
    pub(crate) async fn my_device_tokens(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
    ) -> Result<Vec<device_token::Model>, Error> {
        use sea_orm::QueryOrder;
        Ok(device_token::Entity::find()
            .scoped(tenant)
            .filter(device_token::Column::UserId.eq(principal.user_id()))
            .order_by_desc(device_token::Column::Id)
            .all(tenant.database().read())
            .await?)
    }

    pub(crate) async fn remove_device_token(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        id: Uuid,
        user: Uuid,
        reason: &str,
    ) -> Result<bool, Error> {
        let transaction = tenant.database().begin_write().await?;
        let deleted = device_token::Entity::delete_many()
            .filter(device_token::Column::Id.eq(id))
            .filter(device_token::Column::UserId.eq(user))
            .scoped(tenant)
            .exec(&transaction)
            .await?
            .rows_affected;
        if deleted > 0 {
            let event = events::PushSubscriptionRemoved(
                json!({"id": id, "user_id": user, "channel": Channel::MobilePush.as_str(), "reason": reason}),
            );
            self.outbox
                .record(
                    &transaction,
                    tenant,
                    actor,
                    Some(format!("notifications.device_token:{id}")),
                    &event,
                )
                .await?;
        }
        transaction.commit().await?;
        Ok(deleted > 0)
    }
}
