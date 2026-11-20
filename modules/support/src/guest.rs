//! Người mua tự mở phiếu trên website: có tài khoản (thấy ở `/me/tickets`), hay vãng lai bằng email
//! / số điện thoại. Khách vãng lai nhận **token phiếu** một lần (database chỉ giữ bản băm) để xem và
//! viết tiếp vào đúng một phiếu; trước tin thứ hai phải xác thực liên lạc bằng mã một lần (OTP) gửi
//! qua email hay SMS. Đăng ký tài khoản sau đó thì `POST /me/claim` gộp các phiếu cùng liên lạc đã
//! xác thực vào tài khoản.

use std::time::Duration;

use autoxide_kernel::access::Principal;
use autoxide_kernel::antispam::{Claim, Limits, Submission as Check, Verdict};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::ClientInfo;
use autoxide_kernel::secret::sha256;
use autoxide_kernel::sms::PhoneNumber;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::throttle::{Outcome, Throttle};
use autoxide_notifications::api::{Notification, Recipient};
use chrono::Utc;
use sea_orm::{ColumnTrait, Condition, EntityTrait, IntoActiveModel, QueryFilter, Set};
use serde_json::json;
use uuid::Uuid;

use crate::entity::ticket;
use crate::events::{self, Change};
use crate::notify::VERIFICATION_CODE;
use crate::service::{MODULE, SupportService, Ticket, finish, not_found, now};
use crate::status::{Channel, Status};
use crate::tickets::{Opened, Opening, Who};

/// Phạm vi token form của form mở phiếu (`GET /api/v1/antispam/token?scope=support:tickets`).
pub(crate) const FORM_SCOPE: &str = "support:tickets";
/// Mã gọi quốc gia mặc định khi số không có `+` (đổi qua `+<mã>` đầu số).
const DEFAULT_COUNTRY: &str = "84";
/// Mã xác thực sống 15 phút, sai quá 5 lần thì phải xin mã mới.
const OTP_MINUTES: i64 = 15;
const OTP_ATTEMPTS: i32 = 5;
/// Xin mã mới tối đa 5 lần mỗi giờ cho một phiếu.
const OTP_PER_HOUR: u32 = 5;
const HOUR: Duration = Duration::from_secs(3600);

/// Phiếu người mua mở trên website.
#[derive(Debug, Clone, Default)]
pub(crate) struct PublicOpening {
    pub org_id: Option<Uuid>,
    pub name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub category: String,
    pub subject: String,
    pub body: String,
    pub locale: Option<String>,
    pub follow_up_of: Option<Uuid>,
    pub media_ids: Vec<Uuid>,
    pub form_token: Option<String>,
    pub trap: Option<String>,
    pub challenge: Option<String>,
}

impl SupportService {
    /// Mở phiếu từ website. Đăng nhập: phiếu của tài khoản (kênh `web`). Không đăng nhập: khách
    /// vãng lai — cần đúng một trong email / số điện thoại, qua chống spam (`SpamGuard`) và giới
    /// hạn theo IP băm, email, số điện thoại; nhận token phiếu và mã xác thực.
    pub(crate) async fn open_public(
        &self,
        tenant: &TenantContext,
        principal: Option<&Principal>,
        client: &ClientInfo,
        input: PublicOpening,
    ) -> Result<Opened, Error> {
        if let Some(principal) = principal {
            let opening = Opening {
                org_id: input.org_id,
                requester: Who::User(principal.user_id()),
                requester_name: input.name.unwrap_or_default(),
                verified: true,
                category: input.category,
                subject: input.subject,
                body: input.body,
                priority: None,
                channel: Channel::Web,
                media_ids: input.media_ids,
                links: Vec::new(),
                reference: None,
                follow_up_of: input.follow_up_of,
                locale: input.locale,
            };
            let actor = Actor::from(principal);
            return self.open_ticket(tenant, actor, Some(principal.user_id()), opening, None).await;
        }
        let org = input.org_id.unwrap_or_else(|| tenant.id().as_uuid());
        if !self.policy(tenant, org).await?.policy.guest_tickets {
            return Err(guest_tickets_disabled());
        }
        let requester = contact(input.email.as_deref(), input.phone.as_deref())?;
        if !input.media_ids.is_empty() {
            return Err(crate::messages::attachment_not_owned());
        }
        let ip = Throttle::new(MODULE, self.config.guest_per_hour, HOUR)?;
        let limits = Limits::default().ip(Some(ip.clone())).email(Some(ip.clone()));
        let email = match &requester {
            Who::Email(email) => Some(email.as_str()),
            _ => None,
        };
        if let Who::Phone(phone) = &requester
            && let Outcome::Limited { retry_after } =
                ip.hit(tenant, &format!("phone:{phone}")).await?
        {
            return Err(throttled(retry_after));
        }
        let checked = self
            .parts
            .guard
            .check(
                tenant,
                &Check::new(FORM_SCOPE, client)
                    .honeypot(input.trap.as_deref())
                    .form_token(input.form_token.as_deref())
                    .challenge(input.challenge.as_deref())
                    .email(email)
                    .content(Some(&input.body)),
                &limits,
            )
            .await?;
        let opening = Opening {
            org_id: input.org_id,
            requester: requester.clone(),
            requester_name: input.name.unwrap_or_default(),
            verified: false,
            category: input.category,
            subject: input.subject,
            body: input.body,
            priority: None,
            channel: Channel::Guest,
            media_ids: Vec::new(),
            links: Vec::new(),
            reference: None,
            follow_up_of: input.follow_up_of,
            locale: input.locale,
        };
        if let Verdict::Discard { reason } = &checked.verdict {
            // Bỏ đi, không lưu gì; người gửi vẫn được trả lời như thường (không dạy bot cách né).
            tracing::info!(reason, "support guest ticket dropped");
            return Ok(Opened {
                ticket: phantom(tenant, &opening),
                access_token: Some(autoxide_kernel::secret::random_token(32)?),
                created: true,
            });
        }
        let opened =
            self.open_ticket(tenant, Actor::public(), None, opening, Some(&checked)).await?;
        if opened.created
            && let Some(token) = &opened.access_token
            && let Err(error) = self.send_code(tenant, &opened.ticket, Some(token)).await
        {
            // Phiếu đã có; mã gửi lỗi thì người mua xin lại bằng `verify` không kèm mã.
            tracing::warn!(ticket_id = %opened.ticket.id, code = error.code(), "verification code not sent");
        }
        Ok(opened)
    }

    /// Phiếu `id` mở bằng token phiếu `token`. Sai token, phiếu khác, token đã hết hạn (phiếu đóng
    /// quá `AUTOXIDE_SUPPORT_TOKEN_DAYS_AFTER_CLOSE` ngày) → 404 như không có phiếu.
    pub(crate) async fn by_token(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        token: &str,
    ) -> Result<Ticket, Error> {
        let row = ticket::Entity::find_by_id(id)
            .filter(ticket::Column::TokenHash.eq(sha256(token.trim())))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| not_found(id))?;
        let expired = row.closed_at.is_some_and(|closed| {
            closed + chrono::Duration::days(self.config.token_days_after_close) < Utc::now()
        });
        if expired { Err(not_found(id)) } else { Ok(row) }
    }

    /// Xác thực liên lạc của phiếu vãng lai: không kèm `code` → gửi mã mới; kèm `code` → kiểm mã.
    /// Đúng mã thì phiếu `verified` và người mua viết tiếp được.
    pub(crate) async fn verify(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        token: &str,
        code: Option<&str>,
    ) -> Result<Ticket, Error> {
        let row = self.by_token(tenant, id, token).await?;
        if row.verified_at.is_some() {
            return Ok(row);
        }
        let Some(code) = code.map(str::trim).filter(|code| !code.is_empty()) else {
            let throttle = Throttle::new(MODULE, OTP_PER_HOUR, HOUR)?;
            if let Outcome::Limited { retry_after } =
                throttle.hit(tenant, &format!("otp:{id}")).await?
            {
                return Err(throttled(retry_after));
            }
            self.send_code(tenant, &row, None).await?;
            return self.by_token(tenant, id, token).await;
        };
        let expected = self.code_hash(tenant, id, code).await?;
        let transaction = tenant.database().begin_write().await?;
        // Khoá dòng phiếu: N lần đoán mã cùng lúc nối đuôi nhau, mỗi lần thấy số lần thử của lần
        // trước — không thì Postgres cho cả N cùng thấy 0 lần thử và giới hạn 5 lần mất tác dụng.
        let current = crate::service::lock_ticket(&transaction, tenant, id).await?;
        let live = current.otp_expires_at.is_some_and(|at| at > Utc::now())
            && current.otp_attempts < OTP_ATTEMPTS;
        let mut row = current.clone().into_active_model();
        if live && current.otp_hash.as_deref() == Some(expected.as_str()) {
            row.verified_at = Set(Some(now()));
            row.otp_hash = Set(None);
            row.otp_expires_at = Set(None);
            row.otp_attempts = Set(0);
            let after =
                ticket::Entity::update(row).validate()?.scoped(tenant).exec(&transaction).await?;
            transaction.commit().await?;
            return Ok(after);
        }
        // Sai mã: đếm lần thử (quá 5 lần thì mã cũ hết dùng được).
        row.otp_attempts = Set(current.otp_attempts.saturating_add(1));
        ticket::Entity::update(row).validate()?.scoped(tenant).exec(&transaction).await?;
        transaction.commit().await?;
        Err(code_invalid())
    }

    /// Phát mã mới (6 chữ số) cho phiếu và gửi tới email / số của người hỏi. `token`: token phiếu
    /// vừa phát (chỉ lúc tạo) để kèm link mở phiếu.
    async fn send_code(
        &self,
        tenant: &TenantContext,
        ticket: &Ticket,
        token: Option<&str>,
    ) -> Result<(), Error> {
        let recipient = match (&ticket.requester_email, &ticket.requester_phone) {
            (Some(email), _) => Recipient::Email(email.clone()),
            (None, Some(phone)) => Recipient::Phone(phone.clone()),
            (None, None) => return Ok(()),
        };
        let Some(notifier) = &self.parts.notifier else {
            return Err(verification_unavailable());
        };
        let code = random_code()?;
        let hash = self.code_hash(tenant, ticket.id, &code).await?;
        let transaction = tenant.database().begin_write().await?;
        let current = ticket::Entity::find_by_id(ticket.id)
            .scoped(tenant)
            .one(&transaction)
            .await?
            .ok_or_else(|| not_found(ticket.id))?;
        let mut row = current.into_active_model();
        row.otp_hash = Set(Some(hash));
        row.otp_expires_at = Set(Some(now() + chrono::Duration::minutes(OTP_MINUTES)));
        row.otp_attempts = Set(0);
        ticket::Entity::update(row).validate()?.scoped(tenant).exec(&transaction).await?;
        transaction.commit().await?;
        // Gửi NGOÀI transaction ghi (luật dữ liệu: transaction ghi không gọi ra ngoài).
        let mut notification = Notification::new(
            VERIFICATION_CODE.key(),
            recipient,
            json!({"code": code, "number": ticket.number}),
            format!("support.otp:{}:{}", ticket.id, Uuid::now_v7()),
        );
        if let (Some(url), Some(token)) = (&self.config.guest_url, token) {
            notification = notification
                .link(url.replace("{id}", &ticket.id.to_string()).replace("{token}", token));
        }
        if let Some(locale) = &ticket.locale {
            notification = notification.locale(locale.clone());
        }
        notifier.notify(tenant.id(), notification).await?;
        Ok(())
    }

    async fn code_hash(
        &self,
        tenant: &TenantContext,
        id: Uuid,
        code: &str,
    ) -> Result<String, Error> {
        let pepper = self.parts.secret.derive_for(tenant.id(), "support.otp").await?;
        let pepper: String = pepper.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(sha256(&format!("{pepper}:{id}:{code}")))
    }

    /// Gộp phiếu vãng lai vào tài khoản `principal`: phiếu đã xác thực cùng số điện thoại mà tài
    /// khoản đã xác thực (`compliance`), và — khi đưa token của một phiếu đã xác thực — mọi phiếu
    /// đã xác thực cùng email / số của phiếu đó. Gọi lại không đổi gì thêm. Trả số phiếu đã gộp.
    pub(crate) async fn claim(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        proof: Option<(Uuid, String)>,
    ) -> Result<u64, Error> {
        let user = principal.user_id();
        let mut any = Condition::any();
        let mut has = false;
        if let Some(accounts) = &self.parts.accounts
            && let Some(phone) = accounts.verified_phone(tenant.id(), user).await?
        {
            any = any.add(ticket::Column::RequesterPhone.eq(phone.as_str()));
            has = true;
        }
        if let Some((id, token)) = proof {
            let row = self.by_token(tenant, id, &token).await?;
            if row.verified_at.is_none() {
                return Err(verification_required_for_claim());
            }
            if let Some(email) = row.requester_email {
                any = any.add(ticket::Column::RequesterEmail.eq(email));
            }
            if let Some(phone) = row.requester_phone {
                any = any.add(ticket::Column::RequesterPhone.eq(phone));
            }
            has = true;
        }
        if !has {
            return Ok(0);
        }
        let transaction = tenant.database().begin_write().await?;
        let rows = ticket::Entity::find()
            .filter(any)
            .filter(ticket::Column::RequesterUserId.is_null())
            .filter(ticket::Column::VerifiedAt.is_not_null())
            .scoped(tenant)
            .all(&transaction)
            .await?;
        let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
        if ids.is_empty() {
            transaction.rollback().await?;
            return Ok(0);
        }
        let result = ticket::Entity::update_many()
            .col_expr(ticket::Column::RequesterUserId, sea_orm::sea_query::Expr::value(user))
            .col_expr(ticket::Column::UpdatedAt, sea_orm::sea_query::Expr::value(now()))
            .filter(ticket::Column::Id.is_in(ids.clone()))
            .filter(ticket::Column::RequesterUserId.is_null())
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let event = events::TicketsClaimed(Change {
            before: None,
            after: Some(json!({"user_id": user, "ticket_ids": ids})),
        });
        self.parts
            .outbox
            .record(&transaction, tenant, Actor::from(principal), None, &event)
            .await?;
        transaction.commit().await?;
        Ok(result.rows_affected)
    }

    /// Nhận lại (pha hai của chống spam): token form đã dùng thì trả phiếu cũ.
    pub(crate) async fn replayed(
        &self,
        tenant: &TenantContext,
        claim: Claim,
    ) -> Result<Option<Ticket>, Error> {
        let Claim::Replay { result } = claim else {
            return Ok(None);
        };
        let Some(id) = result.and_then(|raw| raw.parse::<Uuid>().ok()) else {
            return Ok(None);
        };
        Ok(ticket::Entity::find_by_id(id).scoped(tenant).one(tenant.database().read()).await?)
    }
}

/// Liên lạc của khách vãng lai: đúng một trong email / số điện thoại, đã chuẩn hoá.
fn contact(email: Option<&str>, phone: Option<&str>) -> Result<Who, Error> {
    let email = email.map(str::trim).filter(|email| !email.is_empty());
    let phone = phone.map(str::trim).filter(|phone| !phone.is_empty());
    let mut problems = Vec::new();
    let who = match (email, phone) {
        (Some(email), None) => {
            if !autoxide_kernel::email::is_valid(email) {
                problems.push(FieldError::new("email", "invalid", "is not an email address"));
            }
            Who::Email(autoxide_kernel::email::normalize(email))
        }
        (None, Some(raw)) => match PhoneNumber::parse(raw, DEFAULT_COUNTRY) {
            Some(phone) => Who::Phone(phone.as_str().to_owned()),
            None => {
                problems.push(FieldError::new("phone", "invalid", "is not a phone number"));
                Who::Phone(String::new())
            }
        },
        _ => {
            problems.push(FieldError::new(
                "email",
                "contact_required",
                "give either `email` or `phone` (not both) so we can answer you",
            ));
            Who::Email(String::new())
        }
    };
    finish(problems)?;
    Ok(who)
}

/// Phiếu giả (không lưu) trả cho bài bị chống spam bỏ đi.
fn phantom(tenant: &TenantContext, opening: &Opening) -> Ticket {
    let at = now();
    Ticket {
        id: Uuid::now_v7(),
        tenant_id: tenant.id().as_uuid(),
        org_id: opening.org_id.unwrap_or_else(|| tenant.id().as_uuid()),
        number: "000000".to_owned(),
        requester_user_id: None,
        requester_email: None,
        requester_phone: None,
        requester_name: opening.requester_name.clone(),
        locale: opening.locale.clone(),
        verified_at: None,
        token_hash: None,
        otp_hash: None,
        otp_expires_at: None,
        otp_attempts: 0,
        category: opening.category.clone(),
        priority: "normal".to_owned(),
        status: Status::Open.as_str().to_owned(),
        resolution: None,
        subject: opening.subject.clone(),
        channel: Channel::Guest.as_str().to_owned(),
        assignee_user_id: None,
        team_id: None,
        first_response_due_at: None,
        resolve_due_at: None,
        first_responded_at: None,
        sla_paused_at: None,
        sla_paused_minutes: 0,
        solved_at: None,
        closed_at: None,
        breached: false,
        case_id: None,
        dispute_id: None,
        follow_up_of: None,
        escalated_to_platform: false,
        satisfaction: None,
        satisfaction_note: None,
        auto_reply_paused: false,
        auto_replies_in_row: 0,
        last_message_at: at,
        reference: None,
        search_key: String::new(),
        created_at: at,
        updated_at: at,
    }
}

/// Mã 6 chữ số ngẫu nhiên. Mỗi ký tự base64url của token ngẫu nhiên mang 6 bit đều nhau; 10 ký tự
/// cho 60 bit, chia lấy dư cho 1 000 000 lệch không đáng kể.
fn random_code() -> Result<String, Error> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let raw = autoxide_kernel::secret::random_token(16)?;
    let mut number = 0u64;
    for byte in raw.bytes().take(10) {
        let value = ALPHABET.iter().position(|known| *known == byte).unwrap_or(0) as u64;
        number = (number << 6) | value;
    }
    Ok(format!("{:06}", number % 1_000_000))
}

fn throttled(retry_after: u64) -> Error {
    Error::new(
        ErrorKind::TooManyRequests,
        "support.too_many_requests",
        "Too many requests from you; try again in {seconds} seconds.",
    )
    .with_param("seconds", retry_after)
}

fn code_invalid() -> Error {
    Error::validation(vec![FieldError::new(
        "code",
        "invalid",
        "is wrong or expired; ask for a new code",
    )])
}

fn verification_unavailable() -> Error {
    Error::unavailable(
        "support.verification_unavailable",
        "Verification codes cannot be sent on this server (notifications are not set up).",
    )
}

fn verification_required_for_claim() -> Error {
    Error::new(
        ErrorKind::Forbidden,
        "support.verification_required",
        "Confirm the email or phone number of this ticket first: \
         `POST /api/v1/support/public/tickets/{id}/verify`.",
    )
}

fn guest_tickets_disabled() -> Error {
    Error::new(
        ErrorKind::Forbidden,
        "support.guest_tickets_disabled",
        "This shop takes support tickets from signed-in customers only: sign in first.",
    )
}
