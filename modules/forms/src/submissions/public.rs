//! Nhận bài gửi công khai, theo hợp đồng chung (`docs/prompts/reader-input-contract.md`):
//!
//! 1. Kiểm form (có, đang mở), `Origin`, rồi kiểm MỌI câu trả lời (báo mọi lỗi một lần), dò
//!    trùng, rồi `SpamGuard::check` — mọi việc có thể gọi mạng nằm ở pha này, NGOÀI
//!    transaction.
//! 2. `Discard` → không ghi gì. `Accept` / `Quarantine` → một transaction: chèn bài (spam thì
//!    trạng thái `spam` kèm lý do), event (chỉ bài được nhận), rồi chèn nonce của token
//!    (`Checked::claim`). Nonce đã có → bài này đã gửi rồi: rollback, trả lại ID lần đầu.
//!
//! Câu trả lời cho nhận, giữ lại và bỏ đi GIỐNG HỆT (cùng mã 201, cùng hình dạng, ID cùng định
//! dạng — bỏ đi thì ID giả): bot không phân biệt được.

use std::time::Duration;

use autoxide_kernel::access::Principal;
use autoxide_kernel::antispam::{self, Claim, Limits, Submission as Check, Verdict};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::ClientInfo;
use autoxide_kernel::rows;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::throttle::Throttle;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, PaginatorTrait,
    QueryFilter,
};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::entity::{form as form_row, submission};
use crate::events::{self, Change};
use crate::forms::fields::{self, FieldType, Locale};
use crate::forms::{Form, FormStatus, normalize_origin};
use crate::permissions::FORMS_SUBMIT;
use crate::service::{FormsService, form_not_found, now, to_json};
use crate::submissions::inbox::{SubmissionStatus, summary};

const HOUR: Duration = Duration::from_secs(3600);
/// Cùng nội dung gửi tới cùng form trong chừng này giờ thì bị coi là trùng (giữ lại).
const DUPLICATE_HOURS: i64 = 24;
const URL_MAX: usize = 2048;

/// Một bài gửi công khai, như client gửi lên.
#[derive(Debug, Clone, Default)]
pub(crate) struct PublicSubmission {
    pub data: Map<String, Value>,
    pub locale: Option<String>,
    pub page_url: Option<String>,
    pub referrer: Option<String>,
    pub form_token: Option<String>,
    pub trap: Option<String>,
    pub challenge: Option<String>,
}

/// Câu trả lời cho người gửi: như nhau dù bài được nhận, bị giữ lại hay bị bỏ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Receipt {
    pub id: Uuid,
    pub message: String,
}

impl FormsService {
    /// Form `key` cho website hiển thị; form đóng vẫn trả (để website báo "đã đóng").
    pub(crate) async fn public_form(
        &self,
        tenant: &TenantContext,
        key: &str,
    ) -> Result<Form, Error> {
        self.find_by_key(tenant, key).await
    }

    /// Nhận bài gửi cho form `key`. `principal` có `forms.submit` (máy chủ website gọi bằng API
    /// key): bỏ qua bẫy, token, đề, vẫn bị giới hạn theo người dùng. Đăng nhập mà KHÔNG có
    /// `forms.submit` (vd tài khoản tự đăng ký) thì đi đúng đường của người lạ — đăng nhập không
    /// phải là bằng chứng "không phải bot". `principal` vẫn là người làm (actor) của event.
    pub(crate) async fn submit(
        &self,
        tenant: &TenantContext,
        key: &str,
        principal: Option<&Principal>,
        client: &ClientInfo,
        input: PublicSubmission,
    ) -> Result<Receipt, Error> {
        let form = self.find_by_key(tenant, key).await?;
        if form.status() == FormStatus::Closed {
            return Err(Error::conflict(
                "forms.closed",
                "This form is closed and does not take submissions.",
            ));
        }
        if let Some(origin) = client.origin.as_deref()
            && !form.allowed_origins.is_empty()
            && !normalize_origin(origin)
                .is_some_and(|origin| form.allowed_origins.contains(&origin))
        {
            return Err(Error::forbidden(
                "forms.origin_not_allowed",
                "This form cannot be sent from this website (its `Origin` is not in the form's `allowed_origins`).",
            ));
        }

        let mut problems = Vec::new();
        let locale = match input.locale.as_deref().map(str::trim).filter(|raw| !raw.is_empty()) {
            None => None,
            Some(raw) => {
                let parsed = Locale::parse(raw);
                if parsed.is_none() {
                    problems.push(FieldError::new("locale", "invalid", "must be `en` or `vi`"));
                }
                parsed
            }
        };
        // Không gửi `page_url` thì lấy header `Referer` (trình duyệt gửi trang chứa form), chỉ
        // khi nó là địa chỉ `http(s)` — header lạ không phải lỗi của người gửi.
        let page_url = match input.page_url {
            Some(url) => check_url("page_url", Some(url), &mut problems),
            None => check_url("page_url", client.referer.clone(), &mut Vec::new()),
        };
        let referrer = check_url("referrer", input.referrer, &mut problems);
        let answers = fields::check_answers(&form.fields, &input.data, &mut problems);
        let data = to_json(&answers.data)?;
        if data.len() > self.config.max_submission_bytes {
            problems.push(
                FieldError::new("data", "length", "must be at most {max} bytes of JSON")
                    .with_param("max", self.config.max_submission_bytes),
            );
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let email =
            fields::sender_email(&form.fields, form.row.reply_to_field.as_deref(), &answers.data)
                .map(|email| autoxide_kernel::email::normalize(&email));
        let content_hash = antispam::content_hash(&data);
        let since = now() - chrono::Duration::hours(DUPLICATE_HOURS);
        let duplicate = submission::Entity::find()
            .scoped(tenant)
            .filter(submission::Column::FormId.eq(form.row.id))
            .filter(submission::Column::ContentHash.eq(content_hash.as_str()))
            .filter(submission::Column::CreatedAt.gt(since))
            .count(tenant.database().read())
            .await?
            > 0;

        // Pha một: chống spam, ngoài transaction.
        let scope = form.scope();
        let limits = self.limits(&form)?;
        let trusted = principal.filter(|principal| principal.has(&FORMS_SUBMIT));
        let checked = self
            .guard
            .check(
                tenant,
                &Check::new(&scope, client)
                    .principal(trusted)
                    .honeypot(input.trap.as_deref())
                    .form_token(input.form_token.as_deref())
                    .challenge(input.challenge.as_deref())
                    .email(email.as_deref())
                    .content(Some(&answers.text))
                    .duplicate(duplicate),
                &limits,
            )
            .await?;
        let message = form.success_message.get(locale.unwrap_or_default()).to_owned();
        let (status, spam_reason) = match &checked.verdict {
            Verdict::Discard { reason } => {
                tracing::info!(form_id = %form.row.id, reason, "public submission dropped");
                return Ok(Receipt { id: Uuid::now_v7(), message });
            }
            Verdict::Accept => (SubmissionStatus::New, None),
            Verdict::Quarantine { reasons } => (SubmissionStatus::Spam, Some(reasons.join(","))),
        };
        let ip_fingerprint = match client.ip {
            Some(ip) => {
                Some(autoxide_kernel::ip_hash::fingerprint(&self.secret, "forms", ip).await?)
            }
            None => None,
        };
        let now = now();
        let consent = form
            .fields
            .iter()
            .find(|field| {
                field.kind == FieldType::Consent
                    && answers.data.get(&field.key) == Some(&Value::Bool(true))
            })
            .map(|field| {
                json!({
                    "text": field.label.get(locale.unwrap_or_default()),
                    "version": form.row.version.to_string(),
                    "at": now,
                    "source": scope,
                    "field": field.key,
                    "ip_fingerprint": ip_fingerprint,
                })
            });
        let row = submission::Model {
            id: Uuid::now_v7(),
            tenant_id: tenant.id().as_uuid(),
            form_id: form.row.id,
            form_version: form.row.version,
            data,
            status: status.as_str().to_owned(),
            spam_reason,
            email,
            locale: locale.map(|locale| locale.as_str().to_owned()),
            page_url,
            referrer,
            user_agent: client.user_agent.clone(),
            ip_fingerprint,
            consent: consent.as_ref().map(to_json).transpose()?,
            content_hash,
            search_key: autoxide_kernel::search::fold(&answers.text),
            created_at: now,
            updated_at: now,
        };

        // Pha hai: một transaction ngắn. Khoá dòng form trước (khoá dòng tới hết transaction
        // trên Postgres; SQLite chỉ có một bên ghi): form bị xoá giữa hai pha thì dừng ở đây,
        // không để lại bài mồ côi chứa dữ liệu cá nhân; xoá form chạy sau thì thấy và xoá cả bài này.
        let transaction = tenant.database().begin_write().await?;
        if !lock_form(&transaction, tenant, form.row.id).await? {
            transaction.rollback().await?;
            return Err(form_not_found());
        }
        let row = row.into_active_model().insert(&transaction).await?;
        if status == SubmissionStatus::New {
            let actor = principal.map_or_else(Actor::public, Actor::from);
            let event = events::SubmissionReceived(Change {
                before: None,
                after: Some(summary(&row, &form.row.key)),
            });
            self.outbox
                .record(&transaction, tenant, actor, events::submission_subject(row.id), &event)
                .await?;
        }
        match checked.claim(&transaction, tenant, "forms", Some(&row.id.to_string())).await? {
            Claim::New => {
                transaction.commit().await?;
                tracing::info!(form_id = %form.row.id, submission_id = %row.id, status = status.as_str(), "form submission stored");
                Ok(Receipt { id: row.id, message })
            }
            Claim::Replay { result } => {
                transaction.rollback().await?;
                let id = result.and_then(|id| id.parse().ok()).unwrap_or_else(Uuid::now_v7);
                Ok(Receipt { id, message })
            }
        }
    }

    /// Giới hạn số lần của form (theo IP băm, email, người dùng), trên bảng `forms_throttle`.
    fn limits(&self, form: &Form) -> Result<Limits, Error> {
        let limit = |own: Option<i32>, default: u32| {
            own.and_then(|own| u32::try_from(own).ok()).filter(|own| *own > 0).unwrap_or(default)
        };
        let throttle = |limit: u32| Throttle::new("forms", limit, HOUR);
        Ok(Limits::default()
            .ip(Some(throttle(limit(form.row.ip_limit_per_hour, self.config.per_ip_per_hour))?))
            .email(Some(throttle(limit(
                form.row.email_limit_per_hour,
                self.config.per_email_per_hour,
            ))?))
            .user(Some(throttle(self.config.per_user_per_hour)?)))
    }
}

/// Khoá dòng form `id` tới hết transaction (`rows::lock_one`, D-181; không ghi gì nên ETag giữ
/// nguyên). `false`: form không còn.
async fn lock_form<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    id: Uuid,
) -> Result<bool, Error> {
    Ok(rows::lock_one(connection, form_row::Entity::find_by_id(id).scoped(tenant)).await?.is_some())
}

/// Địa chỉ `http(s)` tuỳ chọn; sai thì thêm lỗi `field`.
fn check_url(field: &str, value: Option<String>, problems: &mut Vec<FieldError>) -> Option<String> {
    let value = value.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty())?;
    let lower = value.to_ascii_lowercase();
    let valid = (lower.starts_with("https://") || lower.starts_with("http://"))
        && value.len() <= URL_MAX
        && !value.chars().any(|c| c.is_whitespace() || c.is_control());
    if valid {
        Some(value)
    } else {
        problems.push(FieldError::new(
            field,
            "invalid",
            "must be an `https://` or `http://` address",
        ));
        None
    }
}
