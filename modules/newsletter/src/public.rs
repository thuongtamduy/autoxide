//! Phần công khai (người lạ, website gọi): đăng ký, xác nhận, huỷ một chạm, trang lựa chọn.
//!
//! Đăng ký theo hợp đồng chung (`docs/prompts/reader-input-contract.md`):
//!
//! 1. Kiểm dữ liệu (báo mọi lỗi một lần), rồi `SpamGuard::check` — NGOÀI transaction.
//! 2. `Discard` → không ghi gì. `Accept` / `Quarantine` → một transaction ngắn, giống nhau cho
//!    mọi địa chỉ: xếp job [`SUBSCRIBE`] và chèn nonce của token form (`Checked::claim`). Nonce
//!    đã có → rollback (gửi lại cùng token = không thêm job).
//! 3. Job (`apply_subscription`): một transaction tạo hoặc giữ người đăng ký (email unique, chèn
//!    `ON CONFLICT DO NOTHING` nên hai job cùng email ra một dòng), đặt đăng ký, phát token xác
//!    nhận, event; SAU commit đếm bộ đếm mail xác nhận của địa chỉ rồi xếp hàng mail nếu cần và
//!    còn lượt (bộ đếm và bên gửi mail ghi bằng kết nối ghi riêng; gọi trong transaction thì
//!    SQLite — một bên ghi — chờ chính mình). Lỗi ở bước mail chỉ ghi log.
//!
//! Câu trả lời luôn GIỐNG HỆT (202, cùng thân) dù email mới, đang chờ, đã xác nhận, bị chặn hay
//! bị bỏ vì spam: không ai dò được địa chỉ nào đã có trong danh sách. Thời gian trả lời cũng
//! không lộ: phần tuỳ theo trạng thái của địa chỉ chạy trong job (D-154).
//!
//! Buộc xác nhận qua mail, kể cả ở danh sách không bật double opt-in, khi: bị `Quarantine` (gửi
//! quá nhanh… — người thật bấm link là xong, bot thì không); hoặc đăng ký hiện có đang
//! `unsubscribed` (người đã rời đi thì chỉ chính họ mới vào lại được, người lạ không đăng ký hộ
//! được) hay `unconfirmed` (đang chờ xác nhận thì không được gửi lại để bỏ qua bước đó).
//!
//! Gọi bằng tài khoản có `newsletter.subscribe` (máy chủ website): bỏ qua bẫy / token / đề, vẫn
//! bị giới hạn theo tài khoản. Đăng nhập mà không có quyền đó thì đi đường của người lạ.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use autoxide_kernel::access::Principal;
use autoxide_kernel::antispam::{Claim, Limits, Submission as Check, Verdict};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::ClientInfo;
use autoxide_kernel::job::{EnqueueOptions, Job, JobContext, JobHandler};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext, link_token, resolve_link_token};
use autoxide_kernel::throttle::Throttle;
use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::audience::lists::List;
use crate::audience::subscribers::{
    ImportPerson, SubscriberStatus, SubscriptionStatus, Wanted, set_subscription,
    subscription_summary, upsert_subscriber,
};
use crate::entity::{campaign, delivery, subscriber, subscription, suppression};
use crate::events::{self, Change};
use crate::permissions::NEWSLETTER_SUBSCRIBE;
use crate::service::{MODULE, NewsletterService, now, to_json};
use crate::text::{self, Locale};

/// Phạm vi của token form cho đăng ký (`GET /api/v1/antispam/token?scope=newsletter:subscribe`).
pub(crate) const SCOPE: &str = "newsletter:subscribe";
/// Mục đích của token xác nhận trong `newsletter_tokens`.
const CONFIRM: &str = "confirm";
const HOUR: Duration = Duration::from_secs(3600);
const DAY: Duration = Duration::from_secs(24 * 3600);
const LISTS_MAX: usize = 20;

/// Loại job lưu một yêu cầu đăng ký đã qua lớp chống spam.
pub(crate) const SUBSCRIBE: &str = "newsletter.subscribe";

/// Job [`SUBSCRIBE`]: yêu cầu đăng ký đã kiểm, chờ lưu. Mang email và tên người đăng ký — job
/// queue xoá job đã xong sau thời hạn của nó, như job gửi mail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SubscribeJob {
    pub email: String,
    pub name: Option<String>,
    pub locale: Option<Locale>,
    pub list_ids: Vec<Uuid>,
    /// Bằng chứng đồng ý (JSON), lập lúc nhận yêu cầu.
    pub consent: String,
    /// Lý do lớp chống spam giữ lại; có thì phải xác nhận qua mail.
    pub hold_reason: Option<String>,
    /// Người gọi đã đăng nhập (máy chủ website); `None`: người lạ.
    pub actor_user_id: Option<Uuid>,
    pub actor_api_key_id: Option<Uuid>,
}

impl Job for SubscribeJob {
    const KIND: &'static str = SUBSCRIBE;
}

/// Bên chạy job [`SUBSCRIBE`].
pub(crate) struct Subscriber(pub Arc<NewsletterService>);

#[async_trait::async_trait]
impl JobHandler for Subscriber {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        let tenant = job
            .tenant
            .as_ref()
            .ok_or_else(|| Error::internal("newsletter subscribe job without a tenant"))?;
        let job: SubscribeJob =
            serde_json::from_value(job.payload.clone()).map_err(Error::internal)?;
        self.0.apply_subscription(tenant, &job).await
    }
}

/// Một yêu cầu đăng ký, như client gửi lên.
#[derive(Debug, Clone, Default)]
pub(crate) struct SubscribeRequest {
    pub email: Option<String>,
    pub name: Option<String>,
    pub locale: Option<String>,
    pub lists: Vec<String>,
    pub consent_text: Option<String>,
    pub consent_version: Option<String>,
    pub source: Option<String>,
    pub form_token: Option<String>,
    pub trap: Option<String>,
    pub challenge: Option<String>,
}

/// Người có token (từ một mail chiến dịch) xem gì ở trang lựa chọn.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Preferences {
    pub subscriber: subscriber::Model,
    /// Danh sách công khai và danh sách người này có đăng ký, theo `key`, kèm trạng thái.
    pub lists: Vec<(List, Option<SubscriptionStatus>)>,
}

/// Thay đổi ở trang lựa chọn.
#[derive(Debug, Clone, Default)]
pub(crate) struct PreferencesChange {
    pub name: Option<Option<String>>,
    pub locale: Option<String>,
    /// (`key`, có nhận hay không).
    pub lists: Vec<(String, bool)>,
    pub unsubscribe_all: bool,
    pub consent_text: Option<String>,
    pub consent_version: Option<String>,
}

/// Đăng ký vào `list` (trạng thái hiện có `current`) có phải xác nhận qua mail không: danh sách
/// bật double opt-in, bị lớp chống spam giữ lại (`held`), hoặc người này đã rời danh sách hay
/// đang chờ xác nhận — một yêu cầu đăng ký công khai không chứng minh được nó đến từ chính họ.
fn must_confirm(list: &List, current: Option<SubscriptionStatus>, held: bool) -> bool {
    list.row.double_opt_in
        || held
        || matches!(
            current,
            Some(SubscriptionStatus::Unsubscribed | SubscriptionStatus::Unconfirmed)
        )
}

impl NewsletterService {
    /// Nhận một yêu cầu đăng ký. Trả `Ok(())` như nhau cho mọi trường hợp không phải lỗi dữ
    /// liệu / token / cấu hình. `principal` có `newsletter.subscribe`: bỏ qua lớp chống spam cho
    /// người lạ (vẫn bị giới hạn theo tài khoản).
    pub(crate) async fn subscribe(
        &self,
        tenant: &TenantContext,
        principal: Option<&Principal>,
        client: &ClientInfo,
        input: SubscribeRequest,
    ) -> Result<(), Error> {
        let mut problems = Vec::new();
        let email = match input.email.as_deref() {
            Some(email) => autoxide_kernel::email::checked(email, "email", &mut problems),
            None => {
                problems.push(FieldError::new("email", "required", "is required"));
                None
            }
        };
        let name = text::line(input.name.as_deref(), "name", 200, &mut problems);
        let locale = text::locale_param(input.locale.as_deref(), "locale", &mut problems);
        let consent_text =
            text::text(input.consent_text.as_deref(), "consent_text", 2000, &mut problems);
        let version =
            text::line(input.consent_version.as_deref(), "consent_version", 50, &mut problems);
        let source = text::line(input.source.as_deref(), "source", 100, &mut problems);
        let public: BTreeMap<String, List> = self
            .public_lists(tenant)
            .await?
            .into_iter()
            .map(|list| (list.row.key.clone(), list))
            .collect();
        let mut lists: Vec<&List> = Vec::new();
        if input.lists.is_empty() || input.lists.len() > LISTS_MAX {
            problems.push(
                FieldError::new("lists", "length", "must have 1 to {lists_max} list keys")
                    .with_param("lists_max", LISTS_MAX),
            );
        }
        for (index, key) in input.lists.iter().enumerate() {
            match public.get(key.trim()) {
                Some(list) if !lists.iter().any(|seen| seen.row.id == list.row.id) => {
                    lists.push(list)
                }
                Some(_) => {}
                None => problems.push(FieldError::new(
                    format!("lists[{index}]"),
                    "invalid",
                    "no public list has this key",
                )),
            }
        }
        let Some(email) = email.filter(|_| problems.is_empty()) else {
            return Err(Error::validation(problems));
        };
        if lists.iter().any(|list| list.row.double_opt_in) {
            self.confirm_url()?;
        }

        // Pha một: chống spam, ngoài transaction. Chỉ tài khoản có `newsletter.subscribe` mới
        // được bỏ qua lớp chống spam cho người lạ; `principal` vẫn là người làm của event.
        let trusted = principal.filter(|principal| principal.has(&NEWSLETTER_SUBSCRIBE));
        let checked = self
            .guard
            .check(
                tenant,
                &Check::new(SCOPE, client)
                    .principal(trusted)
                    .honeypot(input.trap.as_deref())
                    .form_token(input.form_token.as_deref())
                    .challenge(input.challenge.as_deref())
                    .email(Some(&email)),
                &self.limits()?,
            )
            .await?;
        let hold = match &checked.verdict {
            Verdict::Discard { reason } => {
                tracing::info!(reason, "newsletter subscription dropped");
                return Ok(());
            }
            Verdict::Accept => None,
            Verdict::Quarantine { reasons } => Some(reasons.join(",")),
        };
        let ip_fingerprint = match client.ip {
            Some(ip) => {
                Some(autoxide_kernel::ip_hash::fingerprint(&self.secret, MODULE, ip).await?)
            }
            None => None,
        };
        let consent = to_json(&json!({
            "text": consent_text,
            "version": version,
            "at": now(),
            "source": source.unwrap_or_else(|| SCOPE.to_owned()),
            "ip_fingerprint": ip_fingerprint,
        }))?;
        let job = SubscribeJob {
            email,
            name,
            locale,
            list_ids: lists.iter().map(|list| list.row.id).collect(),
            consent,
            hold_reason: hold,
            actor_user_id: principal.map(Principal::user_id),
            actor_api_key_id: principal.and_then(Principal::api_key_id),
        };

        // Pha hai: một transaction ngắn, CÙNG một việc cho mọi địa chỉ — xếp job và chèn nonce
        // của token form. Việc tuỳ theo trạng thái của địa chỉ (tạo người, đổi đăng ký, token
        // xác nhận, mail) chạy trong job, nên thời gian trả lời không lộ địa chỉ đã có hay chưa
        // (D-154). Nonce đã có (gửi lại cùng token) → rollback: không xếp job thứ hai.
        let transaction = tenant.database().begin_write().await?;
        self.jobs.enqueue(&transaction, tenant, &job, EnqueueOptions::default()).await?;
        if let Claim::Replay { .. } = checked.claim(&transaction, tenant, MODULE, None).await? {
            transaction.rollback().await?;
            return Ok(());
        }
        transaction.commit().await?;
        tracing::info!(lists = job.list_ids.len(), "newsletter subscription request queued");
        Ok(())
    }

    /// Job [`SUBSCRIBE`]: lưu một yêu cầu đăng ký đã qua lớp chống spam, rồi gửi mail xác nhận
    /// nếu cần. Chạy lại (worker chết giữa chừng) thì như người đó gửi lại yêu cầu: không thêm
    /// người, không đổi đăng ký đã xác nhận; có thể thêm một mail xác nhận (vẫn trong giới hạn
    /// mỗi ngày).
    pub(crate) async fn apply_subscription(
        &self,
        tenant: &TenantContext,
        job: &SubscribeJob,
    ) -> Result<(), Error> {
        let now = now();
        let actor = match job.actor_user_id {
            Some(user_id) => match job.actor_api_key_id {
                Some(api_key_id) => Actor::api_key(user_id, api_key_id),
                None => Actor::user(user_id),
            },
            None => Actor::public(),
        };
        let transaction = tenant.database().begin_write().await?;
        // Danh sách bị xoá từ lúc nhận yêu cầu thì bỏ qua.
        let by_id = self.lists_by_id(&transaction, tenant, job.list_ids.iter().copied()).await?;
        let lists: Vec<&List> = job.list_ids.iter().filter_map(|id| by_id.get(id)).collect();
        let person = ImportPerson {
            email: job.email.clone(),
            name: job.name.clone(),
            locale: job.locale,
            attributes: None,
        };
        let (row, _) = upsert_subscriber(&transaction, tenant, &person, now).await?;
        let suppressed = suppression::Entity::find()
            .scoped(tenant)
            .filter(suppression::Column::Email.eq(job.email.as_str()))
            .one(&transaction)
            .await?
            .is_some();
        let held = job.hold_reason.is_some();
        let mut to_confirm: Vec<&List> = Vec::new();
        if row.status != SubscriberStatus::Blocked.as_str() && !suppressed {
            for list in &lists {
                let existing =
                    subscription::Entity::find_by_id((tenant.id().as_uuid(), row.id, list.row.id))
                        .scoped(tenant)
                        .one(&transaction)
                        .await?;
                let current = existing.as_ref().map(|row| SubscriptionStatus::stored(&row.status));
                if current == Some(SubscriptionStatus::Confirmed) {
                    continue;
                }
                let double = must_confirm(list, current, held);
                let status = if double {
                    SubscriptionStatus::Unconfirmed
                } else {
                    SubscriptionStatus::Confirmed
                };
                let wanted = Wanted {
                    status,
                    consent: Some(job.consent.clone()),
                    hold_reason: job.hold_reason.clone(),
                };
                let saved = set_subscription(
                    &transaction,
                    tenant,
                    existing,
                    (row.id, list.row.id),
                    wanted,
                    now,
                )
                .await?;
                if double {
                    to_confirm.push(list);
                }
                if current != Some(status) {
                    let change = Change {
                        before: None,
                        after: Some(subscription_summary(&saved, &list.row.key)),
                    };
                    let subject = events::subscriber_subject(row.id);
                    if double {
                        let event = events::SubscriptionRequested(change);
                        self.outbox.record(&transaction, tenant, actor, subject, &event).await?;
                    } else {
                        let event = events::SubscriptionConfirmed(change);
                        self.outbox.record(&transaction, tenant, actor, subject, &event).await?;
                    }
                }
            }
        }
        // Token xác nhận phát trong transaction; có gửi mail hay không thì xét SAU commit (bộ
        // đếm mail xác nhận ghi bằng kết nối riêng). Token không được gửi đi thì tự hết hạn.
        let issued = if to_confirm.is_empty() {
            None
        } else {
            let ids: Vec<String> = to_confirm.iter().map(|list| list.row.id.to_string()).collect();
            let ttl = chrono::Duration::hours(i64::from(self.config.confirm_hours));
            let subject = row.id.to_string();
            Some(
                self.tokens
                    .issue(&transaction, tenant, CONFIRM, &subject, Some(&ids.join(",")), ttl)
                    .await?,
            )
        };
        transaction.commit().await?;
        tracing::info!(subscriber_id = %row.id, lists = lists.len(), confirming = to_confirm.len(), "newsletter subscription request stored");
        self.sync_consents_quietly(tenant, row.id).await;
        // Sau commit, mọi yêu cầu đã lưu đều đếm một lần vào bộ đếm mail xác nhận của địa chỉ
        // (cần mail hay không). Lỗi ở đây chỉ ghi log, không làm job chạy lại: người đăng ký
        // gửi lại là có link mới.
        let queued = match self.confirm_mail_allowed(tenant, &job.email).await {
            Ok(allowed) => match issued.filter(|_| allowed) {
                Some(issued) => {
                    let locale = job.locale.unwrap_or_else(|| Locale::stored(&row.locale));
                    // Link mang khách của token (`autoxide_kernel::tenancy::link_token`).
                    let token = link_token(tenant, &issued.token);
                    match self
                        .confirmation_mail(
                            tenant,
                            &job.email,
                            locale,
                            &to_confirm,
                            &token,
                            issued.id,
                        )
                        .await
                    {
                        Ok(mail) => self.mailer.send(tenant, mail).await,
                        Err(error) => Err(error),
                    }
                }
                None => Ok(()),
            },
            Err(error) => Err(error),
        };
        if let Err(error) = queued {
            tracing::warn!(subscriber_id = %row.id, code = error.code(), "newsletter confirmation mail not queued");
        }
        Ok(())
    }

    /// Đếm một lần cho `email` vào bộ đếm mail xác nhận (tối đa
    /// `AUTOXIDE_NEWSLETTER_CONFIRM_MAILS_PER_DAY` một ngày); `true` nếu còn được gửi.
    async fn confirm_mail_allowed(
        &self,
        tenant: &TenantContext,
        email: &str,
    ) -> Result<bool, Error> {
        let throttle = Throttle::new(MODULE, self.config.confirm_mails_per_day, DAY)?;
        Ok(throttle.hit(tenant, &format!("confirm:{email}")).await?.is_allowed())
    }

    /// Giới hạn số lần đăng ký (theo IP băm, email, người dùng), trên bảng `newsletter_throttle`.
    fn limits(&self) -> Result<Limits, Error> {
        let throttle = |limit: u32| Throttle::new(MODULE, limit, HOUR);
        Ok(Limits::default()
            .ip(Some(throttle(self.config.per_ip_per_hour)?))
            .email(Some(throttle(self.config.per_email_per_hour)?))
            .user(Some(throttle(self.config.per_user_per_hour)?)))
    }

    /// Xác nhận bằng token trong mail: các đăng ký đang chờ thành `confirmed`. Trả các danh sách
    /// vừa (hay đã) được xác nhận. Khách lấy từ token khi token mang khách; không thì `request`
    /// (khách của request, `None`: khách mặc định) — xem
    /// `autoxide_kernel::tenancy::resolve_link_token`.
    pub(crate) async fn confirm(
        &self,
        request: Option<&TenantContext>,
        token: &str,
    ) -> Result<Vec<List>, Error> {
        let Some((resolved, raw)) = resolve_link_token(&self.databases, request, token).await?
        else {
            return Err(confirmation_invalid());
        };
        let tenant = &resolved;
        let transaction = tenant.database().begin_write().await?;
        let Some(consumed) = self.tokens.consume(&transaction, tenant, CONFIRM, raw).await? else {
            transaction.rollback().await?;
            return Err(confirmation_invalid());
        };
        let subscriber_id: Uuid = consumed.subject.parse().map_err(Error::internal)?;
        let list_ids: Vec<Uuid> =
            consumed.data.unwrap_or_default().split(',').filter_map(|id| id.parse().ok()).collect();
        let lists = self.lists_by_id(&transaction, tenant, list_ids.iter().copied()).await?;
        let now = now();
        let mut confirmed = Vec::new();
        for id in &list_ids {
            let Some(list) = lists.get(id) else { continue };
            // Khoá dòng đăng ký (D-181): huỷ đăng ký cùng lúc từng bị ghi đè thành `confirmed`
            // trên Postgres — người vừa từ chối nhận thư lại thành đã đồng ý (D-367).
            let Some(existing) = autoxide_kernel::rows::lock_one(
                &transaction,
                subscription::Entity::find_by_id((tenant.id().as_uuid(), subscriber_id, *id))
                    .scoped(tenant),
            )
            .await?
            else {
                continue;
            };
            match SubscriptionStatus::stored(&existing.status) {
                SubscriptionStatus::Unconfirmed => {
                    let mut row = existing.into_active_model();
                    row.status = Set(SubscriptionStatus::Confirmed.as_str().to_owned());
                    row.confirmed_at = Set(Some(now));
                    row.updated_at = Set(now);
                    let saved = subscription::Entity::update(row)
                        .validate()?
                        .scoped(tenant)
                        .exec(&transaction)
                        .await?;
                    let change = Change {
                        before: None,
                        after: Some(subscription_summary(&saved, &list.row.key)),
                    };
                    self.outbox
                        .record(
                            &transaction,
                            tenant,
                            Actor::public(),
                            events::subscriber_subject(subscriber_id),
                            &events::SubscriptionConfirmed(change),
                        )
                        .await?;
                    confirmed.push(list.clone());
                }
                SubscriptionStatus::Confirmed => confirmed.push(list.clone()),
                // Đã huỷ sau khi xin đăng ký: không tự đăng ký lại.
                SubscriptionStatus::Unsubscribed => {}
            }
        }
        transaction.commit().await?;
        tracing::info!(subscriber_id = %subscriber_id, lists = confirmed.len(), "newsletter subscription confirmed");
        // Sổ đồng ý chung ghi sau commit (người đăng ký chỉ biết chắc trong transaction); lúc gửi
        // luôn hỏi lại sổ (D-367).
        self.sync_consents_quietly(tenant, subscriber_id).await;
        Ok(confirmed)
    }

    /// Lượt gửi của khách `tenant` có token huỷ đăng ký (phần thô) `token`, kèm chiến dịch.
    async fn by_token(
        &self,
        tenant: &TenantContext,
        token: &str,
    ) -> Result<Option<(delivery::Model, campaign::Model)>, Error> {
        let read = tenant.database().read();
        let Some(found) = delivery::Entity::find()
            .scoped(tenant)
            .filter(
                delivery::Column::UnsubscribeTokenHash.eq(autoxide_kernel::secret::sha256(token)),
            )
            .one(read)
            .await?
        else {
            return Ok(None);
        };
        let campaign =
            campaign::Entity::find_by_id(found.campaign_id).scoped(tenant).one(read).await?;
        Ok(campaign.map(|campaign| (found, campaign)))
    }

    /// Huỷ một chạm (RFC 8058): huỷ đăng ký danh sách của mail mang token. Token lạ cũng trả
    /// `Ok` (không lộ gì). Có hiệu lực ngay. Khách như [`confirm`](Self::confirm).
    pub(crate) async fn unsubscribe(
        &self,
        request: Option<&TenantContext>,
        token: &str,
    ) -> Result<(), Error> {
        let Some((resolved, raw)) = resolve_link_token(&self.databases, request, token).await?
        else {
            return Ok(());
        };
        let tenant = &resolved;
        let Some((found, campaign)) = self.by_token(tenant, raw).await? else {
            return Ok(());
        };
        let Ok(list) = self.find_list(tenant, campaign.list_id).await else {
            return Ok(());
        };
        let transaction = tenant.database().begin_write().await?;
        let now = now();
        let changed = self
            .unsubscribe_in(&transaction, tenant, Actor::public(), found.subscriber_id, &list, now)
            .await?;
        if changed {
            mark_unsubscribed(&transaction, tenant, found.id, now).await?;
        }
        transaction.commit().await?;
        if changed {
            tracing::info!(subscriber_id = %found.subscriber_id, list_id = %list.row.id, "newsletter one-click unsubscribe");
            self.sync_consents_quietly(tenant, found.subscriber_id).await;
        }
        Ok(())
    }

    /// Trang lựa chọn: người mang token, các danh sách công khai và danh sách người đó có đăng ký.
    /// Khách như [`confirm`](Self::confirm).
    pub(crate) async fn preferences(
        &self,
        request: Option<&TenantContext>,
        token: &str,
    ) -> Result<Preferences, Error> {
        let (tenant, raw) =
            resolve_link_token(&self.databases, request, token).await?.ok_or_else(link_invalid)?;
        self.preferences_in(&tenant, raw).await
    }

    /// Như [`preferences`](Self::preferences), trong khách `tenant` với phần thô `token`.
    async fn preferences_in(
        &self,
        tenant: &TenantContext,
        token: &str,
    ) -> Result<Preferences, Error> {
        let (found, _) = self.by_token(tenant, token).await?.ok_or_else(link_invalid)?;
        let row = subscriber::Entity::find_by_id(found.subscriber_id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(link_invalid)?;
        self.preferences_of(tenant, row).await
    }

    async fn preferences_of(
        &self,
        tenant: &TenantContext,
        row: subscriber::Model,
    ) -> Result<Preferences, Error> {
        let memberships: HashMap<Uuid, SubscriptionStatus> = subscription::Entity::find()
            .scoped(tenant)
            .filter(subscription::Column::SubscriberId.eq(row.id))
            .all(tenant.database().read())
            .await?
            .into_iter()
            .map(|row| (row.list_id, SubscriptionStatus::stored(&row.status)))
            .collect();
        let mut lists: BTreeMap<String, List> = self
            .public_lists(tenant)
            .await?
            .into_iter()
            .map(|list| (list.row.key.clone(), list))
            .collect();
        for list in self
            .lists_by_id(tenant.database().read(), tenant, memberships.keys().copied())
            .await?
            .into_values()
        {
            lists.insert(list.row.key.clone(), list);
        }
        let lists = lists
            .into_values()
            .map(|list| {
                let status = memberships.get(&list.row.id).copied();
                (list, status)
            })
            .collect();
        Ok(Preferences { subscriber: row, lists })
    }

    /// Sửa lựa chọn bằng token. Nhận lại một danh sách công khai thì thành `confirmed` ngay (token
    /// đến từ hộp thư của chính người đó), kèm bằng chứng đồng ý nguồn `preferences`.
    pub(crate) async fn update_preferences(
        &self,
        request: Option<&TenantContext>,
        token: &str,
        change: PreferencesChange,
    ) -> Result<Preferences, Error> {
        let (resolved, token) =
            resolve_link_token(&self.databases, request, token).await?.ok_or_else(link_invalid)?;
        let tenant = &resolved;
        let before = self.preferences_in(tenant, token).await?;
        // Thư mang token: huỷ danh sách của chính thư đó thì ghi vào lượt gửi (số huỷ của chiến
        // dịch, D-269).
        let origin = self
            .by_token(tenant, token)
            .await?
            .map(|(found, campaign)| (found.id, campaign.list_id));
        let mut problems = Vec::new();
        let name = change.name.map(|name| text::line(name.as_deref(), "name", 200, &mut problems));
        let locale = text::locale_param(change.locale.as_deref(), "locale", &mut problems);
        let consent_text =
            text::text(change.consent_text.as_deref(), "consent_text", 2000, &mut problems);
        let version =
            text::line(change.consent_version.as_deref(), "consent_version", 50, &mut problems);
        let mut wanted: Vec<(&List, Option<SubscriptionStatus>, bool)> = Vec::new();
        for (index, (key, subscribed)) in change.lists.iter().enumerate() {
            match before.lists.iter().find(|(list, _)| list.row.key == key.trim()) {
                Some((list, status)) => {
                    if *subscribed
                        && !list.row.public
                        && *status != Some(SubscriptionStatus::Confirmed)
                    {
                        problems.push(FieldError::new(
                            format!("lists[{index}].subscribed"),
                            "invalid",
                            "this list cannot be joined from the preferences page",
                        ));
                    }
                    wanted.push((list, *status, *subscribed));
                }
                None => problems.push(FieldError::new(
                    format!("lists[{index}].key"),
                    "invalid",
                    "no list has this key",
                )),
            }
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let now = now();
        let id = before.subscriber.id;
        let transaction = tenant.database().begin_write().await?;
        let mut row = before.subscriber.clone().into_active_model();
        if let Some(name) = name {
            row.name = Set(name);
        }
        if let Some(locale) = locale {
            row.locale = Set(locale.as_str().to_owned());
        }
        if row.is_changed() {
            row.updated_at = Set(now);
            subscriber::Entity::update(row).validate()?.scoped(tenant).exec(&transaction).await?;
        }
        let consent = to_json(&json!({
            "text": consent_text,
            "version": version,
            "at": now,
            "source": "preferences",
            "ip_fingerprint": null,
        }))?;
        let subject = events::subscriber_subject(id);
        for (list, status, subscribed) in wanted {
            if change.unsubscribe_all || !subscribed {
                let changed = self
                    .unsubscribe_in(&transaction, tenant, Actor::public(), id, list, now)
                    .await?;
                if let Some((delivery_id, list_id)) = origin
                    && changed
                    && list_id == list.row.id
                {
                    mark_unsubscribed(&transaction, tenant, delivery_id, now).await?;
                }
            } else if status != Some(SubscriptionStatus::Confirmed) {
                let existing =
                    subscription::Entity::find_by_id((tenant.id().as_uuid(), id, list.row.id))
                        .scoped(tenant)
                        .one(&transaction)
                        .await?;
                let wanted = Wanted {
                    status: SubscriptionStatus::Confirmed,
                    consent: Some(consent.clone()),
                    hold_reason: None,
                };
                let saved = set_subscription(
                    &transaction,
                    tenant,
                    existing,
                    (id, list.row.id),
                    wanted,
                    now,
                )
                .await?;
                let change = Change {
                    before: None,
                    after: Some(subscription_summary(&saved, &list.row.key)),
                };
                self.outbox
                    .record(
                        &transaction,
                        tenant,
                        Actor::public(),
                        subject.clone(),
                        &events::SubscriptionConfirmed(change),
                    )
                    .await?;
            }
        }
        if change.unsubscribe_all {
            for (list, status) in &before.lists {
                if status.is_some() {
                    let changed = self
                        .unsubscribe_in(&transaction, tenant, Actor::public(), id, list, now)
                        .await?;
                    if let Some((delivery_id, list_id)) = origin
                        && changed
                        && list_id == list.row.id
                    {
                        mark_unsubscribed(&transaction, tenant, delivery_id, now).await?;
                    }
                }
            }
        }
        transaction.commit().await?;
        self.sync_consents_quietly(tenant, id).await;
        let row = subscriber::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(link_invalid)?;
        self.preferences_of(tenant, row).await
    }
}

fn confirmation_invalid() -> Error {
    Error::bad_request(
        "newsletter.confirmation_invalid",
        "This confirmation link is invalid, has expired or was already used. Subscribe again to \
         get a new one.",
    )
}

fn link_invalid() -> Error {
    Error::not_found(
        "newsletter.link_invalid",
        "This link does not match any subscription. Use the link of a more recent newsletter.",
    )
}

/// Ghi lần huỷ đăng ký bằng link của lượt gửi `delivery_id` (chỉ lần đầu).
async fn mark_unsubscribed<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    delivery_id: Uuid,
    now: DateTime<Utc>,
) -> Result<(), Error> {
    delivery::Entity::update_many()
        .scoped(tenant)
        .col_expr(delivery::Column::UnsubscribedAt, sea_orm::sea_query::Expr::value(Some(now)))
        .filter(delivery::Column::Id.eq(delivery_id))
        .filter(delivery::Column::UnsubscribedAt.is_null())
        .exec(connection)
        .await?;
    Ok(())
}
