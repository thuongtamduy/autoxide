//! Service của module `notifications`: lỗi chung, chọn kênh và ghi tin vào hàng đợi (`notify`). Gửi
//! thật ở [`crate::delivery`]; hộp thư, tuỳ chọn, đăng ký đẩy ở [`crate::me`]; mẫu ở
//! [`crate::templates`]; thông báo của nền tảng ở [`crate::platform`].

use std::collections::HashMap;
use std::sync::Arc;

use autoxide_auth::api::{TenantAdmins, Users};
use autoxide_compliance::api::{ComplianceRules, ConsentKey, ConsentLedger, ConsentSubject};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Outbox;
use autoxide_kernel::i18n::{Language, Translations};
use autoxide_kernel::job::{EnqueueOptions, Job, JobQueue};
use autoxide_kernel::mail::Mailer;
use autoxide_kernel::push::{MobilePushSender, WebPushSender};
use autoxide_kernel::secret::SecretKey;
use autoxide_kernel::sms::{PhoneNumber, SmsSender};
use autoxide_kernel::tenancy::{DatabaseRegistry, ScopeExt, TenantAdmin, TenantContext};
use autoxide_kernel::throttle::Throttle;
use autoxide_kernel::zns::ZnsSender;
use chrono::{DateTime, SubsecRound, Utc};
use sea_orm::sea_query::OnConflict;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::api::{
    Category, Channel, Notification, NotificationKind, Outcome, Recipient, SkipReason, Skipped,
};
use crate::config::NotificationsConfig;
use crate::entity::{device_token, invitation, message, preference, push_subscription};
use crate::registry::Kinds;

/// Mã nước mặc định khi chuẩn hoá số điện thoại không có `+`.
pub(crate) const DEFAULT_COUNTRY_CODE: &str = "84";

pub(crate) fn now() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(6)
}

/// Job gửi một tin (payload chỉ ID).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Deliver {
    pub message_id: Uuid,
}

impl Job for Deliver {
    const KIND: &'static str = "notifications.deliver";
}

pub(crate) struct NotificationsService {
    pub databases: DatabaseRegistry,
    pub outbox: Outbox,
    pub jobs: JobQueue,
    pub kinds: Arc<Kinds>,
    pub translations: Translations,
    pub config: NotificationsConfig,
    pub throttle: Throttle,
    pub secret: Option<SecretKey>,
    pub users: Option<Arc<dyn Users>>,
    pub admins: Option<Arc<dyn TenantAdmins>>,
    pub tenants: Option<Arc<TenantAdmin>>,
    pub consents: Option<Arc<dyn ConsentLedger>>,
    pub rules: Option<Arc<dyn ComplianceRules>>,
    pub mailer: Option<Arc<dyn Mailer>>,
    pub sms: Option<Arc<dyn SmsSender>>,
    pub zns: Option<Arc<dyn ZnsSender>>,
    pub web_push: Option<Arc<dyn WebPushSender>>,
    pub mobile_push: Option<Arc<dyn MobilePushSender>>,
    /// Bộ dựng mẫu của kernel (mẫu khách tự soạn, D-206).
    pub templates: autoxide_kernel::template::TemplateEngine,
    /// Hồ sơ pháp lý của `compliance` (chân thư của mail HTML).
    pub legal: Option<Arc<dyn autoxide_compliance::api::LegalProfiles>>,
    /// Giới hạn gửi thử mẫu mỗi người mỗi giờ.
    pub test_throttle: Throttle,
}

// ---- Lỗi -----------------------------------------------------------------------------------------

pub(crate) fn kind_unknown(kind: &str) -> Error {
    Error::new(
        ErrorKind::Validation,
        "notifications.kind_unknown",
        "There is no notification kind `{kind}`; see `GET /api/v1/notifications/kinds`.",
    )
    .with_param("kind", kind)
}

pub(crate) fn recipient_unknown() -> Error {
    Error::new(
        ErrorKind::Validation,
        "notifications.recipient_unknown",
        "There is no user with this id in the tenant.",
    )
}

pub(crate) fn message_not_found() -> Error {
    Error::not_found("notifications.message_not_found", "There is no such message.")
}

pub(crate) fn link_invalid() -> Error {
    Error::bad_request("notifications.link_invalid", "The link has expired or is not valid.")
}

/// Thông báo có giá trị hợp lệ: khoá chống trùng, link, tham số.
fn check_notification(notification: &Notification) -> Result<(), Error> {
    let mut problems = Vec::new();
    if notification.dedupe_key.trim().is_empty() || notification.dedupe_key.len() > 160 {
        problems.push(FieldError::new("dedupe_key", "length", "must be 1 to 160 characters"));
    }
    if let Some(link) = &notification.link
        && !valid_link(link)
    {
        problems.push(FieldError::new(
            "link",
            "invalid",
            "must be an http(s) URL or a path starting with `/`, at most 2000 characters",
        ));
    }
    if !(notification.params.is_object() || notification.params.is_null())
        || notification.params.to_string().len() > 10_000
    {
        problems.push(FieldError::new(
            "params",
            "invalid",
            "must be an object of at most 10000 bytes",
        ));
    }
    if problems.is_empty() { Ok(()) } else { Err(Error::validation(problems)) }
}

pub(crate) fn valid_link(link: &str) -> bool {
    let lower = link.to_ascii_lowercase();
    link.len() <= 2000
        && !link.chars().any(char::is_control)
        && (lower.starts_with("https://")
            || lower.starts_with("http://")
            || (link.starts_with('/') && !link.starts_with("//")))
}

/// Lý do bỏ qua từ cột `skip_reason`.
pub(crate) fn parse_skip_reason(raw: &str) -> Option<SkipReason> {
    [
        SkipReason::NoConsent,
        SkipReason::ConsentWithdrawn,
        SkipReason::PreferenceOff,
        SkipReason::NoAddress,
        SkipReason::ChannelDisabled,
        SkipReason::QuietHoursExpired,
        SkipReason::InvitationUsed,
        SkipReason::Throttled,
    ]
    .into_iter()
    .find(|reason| reason.as_str() == raw)
}

/// Kết quả từ các dòng tin của một thông báo.
fn outcome_of(rows: &[message::Model]) -> Outcome {
    let mut outcome = Outcome::default();
    for row in rows {
        let channel = Channel::parse(&row.channel);
        match (row.status.as_str(), channel, parse_skip_reason(&row.skip_reason)) {
            ("skipped", Some(channel), Some(reason)) => {
                outcome.skipped.push(Skipped { channel, reason });
            }
            _ => outcome.message_ids.push(row.id),
        }
    }
    outcome
}

/// Đối tượng đồng ý của một tin quảng cáo: người dùng, hay địa chỉ của kênh.
pub(crate) fn consent_subject(
    user_id: Option<Uuid>,
    channel: Channel,
    address: Option<&str>,
) -> Option<ConsentSubject> {
    if let Some(user) = user_id {
        return Some(ConsentSubject::User(user));
    }
    let address = address?.to_owned();
    match channel {
        Channel::Email => Some(ConsentSubject::Email(address)),
        Channel::Sms | Channel::Zns => Some(ConsentSubject::Phone(address)),
        _ => None,
    }
}

/// Người nhận đã tra: người dùng (nếu có), email, số, ngôn ngữ ưa thích.
struct Resolved {
    user_id: Option<Uuid>,
    email: Option<String>,
    phone: Option<String>,
    locale: Option<String>,
    /// Khoá đếm giới hạn số tin mỗi giờ.
    throttle_key: String,
}

/// Một kênh đã quyết: địa chỉ và trạng thái ban đầu.
struct Planned {
    channel: Channel,
    address: Option<String>,
    skip: Option<SkipReason>,
}

impl NotificationsService {
    pub(crate) async fn tenant(
        &self,
        id: autoxide_kernel::tenancy::TenantId,
    ) -> Result<TenantContext, Error> {
        self.databases.tenant(id).await
    }

    /// Gửi `notification` cho khách `tenant`: chọn kênh, ghi tin + job trong một transaction.
    pub(crate) async fn notify(
        &self,
        tenant: &TenantContext,
        notification: Notification,
    ) -> Result<Outcome, Error> {
        let kind =
            self.kinds.get(&notification.kind).ok_or_else(|| kind_unknown(&notification.kind))?;
        check_notification(&notification)?;
        if notification.recipient != Recipient::TenantAdmins {
            return self.notify_one(tenant, &kind, notification).await;
        }
        // Mỗi quản trị viên một thông báo, khoá chống trùng riêng: chạy lại không gửi trùng ai.
        let mut outcome = Outcome::default();
        let Some(admins) = &self.admins else {
            return Ok(outcome);
        };
        for admin in admins.admins(tenant).await? {
            let mut one = notification.clone();
            one.recipient = Recipient::User(admin.id);
            one.dedupe_key = format!("{}:{}", notification.dedupe_key, admin.id);
            let got = self.notify_one(tenant, &kind, one).await?;
            outcome.message_ids.extend(got.message_ids);
            outcome.skipped.extend(got.skipped);
        }
        Ok(outcome)
    }

    async fn resolve(
        &self,
        tenant: &TenantContext,
        recipient: &Recipient,
    ) -> Result<Resolved, Error> {
        Ok(match recipient {
            Recipient::User(id) => {
                let (email, locale) = match &self.users {
                    Some(users) => {
                        let user = users.find(tenant, *id).await?.ok_or_else(recipient_unknown)?;
                        let email = user.is_active().then(|| user.email.clone());
                        (email, user.locale.clone())
                    }
                    None => (None, None),
                };
                Resolved {
                    user_id: Some(*id),
                    email,
                    phone: None,
                    locale,
                    throttle_key: format!("user:{id}"),
                }
            }
            Recipient::Email(raw) => {
                let email = raw.trim().to_lowercase();
                if !autoxide_kernel::email::is_valid(&email) {
                    return Err(Error::validation(vec![FieldError::new(
                        "recipient.email",
                        "invalid",
                        "must be an email address",
                    )]));
                }
                Resolved {
                    user_id: None,
                    throttle_key: format!("email:{email}"),
                    email: Some(email),
                    phone: None,
                    locale: None,
                }
            }
            Recipient::Phone(raw) => {
                let phone = PhoneNumber::parse(raw, DEFAULT_COUNTRY_CODE).ok_or_else(|| {
                    Error::validation(vec![FieldError::new(
                        "recipient.phone",
                        "invalid",
                        "must be a phone number, e.g. +84912345678",
                    )])
                })?;
                let phone = phone.as_str().to_owned();
                Resolved {
                    user_id: None,
                    throttle_key: format!("phone:{phone}"),
                    email: None,
                    phone: Some(phone),
                    locale: None,
                }
            }
            Recipient::TenantAdmins => {
                return Err(Error::internal("tenant admins are expanded before"));
            }
        })
    }

    /// Tuỳ chọn nhận của người dùng cho `kind`: (kênh → bật?) — dòng theo loại thắng dòng theo nhóm.
    async fn preferences_for(
        &self,
        tenant: &TenantContext,
        user: Uuid,
        kind: &NotificationKind,
    ) -> Result<HashMap<String, bool>, Error> {
        let kind_target = format!("kind:{}", kind.key());
        let category_target = format!("category:{}", kind.category.as_str());
        let rows = preference::Entity::find()
            .scoped(tenant)
            .filter(preference::Column::UserId.eq(user))
            .filter(preference::Column::Target.is_in([kind_target.clone(), category_target]))
            .all(tenant.database().read())
            .await?;
        let mut enabled = HashMap::new();
        for row in rows.iter().filter(|row| row.target != kind_target) {
            enabled.insert(row.channel.clone(), row.enabled);
        }
        for row in rows.iter().filter(|row| row.target == kind_target) {
            enabled.insert(row.channel.clone(), row.enabled);
        }
        Ok(enabled)
    }

    /// Kênh `channel` có bên gửi trên máy chủ này không.
    pub(crate) fn channel_available(&self, channel: Channel) -> bool {
        match channel {
            Channel::Email => self.mailer.is_some(),
            Channel::Sms => self.sms.is_some(),
            Channel::Zns => self.zns.is_some(),
            Channel::WebPush => self.web_push.is_some(),
            Channel::MobilePush => self.mobile_push.is_some(),
            Channel::InApp => true,
        }
    }

    async fn has_push(
        &self,
        tenant: &TenantContext,
        user: Uuid,
        channel: Channel,
    ) -> Result<bool, Error> {
        let read = tenant.database().read();
        let count = match channel {
            Channel::WebPush => {
                push_subscription::Entity::find()
                    .scoped(tenant)
                    .filter(push_subscription::Column::UserId.eq(user))
                    .count(read)
                    .await?
            }
            _ => {
                device_token::Entity::find()
                    .scoped(tenant)
                    .filter(device_token::Column::UserId.eq(user))
                    .count(read)
                    .await?
            }
        };
        Ok(count > 0)
    }

    /// Đồng ý quảng cáo còn hiệu lực cho (đối tượng, mục đích, kênh). Không có sổ đồng ý: không.
    pub(crate) async fn consented(
        &self,
        tenant: &TenantContext,
        purpose: &str,
        channel: Channel,
        subject: Option<ConsentSubject>,
    ) -> Result<bool, Error> {
        let (Some(ledger), Some(subject)) = (&self.consents, subject) else {
            return Ok(false);
        };
        let key = ConsentKey::new(purpose, "", channel.consent_channel());
        ledger.is_granted(tenant.id(), &subject, &key).await
    }

    async fn notify_one(
        &self,
        tenant: &TenantContext,
        kind: &NotificationKind,
        notification: Notification,
    ) -> Result<Outcome, Error> {
        let existing = self.messages_of(tenant, &notification.dedupe_key).await?;
        if !existing.is_empty() {
            return Ok(outcome_of(&existing));
        }
        let who = self.resolve(tenant, &notification.recipient).await?;
        let locale = notification
            .locale
            .as_deref()
            .or(who.locale.as_deref())
            .and_then(Language::from_tag)
            .unwrap_or(Language::En)
            .tag();
        let security = kind.category == Category::Security;
        let preferences = match (who.user_id, security) {
            (Some(user), false) => self.preferences_for(tenant, user, kind).await?,
            _ => HashMap::new(),
        };
        let mut plans = Vec::new();
        for channel in kind.channels.iter().copied() {
            let address = match channel {
                Channel::Email => who.email.clone(),
                Channel::Sms | Channel::Zns => who.phone.clone(),
                _ => None,
            };
            let reachable = match channel {
                Channel::Email | Channel::Sms | Channel::Zns => address.is_some(),
                Channel::InApp => who.user_id.is_some(),
                Channel::WebPush | Channel::MobilePush => match who.user_id {
                    Some(user) => self.has_push(tenant, user, channel).await?,
                    None => false,
                },
            };
            let skip = if preferences.get(channel.as_str()) == Some(&false) {
                Some(SkipReason::PreferenceOff)
            } else if !reachable {
                Some(SkipReason::NoAddress)
            } else if !self.channel_available(channel) {
                Some(SkipReason::ChannelDisabled)
            } else if kind.category == Category::Marketing
                && !self
                    .consented(
                        tenant,
                        kind.consent_purpose(),
                        channel,
                        consent_subject(who.user_id, channel, address.as_deref()),
                    )
                    .await?
            {
                Some(SkipReason::NoConsent)
            } else {
                None
            };
            plans.push(Planned { channel, address, skip });
        }
        // Giới hạn mỗi giờ chỉ đếm khi có kênh sẽ gửi; tin bảo mật không bị giới hạn.
        if !security
            && plans.iter().any(|plan| plan.skip.is_none())
            && !self.throttle.hit(tenant, &who.throttle_key).await?.is_allowed()
        {
            for plan in plans.iter_mut().filter(|plan| plan.skip.is_none()) {
                plan.skip = Some(SkipReason::Throttled);
            }
        }
        let params = if notification.params.is_null() {
            "{}".to_owned()
        } else {
            notification.params.to_string()
        };
        let purpose =
            if kind.category == Category::Marketing { kind.consent_purpose() } else { "" };
        let transaction = tenant.database().begin_write().await?;
        let at = now();
        for plan in &mut plans {
            if kind.invitation && plan.skip.is_none() {
                let subject = match (&plan.address, who.user_id) {
                    (Some(address), _) => address.clone(),
                    (None, Some(user)) => user.to_string(),
                    (None, None) => String::new(),
                };
                let fresh = invitation::Entity::insert(invitation::ActiveModel {
                    tenant_id: Set(tenant.id().as_uuid()),
                    channel: Set(plan.channel.as_str().to_owned()),
                    subject_key: Set(autoxide_kernel::secret::sha256(&format!(
                        "notifications-invitation|{subject}"
                    ))),
                    sent_at: Set(at),
                })
                .on_conflict(
                    OnConflict::columns([
                        invitation::Column::TenantId,
                        invitation::Column::Channel,
                        invitation::Column::SubjectKey,
                    ])
                    .do_nothing()
                    .to_owned(),
                )
                .exec_without_returning(&transaction)
                .await?;
                if fresh == 0 {
                    plan.skip = Some(SkipReason::InvitationUsed);
                }
            }
            let id = Uuid::now_v7();
            let inserted = message::Entity::insert(message::ActiveModel {
                id: Set(id),
                tenant_id: Set(tenant.id().as_uuid()),
                kind: Set(kind.key().to_owned()),
                category: Set(kind.category.as_str().to_owned()),
                channel: Set(plan.channel.as_str().to_owned()),
                user_id: Set(who.user_id),
                address: Set(plan.address.clone()),
                purpose: Set(purpose.to_owned()),
                locale: Set(locale.to_owned()),
                params: Set(params.clone()),
                link: Set(notification.link.clone()),
                status: Set(if plan.skip.is_some() { "skipped" } else { "queued" }.to_owned()),
                skip_reason: Set(plan.skip.map_or("", SkipReason::as_str).to_owned()),
                not_before: Set(None),
                attempts: Set(0),
                provider_reference: Set(None),
                error_code: Set(None),
                unsubscribe_token_hash: Set(None),
                dedupe_key: Set(notification.dedupe_key.clone()),
                created_at: Set(at),
                sent_at: Set(None),
            })
            .on_conflict(
                OnConflict::columns([
                    message::Column::TenantId,
                    message::Column::DedupeKey,
                    message::Column::Channel,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec_without_returning(&transaction)
            .await?;
            if inserted > 0 && plan.skip.is_none() {
                self.jobs
                    .enqueue(
                        &transaction,
                        tenant,
                        &Deliver { message_id: id },
                        EnqueueOptions::default().unique_key(format!("deliver:{id}")),
                    )
                    .await?;
            }
        }
        transaction.commit().await?;
        let rows = self.messages_of(tenant, &notification.dedupe_key).await?;
        Ok(outcome_of(&rows))
    }

    async fn messages_of(
        &self,
        tenant: &TenantContext,
        dedupe_key: &str,
    ) -> Result<Vec<message::Model>, Error> {
        Ok(message::Entity::find()
            .scoped(tenant)
            .filter(message::Column::DedupeKey.eq(dedupe_key))
            .all(tenant.database().write())
            .await?)
    }

    /// Tham số của mẫu từ JSON đã lưu: chữ giữ nguyên, số và đúng / sai thành chữ.
    pub(crate) fn params_of(raw: &str) -> Vec<(String, String)> {
        let Ok(Value::Object(map)) = serde_json::from_str::<Value>(raw) else {
            return Vec::new();
        };
        map.into_iter()
            .filter_map(|(name, value)| match value {
                Value::String(text) => Some((name, text)),
                Value::Number(number) => Some((name, number.to_string())),
                Value::Bool(flag) => Some((name, flag.to_string())),
                _ => None,
            })
            .collect()
    }
}
