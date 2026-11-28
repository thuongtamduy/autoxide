//! Sổ yêu cầu của chủ thể dữ liệu đến qua kênh khác — email, thư, điện thoại, cơ quan (D-217, D-360).
//! Yêu cầu người dùng tự làm trong tài khoản (xuất, xoá) chạy ngay, không qua sổ này.
//!
//! - Hạn trả lời là dữ liệu có ngày hiệu lực: quy tắc `privacy.subject_requests`
//!   (`respond_within_hours` theo loại; Luật 91/2025/QH15, NĐ 356/2025/NĐ-CP — cần đối chiếu văn bản gốc).
//! - `fulfil` dùng cơ chế sẵn có: `access` của người dùng → bản xuất dữ liệu cá nhân của mọi module
//!   (`PersonalDataExport`, trả trong response, không lưu); `erasure` của email / số → bỏ giá trị trong sổ
//!   đồng ý và số đã xác thực; xoá cả tài khoản là `DELETE /api/v1/auth/users/{id}` (phát
//!   `auth.user_deleted` tới mọi module).

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::sms::PhoneNumber;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::api::ConsentSubject;
use crate::entity::{consent, subject_request};
use crate::events;
use crate::permissions::PRIVACY_MANAGE;
use crate::service::{ComplianceService, now};

const KINDS: [&str; 6] =
    ["access", "erasure", "rectification", "objection", "restriction", "withdrawal"];
const CHANNELS: [&str; 5] = ["email", "post", "phone", "authority", "other"];

pub(crate) fn subject_request_not_found() -> Error {
    Error::not_found(
        "compliance.subject_request_not_found",
        "There is no such data subject request.",
    )
}

fn subject_request_closed(status: &str) -> Error {
    Error::conflict(
        "compliance.subject_request_closed",
        "The request is already {status}; it cannot change any more.",
    )
    .with_param("status", status)
}

/// Yêu cầu mới.
#[derive(Debug, Clone)]
pub(crate) struct NewRequest {
    pub kind: String,
    pub channel: String,
    pub subject: String,
    pub received_at: Option<Timestamp>,
    pub notes: Option<String>,
}

/// Kết quả của `fulfil`.
#[derive(Debug, Clone)]
pub(crate) struct Fulfilled {
    pub request: subject_request::Model,
    /// Dữ liệu cá nhân (yêu cầu `access`), theo module.
    pub export: Option<Value>,
    /// Số bản ghi đã xoá / bỏ giá trị (yêu cầu `erasure`).
    pub erased: Option<u64>,
    /// Việc còn lại con người phải làm.
    pub next_steps: Option<String>,
}

/// (`subject_kind`, giá trị đã chuẩn hoá) của đối tượng: ID người dùng, email hay số điện thoại.
fn parse_subject(raw: &str) -> Option<(&'static str, String)> {
    let raw = raw.trim();
    if let Ok(id) = raw.parse::<Uuid>() {
        return Some(("user", id.to_string()));
    }
    if autoxide_kernel::email::is_valid(raw) {
        return Some(("email", raw.to_lowercase()));
    }
    PhoneNumber::parse(raw, "84").map(|phone| ("phone", phone.as_str().to_owned()))
}

fn consent_subject(row: &subject_request::Model) -> Option<ConsentSubject> {
    match row.subject_kind.as_str() {
        "email" => Some(ConsentSubject::Email(row.subject.clone())),
        "phone" => Some(ConsentSubject::Phone(row.subject.clone())),
        "user" => row.subject.parse().ok().map(ConsentSubject::User),
        _ => None,
    }
}

/// Hạn trả lời và hạn thực hiện theo thân quy tắc `body` (xem [`ComplianceService::request_deadlines`]).
fn deadlines(
    body: Option<&Value>,
    kind: &str,
    received_at: Timestamp,
) -> (Timestamp, Option<Timestamp>) {
    let hours = body
        .and_then(|body| body.get("respond_within_hours")?.get(kind)?.as_i64())
        .filter(|hours| *hours > 0)
        .unwrap_or(72);
    let due_at = received_at + SignedDuration::from_hours(hours);
    let days = body
        .and_then(|body| body.get("fulfil_within_days")?.get(kind)?.as_i64())
        .filter(|days| *days > 0);
    let fulfil_due_at = days.map(|days| {
        let unit = body.and_then(|body| body.get("fulfil_unit")?.as_str()).unwrap_or("days");
        if unit == "working_days" {
            let calendar = crate::cases::deadlines(body, "");
            crate::cases::add_working_days(
                received_at,
                days,
                &calendar.holidays,
                calendar.utc_offset_minutes,
            )
        } else {
            if unit != "days" {
                // Đơn vị lạ: dùng ngày thường — hạn ngắn hơn, hướng an toàn cho chủ thể dữ liệu.
                tracing::warn!(
                    unit,
                    "privacy.subject_requests has an unknown fulfil_unit; using days"
                );
            }
            received_at + SignedDuration::from_hours(24 * days)
        }
    });
    (due_at, fulfil_due_at)
}

/// Yêu cầu quá hạn: chưa trả lời (`received`) mà qua hạn trả lời, hay chưa xong (`received`, `in_progress`) mà
/// qua hạn thực hiện. Đã nhận xử lý (`in_progress`) thì hạn trả lời đã đạt. Yêu cầu ghi trước khi có hạn thực hiện
/// (m0025, `fulfil_due_at` trống) hay loại quy tắc không đặt hạn: so với hạn trả lời như trước — không mất cảnh báo
/// (kiểm độc lập #620).
pub(crate) fn overdue_condition(at: Timestamp) -> sea_orm::Condition {
    use sea_orm::Condition;
    Condition::any()
        .add(
            Condition::all()
                .add(subject_request::Column::Status.eq("received"))
                .add(subject_request::Column::DueAt.lt(at)),
        )
        .add(
            Condition::all()
                .add(subject_request::Column::Status.is_in(["received", "in_progress"]))
                .add(
                    Condition::any().add(subject_request::Column::FulfilDueAt.lt(at)).add(
                        Condition::all()
                            .add(subject_request::Column::FulfilDueAt.is_null())
                            .add(subject_request::Column::DueAt.lt(at)),
                    ),
                ),
        )
}

/// Như [`overdue_condition`] cho một dòng đã đọc.
pub(crate) fn is_overdue(row: &subject_request::Model, at: Timestamp) -> bool {
    let open = matches!(row.status.as_str(), "received" | "in_progress");
    let fulfil_due = row.fulfil_due_at.unwrap_or(row.due_at);
    (row.status == "received" && row.due_at < at) || (open && fulfil_due < at)
}

impl ComplianceService {
    /// Hạn trả lời / xác nhận và hạn thực hiện của yêu cầu loại `kind` nhận lúc `received_at`, theo quy tắc
    /// `privacy.subject_requests` có hiệu lực lúc đó:
    /// - `respond_within_hours[kind]` (mặc định 72 giờ);
    /// - `fulfil_within_days[kind]` (LG-4b; không có thì không có hạn thực hiện) theo đơn vị `fulfil_unit`:
    ///   `days` (ngày thường — mặc định, chặt hơn) hay `working_days` (bỏ thứ Bảy, Chủ nhật, ngày trong
    ///   `holidays`, giờ địa phương `utc_offset_minutes`). Nguồn chưa thống nhất ngày làm việc hay ngày thường,
    ///   và hạn dài hơn khi có bên xử lý thứ ba (NĐ 356/2025 Điều 5 theo bản phân tích) — cần đối chiếu văn bản
    ///   gốc; Autoxide dùng hạn không kéo dài, hướng an toàn cho chủ thể dữ liệu.
    ///
    /// Thân quy tắc không phải JSON là lỗi (không đoán hạn).
    async fn request_deadlines(
        &self,
        tenant: &TenantContext,
        kind: &str,
        received_at: Timestamp,
    ) -> Result<(Timestamp, Option<Timestamp>), Error> {
        let rule = self.rule_for(tenant, "privacy.subject_requests", received_at).await?;
        let body = match rule {
            Some(row) => Some(serde_json::from_str::<Value>(&row.body).map_err(|error| {
                Error::internal(format!(
                    "the rule privacy.subject_requests version {} is not valid JSON: {error}",
                    row.version
                ))
            })?),
            None => None,
        };
        Ok(deadlines(body.as_ref(), kind, received_at))
    }

    pub(crate) async fn record_subject_request(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewRequest,
    ) -> Result<subject_request::Model, Error> {
        principal.require(&PRIVACY_MANAGE)?;
        let mut problems = Vec::new();
        if !KINDS.contains(&input.kind.as_str()) {
            problems.push(FieldError::new(
                "kind",
                "invalid",
                "must be `access`, `erasure`, `rectification`, `objection`, `restriction` or `withdrawal`",
            ));
        }
        if !CHANNELS.contains(&input.channel.as_str()) {
            problems.push(FieldError::new(
                "channel",
                "invalid",
                "must be `email`, `post`, `phone`, `authority` or `other`",
            ));
        }
        let subject = parse_subject(&input.subject);
        if subject.is_none() {
            problems.push(FieldError::new(
                "subject",
                "invalid",
                "must be a user id, an email address or a phone number",
            ));
        }
        let at = now();
        let received_at = input.received_at.unwrap_or(at);
        if received_at > at + SignedDuration::from_mins(5) {
            problems.push(FieldError::new("received_at", "range", "must not be in the future"));
        }
        if input.notes.as_deref().is_some_and(|notes| notes.len() > 5000) {
            problems.push(FieldError::new("notes", "length", "must be at most 5000 characters"));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let (subject_kind, subject) = subject.unwrap_or_default();
        // Hạn tính theo quy tắc hiệu lực lúc nhận yêu cầu.
        let (due_at, fulfil_due_at) =
            self.request_deadlines(tenant, &input.kind, received_at).await?;
        let row = subject_request::Model {
            id: Uuid::now_v7(),
            tenant_id: tenant.id().as_uuid(),
            kind: input.kind,
            channel: input.channel,
            subject_kind: subject_kind.to_owned(),
            subject,
            received_at,
            due_at,
            fulfil_due_at,
            status: "received".to_owned(),
            reason: None,
            notes: input.notes,
            handled_by: None,
            closed_at: None,
            created_by: Some(principal.user_id()),
            created_at: at,
            updated_at: at,
        };
        let transaction = tenant.database().begin_write().await?;
        let saved = subject_request::ActiveModel::from(row).insert(&transaction).await?;
        let event = events::SubjectRequestReceived(json!({
            "request_id": saved.id, "kind": saved.kind, "channel": saved.channel, "due_at": saved.due_at,
            "fulfil_due_at": saved.fulfil_due_at,
        }));
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                Some(format!("compliance.subject_request:{}", saved.id)),
                &event,
            )
            .await?;
        transaction.commit().await?;
        Ok(saved)
    }

    pub(crate) async fn subject_request(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<subject_request::Model, Error> {
        principal.require(&PRIVACY_MANAGE)?;
        subject_request::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(subject_request_not_found)
    }

    pub(crate) async fn list_subject_requests(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        status: Option<&str>,
        overdue: bool,
        page: &PageRequest,
    ) -> Result<Page<subject_request::Model>, Error> {
        principal.require(&PRIVACY_MANAGE)?;
        let mut select = subject_request::Entity::find().scoped(tenant);
        if let Some(status) = status {
            select = select.filter(subject_request::Column::Status.eq(status));
        }
        if overdue {
            select = select.filter(overdue_condition(Timestamp::now()));
        }
        fetch_page(
            select,
            subject_request::Column::Id,
            |row| row.id,
            page,
            tenant.database().read(),
        )
        .await
    }

    /// Ghi trạng thái mới (đúng phiên bản đã đọc), kèm event.
    async fn save_request(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        before: &subject_request::Model,
        row: subject_request::Model,
    ) -> Result<subject_request::Model, Error> {
        let transaction = tenant.database().begin_write().await?;
        let result = subject_request::Entity::update_many()
            .set(subject_request::ActiveModel::from(row.clone()).reset_all())
            .filter(subject_request::Column::TenantId.eq(tenant.id().as_uuid()))
            .filter(subject_request::Column::Id.eq(row.id))
            .filter(subject_request::Column::UpdatedAt.eq(before.updated_at))
            .exec(&transaction)
            .await?;
        if result.rows_affected == 0 {
            return Err(autoxide_kernel::version::changed());
        }
        let event = events::SubjectRequestUpdated(json!({
            "request_id": row.id, "kind": row.kind, "status": row.status, "previous_status": before.status,
        }));
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                Some(format!("compliance.subject_request:{}", row.id)),
                &event,
            )
            .await?;
        transaction.commit().await?;
        Ok(row)
    }

    /// Đổi trạng thái (`in_progress` / `rejected`, `reason` bắt buộc khi từ chối) hay ghi chú.
    pub(crate) async fn update_subject_request(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        status: Option<String>,
        reason: Option<String>,
        notes: Option<String>,
    ) -> Result<subject_request::Model, Error> {
        let before = self.subject_request(tenant, principal, id).await?;
        let mut problems = Vec::new();
        if status.as_deref().is_some_and(|status| !["in_progress", "rejected"].contains(&status)) {
            problems.push(FieldError::new(
                "status",
                "invalid",
                "must be `in_progress` or `rejected`; use `POST …/fulfil` to fulfil it",
            ));
        }
        if status.as_deref() == Some("rejected")
            && reason.as_deref().is_none_or(|text| text.trim().is_empty())
        {
            problems.push(FieldError::new("reason", "required", "is required to reject a request"));
        }
        if reason.as_deref().is_some_and(|text| text.len() > 5000)
            || notes.as_deref().is_some_and(|text| text.len() > 5000)
        {
            problems.push(FieldError::new("notes", "length", "must be at most 5000 characters"));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        if !matches!(before.status.as_str(), "received" | "in_progress") {
            return Err(subject_request_closed(&before.status));
        }
        let mut row = before.clone();
        if let Some(status) = status {
            if status == "rejected" {
                row.closed_at = Some(now());
            }
            row.status = status;
        }
        if reason.is_some() {
            row.reason = reason;
        }
        if notes.is_some() {
            row.notes = notes;
        }
        row.handled_by = Some(principal.user_id());
        row.updated_at = now();
        self.save_request(tenant, principal, &before, row).await
    }

    /// Thực hiện yêu cầu bằng cơ chế sẵn có rồi đánh dấu `fulfilled`.
    pub(crate) async fn fulfil_subject_request(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        notes: Option<String>,
    ) -> Result<Fulfilled, Error> {
        let before = self.subject_request(tenant, principal, id).await?;
        if notes.as_deref().is_some_and(|text| text.len() > 5000) {
            return Err(Error::validation(vec![FieldError::new(
                "notes",
                "length",
                "must be at most 5000 characters",
            )]));
        }
        if !matches!(before.status.as_str(), "received" | "in_progress") {
            return Err(subject_request_closed(&before.status));
        }
        let subject = consent_subject(&before).ok_or_else(subject_request_not_found)?;
        let (mut export, mut erased, mut next_steps) = (None, None, None);
        match (before.kind.as_str(), &subject) {
            ("access", ConsentSubject::User(user)) => {
                let data = self.personal_data.export(tenant, *user).await?;
                export = Some(serde_json::to_value(data).map_err(Error::internal)?);
            }
            ("access", contact) => {
                let (kind, key, _, _) = crate::consents::subject_columns(contact);
                let rows = consent::Entity::find()
                    .scoped(tenant)
                    .filter(consent::Column::SubjectKind.eq(kind))
                    .filter(consent::Column::SubjectKey.eq(key))
                    .all(tenant.database().read())
                    .await?;
                let consents: Vec<Value> = rows
                    .iter()
                    .map(|row| {
                        serde_json::to_value(crate::consents::describe_consent(row))
                            .unwrap_or_default()
                    })
                    .collect();
                export = Some(json!({"compliance": {"consents": consents}}));
                next_steps = Some(
                    "Other modules keep data by account; search the other modules (e.g. forms, \
                     newsletter) for this address too."
                        .to_owned(),
                );
            }
            ("erasure", ConsentSubject::User(user)) => {
                Self::erase_user(tenant, *user).await?;
                erased = Some(1);
                next_steps = Some(format!(
                    "Delete the account itself with DELETE /api/v1/auth/users/{user}: every module then \
                     erases its data (event `auth.user_deleted`)."
                ));
            }
            ("erasure" | "withdrawal", contact) => {
                erased = Some(self.erase_contact(tenant, contact).await?);
            }
            _ => {
                next_steps =
                    Some("Make the change by hand, then keep a note of what was done.".to_owned());
            }
        }
        let mut row = before.clone();
        row.status = "fulfilled".to_owned();
        row.closed_at = Some(now());
        row.handled_by = Some(principal.user_id());
        if notes.is_some() {
            row.notes = notes;
        }
        row.updated_at = now();
        let request = self.save_request(tenant, principal, &before, row).await?;
        Ok(Fulfilled { request, export, erased, next_steps })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doi_tuong_cua_yeu_cau_nhan_dang_dung() {
        let id = Uuid::now_v7();
        assert_eq!(parse_subject(&id.to_string()), Some(("user", id.to_string())));
        assert_eq!(
            parse_subject(" Ada@Example.com "),
            Some(("email", "ada@example.com".to_owned()))
        );
        assert_eq!(parse_subject("0912 345 678"), Some(("phone", "+84912345678".to_owned())));
        assert_eq!(parse_subject("hello"), None);
    }
}
