//! Gửi tin (job `notifications.deliver`), khung giờ gửi tin quảng cáo, nhãn quảng cáo, dọn nhật ký
//! (job `notifications.purge`) và huỷ tin quảng cáo khi người nhận rút đồng ý.
//!
//! Căn cứ (cần đối chiếu văn bản gốc / người có chuyên môn pháp lý): NĐ 91/2020/NĐ-CP về chống tin
//! nhắn rác, thư điện tử rác, cuộc gọi rác — tin quảng cáo chỉ gửi khi người nhận đã đồng ý trước,
//! trong khung giờ cho phép, có nhãn quảng cáo và cách từ chối nhận. Khung giờ và nhãn là quy tắc có
//! ngày hiệu lực của `compliance` (`notifications.send_windows`, `notifications.marketing_labels`),
//! không viết cứng.

use std::sync::Arc;

use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::event::{Actor, Delivered, EventHandler};
use autoxide_kernel::i18n::Language;
use autoxide_kernel::job::{EnqueueOptions, JobContext, JobHandler};
use autoxide_kernel::mail::{Mail, MailTemplate};
use autoxide_kernel::push::{DeviceToken, PushContent, PushDelivery, WebPushSubscription};
use autoxide_kernel::sms::{PhoneNumber, SmsMessage, SmsPurpose};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::zns::ZnsMessage;
use chrono::{DateTime, Duration, Timelike, Utc};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect, Set};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::{Category, Channel, NotificationKind, SkipReason};
use crate::entity::{device_token, inbox, message, push_subscription, template};
use crate::service::{DEFAULT_COUNTRY_CODE, Deliver, NotificationsService, consent_subject, now};
use crate::templates::Rendered;

/// Quy tắc khung giờ gửi tin quảng cáo.
pub(crate) const SEND_WINDOWS: &str = "notifications.send_windows";
/// Quy tắc nhãn quảng cáo.
pub(crate) const MARKETING_LABELS: &str = "notifications.marketing_labels";
/// Mục đích mã hoá secret `auth` của đăng ký web push.
pub(crate) const PUSH_AUTH_PURPOSE: &str = "notifications.push_auth";
/// Mục đích mã hoá token thiết bị.
pub(crate) const DEVICE_TOKEN_PURPOSE: &str = "notifications.device_token";

/// Chân thư quảng cáo: link rút đồng ý. Bộ dựng tự chèn, mẫu không gỡ được.
pub(crate) const UNSUBSCRIBE_FOOTER: MailTemplate = MailTemplate::new(
    "notifications.mail.unsubscribe_footer",
    "Unsubscribe",
    "You get this advertising because you agreed to it. To stop, open {url}",
);

/// Tin quảng cáo chưa gửi được trong khung giờ sau chừng này giờ thì bỏ.
const QUIET_HOURS_LIMIT_HOURS: i64 = 24;

/// Kết quả gửi một tin.
enum Sent {
    Done(Option<String>),
    Skipped(SkipReason),
    Failed(String),
}

/// `HH:MM` → số phút trong ngày.
fn minutes(raw: &str) -> Option<i64> {
    let (hours, mins) = raw.split_once(':')?;
    let (hours, mins) = (hours.parse::<i64>().ok()?, mins.parse::<i64>().ok()?);
    ((0..24).contains(&hours) && (0..60).contains(&mins)).then_some(hours * 60 + mins)
}

/// Khoá kênh trong quy tắc (`email`, `sms`, `zns`, `push`); in-app không có khung giờ.
fn rule_channel(channel: Channel) -> Option<&'static str> {
    match channel {
        Channel::InApp => None,
        other => Some(other.consent_channel()),
    }
}

/// Lúc khung giờ mở lại, nếu `at` đang ngoài khung `window` (`{"from": "07:00", "to": "22:00"}`)
/// theo giờ địa phương lệch `offset` phút. `None`: đang trong khung, hay không có khung.
pub(crate) fn next_opening(
    window: &Value,
    offset: i64,
    at: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let from = minutes(window.get("from")?.as_str()?)?;
    let to = minutes(window.get("to")?.as_str()?)?;
    let local = at + Duration::minutes(offset);
    let now_minutes = i64::from(local.hour()) * 60 + i64::from(local.minute());
    let inside = if from <= to {
        (from..to).contains(&now_minutes)
    } else {
        // Khung qua nửa đêm (vd 20:00–06:00).
        now_minutes >= from || now_minutes < to
    };
    if inside {
        return None;
    }
    let midnight = local.date_naive().and_hms_opt(0, 0, 0)?.and_utc();
    let mut opening = midnight + Duration::minutes(from);
    if opening <= local {
        opening += Duration::days(1);
    }
    Some(opening - Duration::minutes(offset))
}

impl NotificationsService {
    /// Thân quy tắc `key` đang hiệu lực cho khách (không có `compliance` hay quy tắc: `None`).
    async fn rule(
        &self,
        tenant: &TenantContext,
        key: &str,
        at: DateTime<Utc>,
    ) -> Result<Option<Value>, Error> {
        let Some(rules) = &self.rules else {
            return Ok(None);
        };
        Ok(rules.effective(tenant.id(), key, at).await?.map(|rule| rule.body))
    }

    /// Lúc mở khung giờ kế tiếp cho tin quảng cáo trên `channel`, nếu `at` đang ngoài khung.
    async fn marketing_window(
        &self,
        tenant: &TenantContext,
        channel: Channel,
        at: DateTime<Utc>,
    ) -> Result<Option<DateTime<Utc>>, Error> {
        let (Some(key), Some(body)) =
            (rule_channel(channel), self.rule(tenant, SEND_WINDOWS, at).await?)
        else {
            return Ok(None);
        };
        let offset = body.get("utc_offset_minutes").and_then(Value::as_i64).unwrap_or(0);
        Ok(body
            .get("marketing")
            .and_then(|marketing| marketing.get(key))
            .filter(|window| !window.is_null())
            .and_then(|window| next_opening(window, offset, at)))
    }

    /// Nhãn quảng cáo của kênh: (`prefix` cho tin ngắn / đẩy, `subject_prefix` cho email).
    async fn marketing_label(
        &self,
        tenant: &TenantContext,
        channel: Channel,
        at: DateTime<Utc>,
    ) -> Result<String, Error> {
        let Some(key) = rule_channel(channel) else {
            return Ok(String::new());
        };
        let field = if channel == Channel::Email { "subject_prefix" } else { "prefix" };
        Ok(self
            .rule(tenant, MARKETING_LABELS, at)
            .await?
            .and_then(|body| body.get(key)?.get(field)?.as_str().map(str::to_owned))
            .unwrap_or_default())
    }

    /// Ghi kết quả cuối của tin (chỉ khi tin còn đang chờ: chạy lại không ghi đè).
    async fn finish(
        &self,
        tenant: &TenantContext,
        row: &message::Model,
        sent: Sent,
    ) -> Result<(), Error> {
        let at = now();
        let mut update = message::Entity::update_many()
            .col_expr(message::Column::Attempts, Expr::value(row.attempts + 1))
            .filter(message::Column::Id.eq(row.id))
            .filter(message::Column::Status.is_in(["queued", "deferred"]))
            .scoped(tenant);
        update = match sent {
            Sent::Done(reference) => update
                .col_expr(message::Column::Status, Expr::value("sent"))
                .col_expr(message::Column::SentAt, Expr::value(Some(at)))
                .col_expr(message::Column::ProviderReference, Expr::value(reference)),
            Sent::Skipped(reason) => update
                .col_expr(message::Column::Status, Expr::value("skipped"))
                .col_expr(message::Column::SkipReason, Expr::value(reason.as_str())),
            Sent::Failed(code) => {
                tracing::warn!(message_id = %row.id, channel = row.channel.as_str(), error_code = code.as_str(), "notification failed");
                update
                    .col_expr(message::Column::Status, Expr::value("failed"))
                    .col_expr(message::Column::ErrorCode, Expr::value(Some(code)))
            }
        };
        update.exec(tenant.database().write()).await?;
        Ok(())
    }

    /// Gửi tin `id` của khách `tenant` lúc `at`. Tin đã xong (gửi, bỏ, huỷ) thì không làm gì.
    pub(crate) async fn deliver(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        at: DateTime<Utc>,
    ) -> Result<(), Error> {
        let Some(row) =
            message::Entity::find_by_id(id).scoped(tenant).one(tenant.database().write()).await?
        else {
            return Ok(());
        };
        if !["queued", "deferred"].contains(&row.status.as_str()) {
            return Ok(());
        }
        let (Some(kind), Some(channel)) = (self.kinds.get(&row.kind), Channel::parse(&row.channel))
        else {
            return self.finish(tenant, &row, Sent::Failed("kind_unknown".to_owned())).await;
        };
        let marketing = kind.category == Category::Marketing;
        if marketing {
            // Kiểm lại đồng ý ngay trước khi gọi nhà cung cấp: người nhận có thể vừa rút.
            let subject = consent_subject(row.user_id, channel, row.address.as_deref());
            if !self.consented(tenant, &row.purpose, channel, subject).await? {
                return self
                    .finish(tenant, &row, Sent::Skipped(SkipReason::ConsentWithdrawn))
                    .await;
            }
            if let Some(opening) = self.marketing_window(tenant, channel, at).await? {
                return self.defer(tenant, &row, opening).await;
            }
        }
        let rendered = self.render(tenant, &kind, &row).await?;
        let result = match channel {
            Channel::Email => self.send_email(tenant, &kind, &row, &rendered, at).await,
            Channel::Sms => self.send_sms(tenant, &kind, &row, &rendered, at).await,
            Channel::Zns => self.send_zns(tenant, &row).await,
            Channel::WebPush => self.send_web_push(tenant, &kind, &row, &rendered, at).await,
            Channel::MobilePush => self.send_mobile_push(tenant, &kind, &row, &rendered, at).await,
            Channel::InApp => self.send_in_app(tenant, &kind, &row, &rendered).await,
        };
        match result {
            Ok(sent) => self.finish(tenant, &row, sent).await,
            // Lỗi vĩnh viễn (địa chỉ sai, bị chặn, mẫu chưa duyệt): không thử lại.
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::Validation
                        | ErrorKind::BadRequest
                        | ErrorKind::NotFound
                        | ErrorKind::Forbidden
                ) =>
            {
                self.finish(tenant, &row, Sent::Failed(error.code().to_owned())).await
            }
            // Lỗi tạm thời: job queue thử lại với backoff.
            Err(error) => {
                message::Entity::update_many()
                    .col_expr(message::Column::Attempts, Expr::value(row.attempts + 1))
                    .filter(message::Column::Id.eq(row.id))
                    .scoped(tenant)
                    .exec(tenant.database().write())
                    .await?;
                Err(error)
            }
        }
    }

    /// Ngoài khung giờ: hoãn tới lúc mở khung (quá 24 giờ kể từ lúc tạo thì bỏ).
    async fn defer(
        &self,
        tenant: &TenantContext,
        row: &message::Model,
        opening: DateTime<Utc>,
    ) -> Result<(), Error> {
        if opening - row.created_at > Duration::hours(QUIET_HOURS_LIMIT_HOURS) {
            return self.finish(tenant, row, Sent::Skipped(SkipReason::QuietHoursExpired)).await;
        }
        let transaction = tenant.database().begin_write().await?;
        message::Entity::update_many()
            .col_expr(message::Column::Status, Expr::value("deferred"))
            .col_expr(message::Column::NotBefore, Expr::value(Some(opening)))
            .filter(message::Column::Id.eq(row.id))
            .filter(message::Column::Status.is_in(["queued", "deferred"]))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        self.jobs
            .enqueue(
                &transaction,
                tenant,
                &Deliver { message_id: row.id },
                EnqueueOptions::default().run_at(opening).unique_key(format!(
                    "deliver:{}:{}",
                    row.id,
                    opening.timestamp()
                )),
            )
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn send_email(
        &self,
        tenant: &TenantContext,
        kind: &NotificationKind,
        row: &message::Model,
        rendered: &Rendered,
        at: DateTime<Utc>,
    ) -> Result<Sent, Error> {
        let Some(mailer) = &self.mailer else {
            return Ok(Sent::Skipped(SkipReason::ChannelDisabled));
        };
        let Some(address) = &row.address else {
            return Ok(Sent::Skipped(SkipReason::NoAddress));
        };
        let mut subject = rendered.title.clone();
        let mut text = rendered.body.clone();
        if let Some(link) = &row.link {
            text = format!("{text}\n\n{link}");
        }
        let mut headers = Vec::new();
        if kind.category == Category::Marketing {
            let Some(page) = &self.config.unsubscribe_url else {
                return Ok(Sent::Failed("unsubscribe_url_missing".to_owned()));
            };
            let raw = autoxide_kernel::secret::random_token(32)?;
            message::Entity::update_many()
                .col_expr(
                    message::Column::UnsubscribeTokenHash,
                    Expr::value(Some(autoxide_kernel::secret::sha256(&raw))),
                )
                .filter(message::Column::Id.eq(row.id))
                .scoped(tenant)
                .exec(tenant.database().write())
                .await?;
            let token = autoxide_kernel::tenancy::link_token(tenant, &raw);
            let url = page.replace("{token}", &token);
            let language = Language::from_tag(&row.locale).unwrap_or(Language::En);
            let footer =
                UNSUBSCRIBE_FOOTER.render(&self.translations, language, &[("url", url.as_str())]);
            subject =
                format!("{}{subject}", self.marketing_label(tenant, Channel::Email, at).await?);
            text = format!("{text}\n\n--\n{}", footer.text);
            headers.push(("List-Unsubscribe".to_owned(), format!("<{url}>")));
        }
        let mut mail = Mail::new(address.clone(), subject, text)
            .dedupe_key(format!("notifications:{}", row.id));
        for (name, value) in headers {
            mail = mail.header(name, value);
        }
        mailer.send(tenant, mail).await?;
        Ok(Sent::Done(None))
    }

    async fn send_sms(
        &self,
        tenant: &TenantContext,
        kind: &NotificationKind,
        row: &message::Model,
        rendered: &Rendered,
        at: DateTime<Utc>,
    ) -> Result<Sent, Error> {
        let Some(sender) = &self.sms else {
            return Ok(Sent::Skipped(SkipReason::ChannelDisabled));
        };
        let Some(phone) =
            row.address.as_deref().and_then(|raw| PhoneNumber::parse(raw, DEFAULT_COUNTRY_CODE))
        else {
            return Ok(Sent::Skipped(SkipReason::NoAddress));
        };
        let (purpose, label) = match kind.category {
            Category::Marketing => {
                (SmsPurpose::Marketing, self.marketing_label(tenant, Channel::Sms, at).await?)
            }
            _ => (SmsPurpose::Transactional, String::new()),
        };
        let text = format!("{label}{}", rendered.short);
        let receipt = sender
            .send(SmsMessage::new(tenant.id(), phone, text, purpose, row.id.to_string()))
            .await?;
        Ok(Sent::Done(Some(receipt.provider_reference)))
    }

    async fn send_zns(&self, tenant: &TenantContext, row: &message::Model) -> Result<Sent, Error> {
        let Some(sender) = &self.zns else {
            return Ok(Sent::Skipped(SkipReason::ChannelDisabled));
        };
        let Some(phone) =
            row.address.as_deref().and_then(|raw| PhoneNumber::parse(raw, DEFAULT_COUNTRY_CODE))
        else {
            return Ok(Sent::Skipped(SkipReason::NoAddress));
        };
        // Mã mẫu ZNS do khách đăng ký với Zalo: nằm ở mẫu thay thế của khách.
        let found = template::Entity::find()
            .scoped(tenant)
            .filter(template::Column::Kind.eq(row.kind.as_str()))
            .filter(template::Column::Channel.eq(Channel::Zns.as_str()))
            .all(tenant.database().read())
            .await?;
        let chosen = found
            .iter()
            .find(|item| item.locale == row.locale)
            .or_else(|| found.first())
            .and_then(|item| item.zns_template_id.clone());
        let Some(template_id) = chosen else {
            return Ok(Sent::Failed("zns_template_missing".to_owned()));
        };
        let params: Value = serde_json::from_str(&row.params).unwrap_or_else(|_| json!({}));
        let reference = sender
            .send(ZnsMessage::new(tenant.id(), phone, template_id, params, row.id.to_string()))
            .await?;
        Ok(Sent::Done(Some(reference)))
    }

    async fn push_content(
        &self,
        tenant: &TenantContext,
        kind: &NotificationKind,
        channel: Channel,
        row: &message::Model,
        rendered: &Rendered,
        at: DateTime<Utc>,
    ) -> Result<PushContent, Error> {
        let label = if kind.category == Category::Marketing {
            self.marketing_label(tenant, channel, at).await?
        } else {
            String::new()
        };
        Ok(PushContent::new(
            format!("{label}{}", rendered.title),
            rendered.short.clone(),
            row.link.clone(),
            row.id.to_string(),
        ))
    }

    async fn send_web_push(
        &self,
        tenant: &TenantContext,
        kind: &NotificationKind,
        row: &message::Model,
        rendered: &Rendered,
        at: DateTime<Utc>,
    ) -> Result<Sent, Error> {
        let (Some(sender), Some(secret)) = (&self.web_push, &self.secret) else {
            return Ok(Sent::Skipped(SkipReason::ChannelDisabled));
        };
        let Some(user) = row.user_id else {
            return Ok(Sent::Skipped(SkipReason::NoAddress));
        };
        let subscriptions = push_subscription::Entity::find()
            .scoped(tenant)
            .filter(push_subscription::Column::UserId.eq(user))
            .limit(20)
            .all(tenant.database().read())
            .await?;
        let content = self.push_content(tenant, kind, Channel::WebPush, row, rendered, at).await?;
        let sealer = secret.sealer().await?;
        let mut reference = None;
        for item in subscriptions {
            let auth = sealer.open(
                tenant.id(),
                PUSH_AUTH_PURPOSE,
                &item.id.to_string(),
                &item.auth_secret,
            )?;
            let auth = String::from_utf8_lossy(&auth).into_owned();
            let target = WebPushSubscription::new(item.endpoint.clone(), item.p256dh.clone(), auth);
            match sender.send(tenant.id(), &target, &content).await? {
                PushDelivery::Sent { provider_reference } => {
                    reference.get_or_insert(provider_reference);
                    push_subscription::Entity::update_many()
                        .col_expr(push_subscription::Column::LastUsedAt, Expr::value(Some(now())))
                        .filter(push_subscription::Column::Id.eq(item.id))
                        .scoped(tenant)
                        .exec(tenant.database().write())
                        .await?;
                }
                PushDelivery::Gone => {
                    self.remove_push_subscription(
                        tenant,
                        Actor::system(),
                        item.id,
                        item.user_id,
                        "gone",
                    )
                    .await?;
                }
            }
        }
        Ok(match reference {
            Some(reference) => Sent::Done(Some(reference)),
            None => Sent::Skipped(SkipReason::NoAddress),
        })
    }

    async fn send_mobile_push(
        &self,
        tenant: &TenantContext,
        kind: &NotificationKind,
        row: &message::Model,
        rendered: &Rendered,
        at: DateTime<Utc>,
    ) -> Result<Sent, Error> {
        let (Some(sender), Some(secret)) = (&self.mobile_push, &self.secret) else {
            return Ok(Sent::Skipped(SkipReason::ChannelDisabled));
        };
        let Some(user) = row.user_id else {
            return Ok(Sent::Skipped(SkipReason::NoAddress));
        };
        let devices = device_token::Entity::find()
            .scoped(tenant)
            .filter(device_token::Column::UserId.eq(user))
            .limit(20)
            .all(tenant.database().read())
            .await?;
        let content =
            self.push_content(tenant, kind, Channel::MobilePush, row, rendered, at).await?;
        let sealer = secret.sealer().await?;
        let mut reference = None;
        for item in devices {
            let token = sealer.open(
                tenant.id(),
                DEVICE_TOKEN_PURPOSE,
                &item.id.to_string(),
                &item.token,
            )?;
            let token = String::from_utf8_lossy(&token).into_owned();
            let device = DeviceToken::new(item.platform.clone(), token, item.app_id.clone());
            match sender.send(tenant.id(), &device, &content).await? {
                PushDelivery::Sent { provider_reference } => {
                    reference.get_or_insert(provider_reference);
                    device_token::Entity::update_many()
                        .col_expr(device_token::Column::LastUsedAt, Expr::value(Some(now())))
                        .filter(device_token::Column::Id.eq(item.id))
                        .scoped(tenant)
                        .exec(tenant.database().write())
                        .await?;
                }
                PushDelivery::Gone => {
                    self.remove_device_token(
                        tenant,
                        Actor::system(),
                        item.id,
                        item.user_id,
                        "gone",
                    )
                    .await?;
                }
            }
        }
        Ok(match reference {
            Some(reference) => Sent::Done(Some(reference)),
            None => Sent::Skipped(SkipReason::NoAddress),
        })
    }

    async fn send_in_app(
        &self,
        tenant: &TenantContext,
        kind: &NotificationKind,
        row: &message::Model,
        rendered: &Rendered,
    ) -> Result<Sent, Error> {
        let Some(user) = row.user_id else {
            return Ok(Sent::Skipped(SkipReason::NoAddress));
        };
        // ID của dòng hộp thư = ID của tin: chạy lại không tạo dòng thứ hai.
        inbox::Entity::insert(inbox::ActiveModel {
            id: Set(row.id),
            tenant_id: Set(tenant.id().as_uuid()),
            user_id: Set(user),
            kind: Set(row.kind.clone()),
            category: Set(kind.category.as_str().to_owned()),
            title: Set(rendered.title.chars().take(300).collect()),
            body: Set(rendered.body.clone()),
            link: Set(row.link.clone()),
            read_at: Set(None),
            created_at: Set(row.created_at),
        })
        .on_conflict(OnConflict::column(inbox::Column::Id).do_nothing().to_owned())
        .exec_without_returning(tenant.database().write())
        .await?;
        Ok(Sent::Done(None))
    }

    /// Huỷ tin quảng cáo đang chờ của khách khi một đồng ý bị rút: mỗi tin cùng mục đích (và kênh,
    /// nếu đồng ý gắn kênh) được kiểm lại; đối tượng không còn đồng ý → `canceled`. Gọi lại vô hại.
    pub(crate) async fn cancel_withdrawn(
        &self,
        tenant: &TenantContext,
        purpose: &str,
        consent_channel: &str,
    ) -> Result<u64, Error> {
        let pending = message::Entity::find()
            .scoped(tenant)
            .filter(message::Column::Status.is_in(["queued", "deferred"]))
            .filter(message::Column::Category.eq(Category::Marketing.as_str()))
            .filter(message::Column::Purpose.eq(purpose))
            .limit(1000)
            .all(tenant.database().read())
            .await?;
        let mut canceled = 0;
        for row in pending {
            let Some(channel) = Channel::parse(&row.channel) else {
                continue;
            };
            if !consent_channel.is_empty() && channel.consent_channel() != consent_channel {
                continue;
            }
            let subject = consent_subject(row.user_id, channel, row.address.as_deref());
            if self.consented(tenant, purpose, channel, subject).await? {
                continue;
            }
            canceled += message::Entity::update_many()
                .col_expr(message::Column::Status, Expr::value("canceled"))
                .col_expr(
                    message::Column::SkipReason,
                    Expr::value(SkipReason::ConsentWithdrawn.as_str()),
                )
                .filter(message::Column::Id.eq(row.id))
                .filter(message::Column::Status.is_in(["queued", "deferred"]))
                .scoped(tenant)
                .exec(tenant.database().write())
                .await?
                .rows_affected;
        }
        if canceled > 0 {
            tracing::info!(
                canceled,
                purpose,
                "notifications canceled after a consent was withdrawn"
            );
        }
        Ok(canceled)
    }

    /// Dọn theo hạn lưu trên cả database (mọi khách): xoá địa chỉ và tham số của tin cũ, xoá hộp thư
    /// quá hạn.
    pub(crate) async fn purge(
        &self,
        database: &autoxide_kernel::database::Database,
        at: DateTime<Utc>,
    ) -> Result<(u64, u64), Error> {
        let write = database.write();
        let blanked = message::Entity::update_many()
            .col_expr(message::Column::Address, Expr::value(Option::<String>::None))
            .col_expr(message::Column::Params, Expr::value("{}"))
            .col_expr(message::Column::Link, Expr::value(Option::<String>::None))
            .filter(message::Column::CreatedAt.lt(at - Duration::days(self.config.retention_days)))
            .filter(message::Column::Address.is_not_null())
            .exec(write)
            .await?
            .rows_affected;
        let deleted = inbox::Entity::delete_many()
            .filter(
                inbox::Column::CreatedAt.lt(at - Duration::days(self.config.inbox_retention_days)),
            )
            .exec(write)
            .await?
            .rows_affected;
        Ok((blanked, deleted))
    }
}

/// Bộ chạy job `notifications.deliver`.
pub(crate) struct DeliverJob(pub Arc<NotificationsService>);

#[async_trait::async_trait]
impl JobHandler for DeliverJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let Some(tenant) = &job.tenant else {
            return Ok(());
        };
        let Some(id) =
            job.payload.get("message_id").and_then(Value::as_str).and_then(|raw| raw.parse().ok())
        else {
            return Ok(());
        };
        self.0.deliver(tenant, id, now()).await
    }
}

pub(crate) const PURGE: &str = "notifications.purge";

/// Bộ chạy lịch dọn hằng ngày.
pub(crate) struct PurgeJob(pub Arc<NotificationsService>);

#[async_trait::async_trait]
impl JobHandler for PurgeJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let (blanked, deleted) = self.0.purge(&job.database, now()).await?;
        tracing::info!(blanked, deleted, "notifications purged");
        Ok(())
    }
}

/// Nghe `compliance.consent_withdrawn`: huỷ tin quảng cáo đang chờ của người vừa rút đồng ý.
pub(crate) struct ConsentWithdrawnListener(pub Arc<NotificationsService>);

#[async_trait::async_trait]
impl EventHandler for ConsentWithdrawnListener {
    async fn handle(&self, event: &Delivered) -> Result<(), Error> {
        let purpose = event.payload.get("purpose").and_then(Value::as_str).unwrap_or_default();
        let channel = event.payload.get("channel").and_then(Value::as_str).unwrap_or_default();
        if purpose.is_empty() {
            return Ok(());
        }
        self.0.cancel_withdrawn(&event.tenant, purpose, channel).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn khung_gio_theo_gio_viet_nam() {
        let window = json!({"from": "07:00", "to": "22:00"});
        // 23:30 giờ Việt Nam (16:30 UTC) → mở lại 07:00 hôm sau (00:00 UTC).
        let late = Utc.with_ymd_and_hms(2026, 11, 17, 16, 30, 0).unwrap();
        assert_eq!(
            next_opening(&window, 420, late),
            Some(Utc.with_ymd_and_hms(2026, 11, 18, 0, 0, 0).unwrap())
        );
        // 06:00 giờ Việt Nam (23:00 UTC hôm trước) → 07:00 cùng ngày.
        let early = Utc.with_ymd_and_hms(2026, 11, 16, 23, 0, 0).unwrap();
        assert_eq!(
            next_opening(&window, 420, early),
            Some(Utc.with_ymd_and_hms(2026, 11, 17, 0, 0, 0).unwrap())
        );
        // 10:00 giờ Việt Nam: trong khung.
        let inside = Utc.with_ymd_and_hms(2026, 11, 17, 3, 0, 0).unwrap();
        assert_eq!(next_opening(&window, 420, inside), None);
        // Khung qua nửa đêm.
        let night = json!({"from": "20:00", "to": "06:00"});
        assert_eq!(
            next_opening(&night, 0, Utc.with_ymd_and_hms(2026, 1, 1, 21, 0, 0).unwrap()),
            None
        );
        assert!(next_opening(&json!({"from": "x"}), 0, inside).is_none());
    }
}
