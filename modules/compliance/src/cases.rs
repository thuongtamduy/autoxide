//! Vụ việc (bước 37, D-363): báo vi phạm sở hữu trí tuệ / hàng giả / hàng cấm và khiếu nại của người
//! tiêu dùng (D-364), tiếp nhận → ẩn đích → phản hồi của bên bị phản ánh → quyết định, có hạn và lưu vết.
//!
//! - Hạn là dữ liệu có ngày hiệu lực: quy tắc `cases.deadlines` (giờ xác nhận đã nhận, ngày làm việc để
//!   phản hồi và để quyết định theo loại, giờ với yêu cầu của cơ quan, ngày lễ, múi giờ). Ngày làm việc
//!   tính trong Rust (bỏ thứ Bảy, Chủ nhật, ngày lễ) rồi lưu mốc UTC — không dùng hàm ngày của database.
//! - Ẩn / hiện đích không đụng bảng của module khác: phát event `compliance.target_restricted` /
//!   `compliance.target_restored` (hợp đồng `autoxide_kernel::moderation`), module sở hữu tự ẩn.
//! - Người báo tra trạng thái bằng mã trả một lần lúc gửi (chỉ lưu SHA-256).
//! - Bên bị phản ánh: chủ đối tượng (sổ đối tượng, `owner_user_ids`) và người có
//!   `compliance.cases.respond` trên nút bị phản ánh.
//!
//! Căn cứ — cần đối chiếu văn bản gốc / người có chuyên môn pháp lý: Luật Thương mại điện tử 2025, Luật
//! Sở hữu trí tuệ, NĐ 85/2021/NĐ-CP (gỡ khi cơ quan yêu cầu), Luật Bảo vệ quyền lợi người tiêu dùng
//! 19/2023/QH15 và NĐ 55/2024/NĐ-CP (khiếu nại).

use std::collections::BTreeSet;
use std::sync::Arc;

use autoxide_kernel::access::{OrgTarget, Principal};
use autoxide_kernel::antispam::{Claim, Limits, Submission as Check, Verdict};
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::http::ClientInfo;
use autoxide_kernel::i18n::Language;
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::mail::MailTemplate;
use autoxide_kernel::moderation::Restriction;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, TenantId};
use autoxide_kernel::throttle::Throttle;
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc, Weekday};
use sea_orm::sea_query::{Expr, Query};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter,
    QueryOrder, QuerySelect,
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::{case, case_entry, case_respondent};
use crate::events;
use crate::permissions::{CASES_HANDLE, CASES_READ, CASES_RESPOND};
use crate::service::{ComplianceService, now, require_session};

mod decide;
mod mail;
mod reminders;
mod respond;

/// Lịch nhắc vụ việc quá hạn (hằng giờ).
pub(crate) const CASE_DEADLINES: &str = "compliance.case_deadlines";

pub(crate) const KINDS: [&str; 5] =
    ["ip_infringement", "counterfeit", "prohibited_goods", "consumer_complaint", "other"];
const REPORTER_KINDS: [&str; 4] = ["guest", "user", "rights_holder", "authority"];
const DECISIONS: [&str; 3] = ["upheld", "rejected", "partially_upheld"];
/// Trạng thái còn mở.
const OPEN: [&str; 3] = ["received", "under_review", "awaiting_response"];
/// Loại được tự ẩn khi khách bật chính sách `cases.auto_restrict`.
const AUTO_RESTRICT_KINDS: [&str; 2] = ["ip_infringement", "counterfeit"];
/// Phạm vi token form của hai kênh công khai (`GET /api/v1/antispam/token?scope=…`).
pub(crate) const REPORT_SCOPE: &str = "compliance:reports";
pub(crate) const COMPLAINT_SCOPE: &str = "compliance:complaints";
const CLAIM_MAX: usize = 20_000;
const RIGHTS_MAX: usize = 20;
const HOUR: std::time::Duration = std::time::Duration::from_secs(3600);

pub(crate) const RECEIVED_MAIL: MailTemplate = MailTemplate::new(
    "compliance.mail.case_received",
    "We received your report {reference}",
    "We received your report or complaint {reference} and will acknowledge it by {ack_due} and \
     decide by {decide_by} (UTC).\n\nFollow it here (keep this link private): {link}\n",
);

pub(crate) const DECIDED_MAIL: MailTemplate = MailTemplate::new(
    "compliance.mail.case_decided",
    "Decision on your report {reference}",
    "Your report or complaint {reference} was decided: {decision}.\n\n{reason}\n\nFollow it with the \
     link of the receipt mail.\n",
);

pub(crate) const OPENED_MAIL: MailTemplate = MailTemplate::new(
    "compliance.mail.case_opened",
    "New {kind} case {reference}",
    "A new case was received: {kind}, target {target}.\n\nAcknowledge it by {ack_due} and decide by \
     {decide_by} (UTC): GET /api/v1/compliance/cases/{id}\n",
);

pub(crate) const OVERDUE_MAIL: MailTemplate = MailTemplate::new(
    "compliance.mail.case_overdue",
    "Case {reference} is past its deadline",
    "The case {reference} ({kind}) is past a deadline ({step}: {due}, UTC).\n\n\
     GET /api/v1/compliance/cases/{id}\n",
);

pub(crate) fn case_not_found() -> Error {
    Error::not_found("compliance.case_not_found", "There is no such case.")
}

fn state_invalid(status: &str) -> Error {
    Error::conflict(
        "compliance.case_state_invalid",
        "The case is {status}; this cannot be done now.",
    )
    .with_param("status", status)
}

fn response_closed() -> Error {
    Error::conflict(
        "compliance.response_closed",
        "The time to answer this case is over, or the case is decided.",
    )
}

fn target_not_found(target: &str) -> Error {
    Error::validation(vec![
        FieldError::new("target", "not_found", "there is no `{target}` in this tenant")
            .with_param("target", target),
    ])
}

/// Mã tham chiếu ngắn cho người đọc (8 ký tự cuối của ID).
pub(crate) fn reference(id: Uuid) -> String {
    let text = id.simple().to_string();
    format!("C-{}", text[text.len() - 8..].to_uppercase())
}

// ---- Hạn xử lý -----------------------------------------------------------------------------------

/// Hạn của một loại vụ việc theo quy tắc `cases.deadlines`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Deadlines {
    pub ack_hours: i64,
    pub respond_days: i64,
    pub decide_days: i64,
    /// Với yêu cầu của cơ quan có thẩm quyền: phải quyết (gỡ) trong chừng này giờ.
    pub authority_hours: i64,
    pub holidays: BTreeSet<NaiveDate>,
    /// Múi giờ tính ngày làm việc (phút so với UTC; Việt Nam +420).
    pub utc_offset_minutes: i64,
}

pub(crate) fn deadlines(body: Option<&Value>, kind: &str) -> Deadlines {
    let number = |value: Option<&Value>, default: i64| {
        value.and_then(Value::as_i64).filter(|number| *number > 0).unwrap_or(default)
    };
    let per_kind = body.and_then(|body| body.get("kinds")?.get(kind));
    Deadlines {
        ack_hours: number(body.and_then(|body| body.get("ack_hours")), 24),
        respond_days: number(per_kind.and_then(|item| item.get("respond_working_days")), 5),
        decide_days: number(per_kind.and_then(|item| item.get("decide_working_days")), 10),
        authority_hours: number(body.and_then(|body| body.get("authority_decide_hours")), 24),
        holidays: body
            .and_then(|body| body.get("holidays"))
            .and_then(Value::as_array)
            .map(|items| {
                items.iter().filter_map(|item| item.as_str()?.parse::<NaiveDate>().ok()).collect()
            })
            .unwrap_or_default(),
        utc_offset_minutes: body
            .and_then(|body| body.get("utc_offset_minutes"))
            .and_then(Value::as_i64)
            .filter(|minutes| minutes.abs() <= 14 * 60)
            .unwrap_or(420),
    }
}

/// `start` cộng `days` ngày làm việc (giờ trong ngày giữ nguyên theo giờ địa phương): bỏ thứ Bảy,
/// Chủ nhật và ngày lễ trong `holidays`.
pub(crate) fn add_working_days(
    start: DateTime<Utc>,
    days: i64,
    holidays: &BTreeSet<NaiveDate>,
    utc_offset_minutes: i64,
) -> DateTime<Utc> {
    let offset = Duration::minutes(utc_offset_minutes);
    let local = start.naive_utc() + offset;
    let mut date = local.date();
    let mut counted = 0;
    // Tối đa vài năm: dữ liệu ngày lễ hỏng không làm treo vòng lặp.
    for _ in 0..3660 {
        if counted >= days {
            break;
        }
        let Some(next) = date.succ_opt() else { break };
        date = next;
        if !matches!(date.weekday(), Weekday::Sat | Weekday::Sun) && !holidays.contains(&date) {
            counted += 1;
        }
    }
    DateTime::from_naive_utc_and_offset(date.and_time(local.time()) - offset, Utc)
}

// ---- Đầu vào -------------------------------------------------------------------------------------

/// Người báo.
#[derive(Debug, Clone, Default)]
pub(crate) struct Reporter {
    pub kind: String,
    pub user_id: Option<Uuid>,
    pub name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
}

/// Một vụ việc mới.
#[derive(Debug, Clone, Default)]
pub(crate) struct NewCase {
    pub kind: String,
    pub target: Option<String>,
    pub respondent_org_id: Option<Uuid>,
    pub reporter: Reporter,
    pub claim: String,
    pub rights: Vec<Value>,
}

/// Bài gửi qua kênh công khai (kèm các lớp chống spam).
#[derive(Debug, Clone, Default)]
pub(crate) struct PublicCase {
    pub case: NewCase,
    pub form_token: Option<String>,
    pub trap: Option<String>,
    pub challenge: Option<String>,
}

/// Kết quả tiếp nhận: vụ việc và mã tra cứu (chỉ trả lần này).
#[derive(Debug, Clone)]
pub(crate) struct Opened {
    pub case: case::Model,
    pub token: String,
}

/// Vụ việc kèm lưu vết và bên bị phản ánh.
#[derive(Debug, Clone)]
pub(crate) struct CaseDetail {
    pub case: case::Model,
    pub entries: Vec<case_entry::Model>,
    pub respondents: Vec<Uuid>,
}

/// Bộ lọc danh sách.
#[derive(Debug, Clone, Default)]
pub(crate) struct CaseFilter {
    pub status: Option<String>,
    pub kind: Option<String>,
    pub target: Option<String>,
    pub overdue: bool,
    /// `true`: chỉ vụ lớp chống spam giữ lại; `false` (mặc định): bỏ chúng.
    pub held: bool,
}

pub(crate) fn check_case(input: &NewCase, problems: &mut Vec<FieldError>) {
    if !KINDS.contains(&input.kind.as_str()) {
        problems.push(FieldError::new(
            "kind",
            "invalid",
            "must be `ip_infringement`, `counterfeit`, `prohibited_goods`, `consumer_complaint` or `other`",
        ));
    }
    if !REPORTER_KINDS.contains(&input.reporter.kind.as_str()) {
        problems.push(FieldError::new(
            "reporter_kind",
            "invalid",
            "must be `guest`, `user`, `rights_holder` or `authority`",
        ));
    }
    let claim = input.claim.trim().chars().count();
    if !(10..=CLAIM_MAX).contains(&claim) {
        problems.push(
            FieldError::new("claim", "length", "must be {min} to {max} characters")
                .with_param("min", 10)
                .with_param("max", CLAIM_MAX),
        );
    }
    if input.reporter.name.as_deref().is_some_and(|name| name.trim().chars().count() > 200) {
        problems.push(FieldError::new("name", "length", "must be at most 200 characters"));
    }
    if input.reporter.email.as_deref().is_some_and(|email| !autoxide_kernel::email::is_valid(email))
    {
        problems.push(FieldError::new("email", "email", "must be an email address"));
    }
    if input.reporter.phone.as_deref().is_some_and(|phone| phone.trim().chars().count() > 32) {
        problems.push(FieldError::new("phone", "length", "must be at most 32 characters"));
    }
    // Người lạ phải để lại cách liên lạc: không thì không báo kết quả được.
    if matches!(input.reporter.kind.as_str(), "guest" | "rights_holder" | "authority")
        && input.reporter.user_id.is_none()
    {
        if input.reporter.name.as_deref().is_none_or(|name| name.trim().is_empty()) {
            problems.push(FieldError::new("name", "required", "is required"));
        }
        if input.reporter.email.is_none() && input.reporter.phone.is_none() {
            problems.push(FieldError::new(
                "email",
                "required",
                "give an email address or a phone number to be told the outcome",
            ));
        }
    }
    if input.rights.len() > RIGHTS_MAX
        || input.rights.iter().any(|item| !item.is_object())
        || json!(input.rights).to_string().len() > 20_000
    {
        problems.push(
            FieldError::new(
                "rights",
                "invalid",
                "must be at most {max} objects (certificates, links), 20000 bytes in all",
            )
            .with_param("max", RIGHTS_MAX),
        );
    }
    if input.kind == "consumer_complaint" && input.reporter.kind == "authority" {
        problems.push(FieldError::new(
            "kind",
            "invalid",
            "a consumer complaint comes from a consumer, not an authority",
        ));
    }
    match input.target.as_deref() {
        None => {
            if input.kind != "consumer_complaint" && input.kind != "other" {
                problems.push(FieldError::new(
                    "target",
                    "required",
                    "is required: the reported object, as `<module>.<type>:<id>`",
                ));
            }
        }
        Some(target) => {
            if parse_target(target).is_none() {
                problems.push(FieldError::new(
                    "target",
                    "invalid",
                    "must be `<module>.<type>:<id>`, at most 200 characters",
                ));
            }
        }
    }
}

/// (`<module>.<loại>`, `id`) của một đích.
pub(crate) fn parse_target(target: &str) -> Option<(&str, &str)> {
    if target.len() > 200 {
        return None;
    }
    let (kind, id) = target.split_once(':')?;
    let (module, name) = kind.split_once('.')?;
    let word = |text: &str| {
        !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_lowercase() || byte == b'_')
    };
    let id_ok = !id.is_empty()
        && id.len() <= 100
        && id.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte));
    (word(module) && word(name) && id_ok).then_some((kind, id))
}

fn payload(row: &case::Model) -> Value {
    json!({
        "case_id": row.id, "kind": row.kind, "status": row.status, "target": row.target,
        "reporter_kind": row.reporter_kind, "restricted": row.restricted, "decision": row.decision,
        "respondent_org_id": row.respondent_org_id,
    })
}

fn entry(
    tenant: &TenantContext,
    case_id: Uuid,
    kind: &str,
    author_kind: &str,
    author: Option<Uuid>,
    body: impl Into<String>,
    files: &[Uuid],
) -> case_entry::Model {
    case_entry::Model {
        id: Uuid::now_v7(),
        tenant_id: tenant.id().as_uuid(),
        case_id,
        kind: kind.to_owned(),
        author_kind: author_kind.to_owned(),
        author_user_id: author,
        body: body.into(),
        files: json!(files).to_string(),
        created_at: now(),
    }
}

fn check_body(body: &str, files: &[Uuid]) -> Result<(), Error> {
    let mut problems = Vec::new();
    if body.trim().is_empty() || body.chars().count() > CLAIM_MAX {
        problems.push(
            FieldError::new("body", "length", "must be 1 to {max} characters")
                .with_param("max", CLAIM_MAX),
        );
    }
    if files.len() > 20 {
        problems.push(FieldError::new("files", "length", "must be at most 20 file ids"));
    }
    if problems.is_empty() { Ok(()) } else { Err(Error::validation(problems)) }
}

impl ComplianceService {
    async fn case_deadlines(
        &self,
        tenant: &TenantContext,
        kind: &str,
        at: DateTime<Utc>,
    ) -> Result<Deadlines, Error> {
        let rule = self.rule_for(tenant, "cases.deadlines", at).await?;
        let body = rule.and_then(|row| serde_json::from_str::<Value>(&row.body).ok());
        Ok(deadlines(body.as_ref(), kind))
    }

    /// Hạn của loại `kind` (cho kênh khiếu nại công khai): ngày làm việc để phản hồi, để quyết định.
    pub(crate) async fn deadlines_of(
        &self,
        tenant: &TenantContext,
        kind: &str,
    ) -> Result<Deadlines, Error> {
        self.case_deadlines(tenant, kind, Utc::now()).await
    }

    /// Ghi vụ việc mới; `spam` là lý do lớp chống spam giữ lại (khi đó không tự ẩn, không gửi mail).
    pub(crate) async fn insert_case(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        channel: &str,
        input: NewCase,
        spam: Option<String>,
        claim: Option<&autoxide_kernel::antispam::Checked>,
    ) -> Result<Option<Opened>, Error> {
        // Đích phải có thật (sổ đối tượng, ngoài transaction); bên bị phản ánh là chủ đối tượng.
        let mut owners = Vec::new();
        if let Some(target) = input.target.as_deref()
            && let Some((kind, id)) = parse_target(target)
        {
            if !self.resources.knows(kind) {
                return Err(target_not_found(target));
            }
            match self.resources.resolve(tenant, kind, id).await? {
                Some(resource) => owners = resource.owner_user_ids,
                None => return Err(target_not_found(target)),
            }
        }
        let at = now();
        let rules = self.case_deadlines(tenant, &input.kind, at).await?;
        let authority = input.reporter.kind == "authority";
        let decide_by = if authority {
            at + Duration::hours(rules.authority_hours)
        } else {
            add_working_days(at, rules.decide_days, &rules.holidays, rules.utc_offset_minutes)
        };
        let respond_by =
            add_working_days(at, rules.respond_days, &rules.holidays, rules.utc_offset_minutes)
                .min(decide_by);
        let auto = self
            .policy(tenant, "cases.auto_restrict")
            .await?
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let target = input.target.clone().unwrap_or_default();
        let restrict = spam.is_none()
            && !target.is_empty()
            && (authority || (auto && AUTO_RESTRICT_KINDS.contains(&input.kind.as_str())));
        let token = autoxide_kernel::secret::random_token(24)?;
        let has_respondent = !owners.is_empty() || input.respondent_org_id.is_some();
        let status = if spam.is_some() {
            "received"
        } else if has_respondent {
            "awaiting_response"
        } else {
            "under_review"
        };
        let trim = |value: Option<String>| {
            value.map(|value| value.trim().to_owned()).filter(|value| !value.is_empty())
        };
        let row = case::Model {
            id: Uuid::now_v7(),
            tenant_id: tenant.id().as_uuid(),
            kind: input.kind,
            target,
            respondent_org_id: input.respondent_org_id,
            reporter_kind: input.reporter.kind,
            reporter_user_id: input.reporter.user_id,
            reporter_name: trim(input.reporter.name),
            reporter_email: trim(input.reporter.email)
                .map(|email| autoxide_kernel::email::normalize(&email)),
            reporter_phone: trim(input.reporter.phone),
            channel: channel.to_owned(),
            claim: input.claim.trim().to_owned(),
            rights: json!(input.rights).to_string(),
            status: status.to_owned(),
            restricted: restrict,
            decision: String::new(),
            decision_reason: None,
            ack_due_at: at + Duration::hours(rules.ack_hours),
            acknowledged_at: None,
            respond_by,
            decide_by,
            decided_at: None,
            decided_by: None,
            tracking_hash: autoxide_kernel::secret::sha256(&token),
            spam_reason: spam,
            overdue_notified: String::new(),
            created_by: actor.user_id,
            created_at: at,
            updated_at: at,
        };
        let transaction = tenant.database().begin_write().await?;
        if restrict {
            // Cùng khoá với lệnh khôi phục của vụ khác trên cùng đích (xem `save_case`).
            lock_target(&transaction, tenant, &row.target).await?;
        }
        let saved = case::ActiveModel::from(row).insert(&transaction).await?;
        if let Some(checked) = claim
            && let Claim::Replay { .. } = checked
                .claim(&transaction, tenant, "compliance", Some(&saved.id.to_string()))
                .await?
        {
            // Cùng token form đã gửi rồi: không tạo vụ thứ hai.
            transaction.rollback().await?;
            return Ok(None);
        }
        let mut seen = BTreeSet::new();
        for user in owners {
            if seen.insert(user) {
                case_respondent::ActiveModel::from(case_respondent::Model {
                    tenant_id: tenant.id().as_uuid(),
                    case_id: saved.id,
                    user_id: user,
                })
                .insert(&transaction)
                .await?;
            }
        }
        case_entry::ActiveModel::from(entry(
            tenant,
            saved.id,
            "status",
            "system",
            None,
            saved.status.clone(),
            &[],
        ))
        .insert(&transaction)
        .await?;
        let subject = Some(format!("compliance.case:{}", saved.id));
        let mut opened = payload(&saved);
        opened["respondent_user_ids"] = json!(seen);
        self.outbox
            .record(&transaction, tenant, actor, subject.clone(), &events::CaseOpened(opened))
            .await?;
        if restrict {
            case_entry::ActiveModel::from(entry(
                tenant,
                saved.id,
                "restriction",
                "system",
                None,
                if authority {
                    "restricted: request of a competent authority"
                } else {
                    "restricted: tenant policy `cases.auto_restrict`"
                },
                &[],
            ))
            .insert(&transaction)
            .await?;
            let notice = Restriction::new(saved.id, saved.target.clone(), saved.kind.clone());
            self.outbox
                .record(
                    &transaction,
                    tenant,
                    actor,
                    subject,
                    &events::TargetRestricted(json!(notice)),
                )
                .await?;
        }
        transaction.commit().await?;
        Ok(Some(Opened { case: saved, token }))
    }

    /// Nhân sự nhập vụ việc (vd văn bản của cơ quan, thư của chủ quyền). Cần `compliance.cases.handle`.
    pub(crate) async fn open_case(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewCase,
    ) -> Result<Opened, Error> {
        principal.require(&CASES_HANDLE)?;
        let mut problems = Vec::new();
        check_case(&input, &mut problems);
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let opened = self
            .insert_case(tenant, Actor::from(principal), "staff", input, None, None)
            .await?
            .ok_or_else(|| Error::internal("a staff case cannot be a replay"))?;
        self.mail_opened(tenant, &opened).await;
        Ok(opened)
    }

    /// Kênh công khai: báo vi phạm (`complaint = false`) hay khiếu nại của người tiêu dùng.
    /// Trả lời như nhau dù lớp chống spam nhận, giữ lại hay bỏ (bot không phân biệt được).
    pub(crate) async fn submit_public_case(
        &self,
        tenant: &TenantContext,
        principal: Option<&Principal>,
        client: &ClientInfo,
        complaint: bool,
        input: PublicCase,
    ) -> Result<(Uuid, String), Error> {
        let PublicCase { mut case, form_token, trap, challenge } = input;
        let mut problems = Vec::new();
        if complaint {
            case.kind = "consumer_complaint".to_owned();
        } else if case.kind == "consumer_complaint" {
            problems.push(FieldError::new(
                "kind",
                "invalid",
                "send consumer complaints to `POST /public/complaints`",
            ));
        }
        if case.reporter.kind != "rights_holder" {
            case.reporter.kind = if principal.is_some() { "user" } else { "guest" }.to_owned();
        }
        case.reporter.user_id = principal.map(Principal::user_id);
        check_case(&case, &mut problems);
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let scope = if complaint { COMPLAINT_SCOPE } else { REPORT_SCOPE };
        let limits = Limits::default()
            .ip(Some(Throttle::new("compliance", self.public_reports_per_hour, HOUR)?))
            .email(Some(Throttle::new("compliance", self.public_reports_per_hour, HOUR)?));
        let checked = self
            .guard
            .check(
                tenant,
                &Check::new(scope, client)
                    .principal(principal.filter(|principal| principal.has(&CASES_HANDLE)))
                    .honeypot(trap.as_deref())
                    .form_token(form_token.as_deref())
                    .challenge(challenge.as_deref())
                    .email(case.reporter.email.as_deref())
                    .content(Some(&case.claim)),
                &limits,
            )
            .await?;
        let spam = match &checked.verdict {
            Verdict::Discard { reason } => {
                tracing::info!(reason, "public case dropped");
                return Ok((Uuid::now_v7(), autoxide_kernel::secret::random_token(24)?));
            }
            Verdict::Accept => None,
            Verdict::Quarantine { reasons } => Some(reasons.join(",")),
        };
        let actor = principal.map_or_else(Actor::public, Actor::from);
        let held = spam.is_some();
        match self.insert_case(tenant, actor, "public", case, spam, Some(&checked)).await? {
            Some(opened) => {
                if !held {
                    self.mail_opened(tenant, &opened).await;
                }
                Ok((opened.case.id, opened.token))
            }
            // Gửi lại cùng token: không lộ mã của lần đầu.
            None => Ok((Uuid::now_v7(), autoxide_kernel::secret::random_token(24)?)),
        }
    }

    pub(crate) async fn case_detail(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<CaseDetail, Error> {
        principal.require(&CASES_READ)?;
        let row = self.find_case(tenant, id).await?;
        self.detail(tenant, row).await
    }

    /// Vụ việc kèm lưu vết, không kiểm quyền: chỉ để trả lời một lệnh vừa kiểm quyền xong (người chỉ
    /// có `compliance.cases.handle` vẫn thấy kết quả việc mình làm).
    pub(crate) async fn case_after(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<CaseDetail, Error> {
        let row = self.find_case(tenant, id).await?;
        self.detail(tenant, row).await
    }

    async fn find_case(&self, tenant: &TenantContext, id: Uuid) -> Result<case::Model, Error> {
        case::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(case_not_found)
    }

    async fn detail(&self, tenant: &TenantContext, row: case::Model) -> Result<CaseDetail, Error> {
        let read = tenant.database().read();
        let entries = case_entry::Entity::find()
            .scoped(tenant)
            .filter(case_entry::Column::CaseId.eq(row.id))
            .order_by_asc(case_entry::Column::Id)
            .all(read)
            .await?;
        let respondents = case_respondent::Entity::find()
            .scoped(tenant)
            .filter(case_respondent::Column::CaseId.eq(row.id))
            .all(read)
            .await?
            .into_iter()
            .map(|item| item.user_id)
            .collect();
        Ok(CaseDetail { case: row, entries, respondents })
    }

    pub(crate) async fn list_cases(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        filter: &CaseFilter,
        page: &PageRequest,
    ) -> Result<Page<case::Model>, Error> {
        principal.require(&CASES_READ)?;
        let mut select = case::Entity::find().scoped(tenant);
        if let Some(status) = filter.status.as_deref() {
            select = select.filter(case::Column::Status.eq(status));
        }
        if let Some(kind) = filter.kind.as_deref() {
            select = select.filter(case::Column::Kind.eq(kind));
        }
        if let Some(target) = filter.target.as_deref() {
            select = select.filter(case::Column::Target.eq(target));
        }
        select = if filter.held {
            select.filter(case::Column::SpamReason.is_not_null())
        } else {
            select.filter(case::Column::SpamReason.is_null())
        };
        if filter.overdue {
            select = select
                .filter(case::Column::Status.is_in(OPEN))
                .filter(case::Column::DecideBy.lt(Utc::now()));
        }
        fetch_page(select, case::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    /// Ghi `after` (đúng phiên bản `before` đã đọc), thêm các dòng lưu vết và event, trong một
    /// transaction. `restriction`: `Some(true)` phát `target_restricted`, `Some(false)` phát
    /// `target_restored` (chỉ khi không còn vụ nào khác đang ẩn cùng đích).
    #[allow(clippy::too_many_arguments)]
    async fn save_case(
        &self,
        tenant: &TenantContext,
        actor: Actor,
        before: &case::Model,
        mut after: case::Model,
        entries: Vec<case_entry::Model>,
        restriction: Option<bool>,
        decided: bool,
    ) -> Result<case::Model, Error> {
        after.updated_at = now();
        let transaction = tenant.database().begin_write().await?;
        if restriction.is_some() {
            // Khoá mọi vụ cùng đích trước khi xét "còn vụ khác đang ẩn" (D-181): hai vụ cùng ẩn
            // một đích được khôi phục cùng lúc từng cùng thấy vụ kia còn ẩn trên Postgres, nên
            // không bên nào phát `target_restored` — nội dung bị ẩn vĩnh viễn.
            lock_target(&transaction, tenant, &after.target).await?;
        }
        let result = case::Entity::update_many()
            .set(case::ActiveModel::from(after.clone()).reset_all())
            .filter(case::Column::TenantId.eq(tenant.id().as_uuid()))
            .filter(case::Column::Id.eq(after.id))
            .filter(case::Column::UpdatedAt.eq(before.updated_at))
            .exec(&transaction)
            .await?;
        if result.rows_affected == 0 {
            return Err(autoxide_kernel::version::changed());
        }
        for item in entries {
            case_entry::ActiveModel::from(item).insert(&transaction).await?;
        }
        let subject = Some(format!("compliance.case:{}", after.id));
        let notice = Restriction::new(after.id, after.target.clone(), after.kind.clone());
        match restriction {
            Some(true) => {
                self.outbox
                    .record(
                        &transaction,
                        tenant,
                        actor,
                        subject.clone(),
                        &events::TargetRestricted(json!(notice)),
                    )
                    .await?;
            }
            Some(false) if !self.restricted_elsewhere(&transaction, tenant, &after).await? => {
                self.outbox
                    .record(
                        &transaction,
                        tenant,
                        actor,
                        subject.clone(),
                        &events::TargetRestored(json!(notice)),
                    )
                    .await?;
            }
            _ => {}
        }
        let mut change = payload(&after);
        change["previous_status"] = json!(before.status);
        if decided {
            self.outbox
                .record(&transaction, tenant, actor, subject, &events::CaseDecided(change))
                .await?;
        } else {
            self.outbox
                .record(&transaction, tenant, actor, subject, &events::CaseUpdated(change))
                .await?;
        }
        transaction.commit().await?;
        Ok(after)
    }

    /// Còn vụ khác đang ẩn cùng đích không (thì không hiện lại). Gọi sau [`lock_target`].
    async fn restricted_elsewhere<C: ConnectionTrait>(
        &self,
        connection: &C,
        tenant: &TenantContext,
        row: &case::Model,
    ) -> Result<bool, Error> {
        Ok(case::Entity::find()
            .scoped(tenant)
            .filter(case::Column::Target.eq(row.target.as_str()))
            .filter(case::Column::Restricted.eq(true))
            .filter(case::Column::Id.ne(row.id))
            .one(connection)
            .await?
            .is_some())
    }

    /// Việc đầu tiên của người xử lý trên vụ vừa nhận: ghi đã xác nhận, nhả vụ lớp chống spam giữ.
    fn acknowledge(
        &self,
        row: &mut case::Model,
        entries: &mut Vec<case_entry::Model>,
        tenant: &TenantContext,
    ) {
        if row.acknowledged_at.is_none() {
            row.acknowledged_at = Some(now());
        }
        if row.status == "received" {
            row.spam_reason = None;
            row.status = "under_review".to_owned();
            entries.push(entry(tenant, row.id, "status", "system", None, "under_review", &[]));
        }
    }

    /// Thêm ghi chú nội bộ hay chứng cứ. Cần `compliance.cases.handle`.
    pub(crate) async fn add_case_entry(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        kind: &str,
        body: &str,
        files: &[Uuid],
    ) -> Result<case_entry::Model, Error> {
        principal.require(&CASES_HANDLE)?;
        let before = self.find_case(tenant, id).await?;
        if !["note", "evidence"].contains(&kind) {
            return Err(Error::validation(vec![FieldError::new(
                "kind",
                "invalid",
                "must be `note` or `evidence`",
            )]));
        }
        check_body(body, files)?;
        let line = entry(tenant, id, kind, "staff", Some(principal.user_id()), body, files);
        let mut after = before.clone();
        let mut entries = Vec::new();
        self.acknowledge(&mut after, &mut entries, tenant);
        entries.push(line.clone());
        self.save_case(tenant, Actor::from(principal), &before, after, entries, None, false)
            .await?;
        Ok(line)
    }
}

/// Lịch `compliance.case_deadlines`: nhắc vụ quá hạn, dọn nonce hết hạn của token form.
pub(crate) struct DeadlinesJob(pub Arc<ComplianceService>);

#[async_trait::async_trait]
impl JobHandler for DeadlinesJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.guard.purge_nonces(job.database.write(), "compliance").await?;
        self.0.remind_cases(&job.database).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn han_xu_ly_tinh_ngay_lam_viec() {
        // Thứ Sáu 01/5/2026 17:00 giờ Việt Nam (lễ), 5 ngày làm việc: bỏ T7, CN, và lễ 01/5 → thứ Sáu 08/5.
        let start: DateTime<Utc> = "2026-04-30T10:00:00Z".parse().unwrap();
        let holidays: BTreeSet<NaiveDate> =
            ["2026-05-01"].iter().map(|day| day.parse().unwrap()).collect();
        let due = add_working_days(start, 5, &holidays, 420);
        assert_eq!(due, "2026-05-08T10:00:00Z".parse::<DateTime<Utc>>().unwrap());
        // Không có ngày lễ: thứ Năm 30/4 + 1 ngày làm việc = thứ Sáu 01/5.
        let next = add_working_days(start, 1, &BTreeSet::new(), 420);
        assert_eq!(next, "2026-05-01T10:00:00Z".parse::<DateTime<Utc>>().unwrap());
        // Giờ địa phương sang ngày khác giờ UTC: 23:00 UTC thứ Sáu = 06:00 thứ Bảy ở Việt Nam →
        // 1 ngày làm việc là thứ Hai.
        let late: DateTime<Utc> = "2026-05-08T23:00:00Z".parse().unwrap();
        assert_eq!(
            add_working_days(late, 1, &BTreeSet::new(), 420),
            "2026-05-10T23:00:00Z".parse::<DateTime<Utc>>().unwrap()
        );
    }

    #[test]
    fn han_doc_tu_quy_tac_theo_loai() {
        let body = json!({
            "ack_hours": 12, "authority_decide_hours": 24, "holidays": ["2026-09-02", "nope"],
            "kinds": {"consumer_complaint": {"respond_working_days": 3, "decide_working_days": 7}},
        });
        let rules = deadlines(Some(&body), "consumer_complaint");
        assert_eq!((rules.ack_hours, rules.respond_days, rules.decide_days), (12, 3, 7));
        assert_eq!(rules.holidays.len(), 1);
        let fallback = deadlines(None, "other");
        assert_eq!(
            (fallback.ack_hours, fallback.decide_days, fallback.utc_offset_minutes),
            (24, 10, 420)
        );
    }

    #[test]
    fn dich_dung_dang() {
        assert_eq!(parse_target("content.entry:0190abcd"), Some(("content.entry", "0190abcd")));
        assert_eq!(parse_target("comments.comment:1-2"), Some(("comments.comment", "1-2")));
        assert_eq!(parse_target("content:1"), None);
        assert_eq!(parse_target("Content.entry:1"), None);
        assert_eq!(parse_target("content.entry:"), None);
        assert_eq!(parse_target("content.entry:a b"), None);
    }
}

/// Khoá mọi vụ của đích `target` tới hết `transaction` ghi, theo ID tăng dần (không deadlock).
async fn lock_target(
    transaction: &sea_orm::DatabaseTransaction,
    tenant: &TenantContext,
    target: &str,
) -> Result<(), Error> {
    if target.is_empty() {
        return Ok(());
    }
    autoxide_kernel::rows::lock(
        transaction,
        case::Entity::find().scoped(tenant).filter(case::Column::Target.eq(target)),
        case::Column::Id,
    )
    .await?;
    Ok(())
}
