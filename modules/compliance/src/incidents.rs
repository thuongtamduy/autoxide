//! Sự cố dữ liệu cá nhân (Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15, NĐ 356/2025/NĐ-CP — báo trong 72 giờ
//! từ khi phát hiện; cần đối chiếu văn bản gốc / người có chuyên môn pháp lý; D-360):
//!
//! - Ghi sự cố → `report_due_at` = `detected_at` + giờ của quy tắc `privacy.incident` (dữ liệu có ngày
//!   hiệu lực, mặc định 72).
//! - Job `compliance.incident_deadlines` (15 phút) nhắc ở mỗi mốc `remind_at_hours` và ở mốc hạn: phát
//!   event `compliance.incident_deadline_near` và gửi mail tới chính sách `incident.alert_emails`. Mỗi
//!   mốc một lần: `UPDATE … WHERE reminded_hours = <cũ>` phải đổi đúng một dòng.
//! - Bản nháp thông báo gửi cơ quan và gửi người bị ảnh hưởng dựng từ mẫu cố định + dữ liệu sự cố — chỉ
//!   là bản nháp, con người duyệt và gửi.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::i18n::Language;
use autoxide_kernel::job::{JobContext, JobHandler};
use autoxide_kernel::mail::MailTemplate;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext, TenantId};
use chrono::{DateTime, Duration, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::entity::{incident, incident_entry};
use crate::events;
use crate::permissions::INCIDENTS_MANAGE;
use crate::service::{ComplianceService, now};

/// Job nhắc hạn báo cáo sự cố.
pub(crate) const INCIDENT_DEADLINES: &str = "compliance.incident_deadlines";

const SEVERITIES: [&str; 4] = ["low", "medium", "high", "critical"];
const STATUSES: [&str; 4] = ["open", "contained", "reported", "closed"];

pub(crate) const DEADLINE_MAIL: MailTemplate = MailTemplate::new(
    "compliance.mail.incident_deadline",
    "Personal data breach: {hours} hours since detection — report due {due}",
    "The personal data breach \"{title}\" was detected {hours} hours ago and is not reported yet.\n\n\
     Report due: {due} (UTC).\n\nRecord the report with PATCH /api/v1/compliance/incidents/{id} \
     (authority_notified_at, authority_reference); drafts: GET /api/v1/compliance/incidents/{id}/draft.\n",
);

pub(crate) const AUTHORITY_DRAFT: MailTemplate = MailTemplate::new(
    "compliance.mail.incident_draft_authority",
    "Notification of a personal data breach — {controller}",
    "DRAFT — have a legal professional review it before sending.\n\n\
     To: the authority in charge of personal data protection\n\
     From: {controller} (data controller)\nContact: {contact}\n\n\
     1. Description: {title}. {description}\n\
     2. Detected at: {detected} (UTC)\n\
     3. Kinds of personal data concerned: {categories}\n\
     4. Approximate number of people affected: {affected}\n\
     5. Status and measures taken: {status}\n\
     6. Likely consequences and planned measures: (to complete)\n",
);

pub(crate) const SUBJECTS_DRAFT: MailTemplate = MailTemplate::new(
    "compliance.mail.incident_draft_subjects",
    "Information about an incident affecting your personal data",
    "DRAFT — have a legal professional review it before sending.\n\n\
     Hello,\n\n{controller} informs you of an incident affecting personal data: {title}.\n\
     Detected on {detected} (UTC). Data concerned: {categories}.\n\n\
     What we did: (to complete)\nWhat you can do: (to complete)\n\n\
     Questions: {contact}\n",
);

pub(crate) fn incident_not_found() -> Error {
    Error::not_found("compliance.incident_not_found", "There is no such personal data breach.")
}

/// Sự cố mới.
#[derive(Debug, Clone)]
pub(crate) struct NewIncident {
    pub title: String,
    pub description: String,
    pub severity: String,
    pub detected_at: Option<DateTime<Utc>>,
    pub data_categories: Vec<String>,
    pub affected_count: Option<i64>,
}

/// Thay đổi một sự cố (trường `None` giữ nguyên).
#[derive(Debug, Clone, Default)]
pub(crate) struct IncidentChange {
    pub title: Option<String>,
    pub description: Option<String>,
    pub severity: Option<String>,
    pub status: Option<String>,
    pub data_categories: Option<Vec<String>>,
    pub affected_count: Option<i64>,
    pub authority_notified_at: Option<DateTime<Utc>>,
    pub authority_reference: Option<String>,
    pub subjects_notified_at: Option<DateTime<Utc>>,
}

fn check_text(
    title: Option<&str>,
    description: Option<&str>,
    severity: Option<&str>,
    categories: Option<&[String]>,
    affected: Option<i64>,
    problems: &mut Vec<FieldError>,
) {
    if title.is_some_and(|title| title.trim().is_empty() || title.chars().count() > 200) {
        problems.push(FieldError::new("title", "length", "must be 1 to 200 characters"));
    }
    if description.is_some_and(|text| text.len() > 20_000) {
        problems.push(FieldError::new("description", "length", "must be at most 20000 characters"));
    }
    if severity.is_some_and(|severity| !SEVERITIES.contains(&severity)) {
        problems.push(FieldError::new(
            "severity",
            "invalid",
            "must be `low`, `medium`, `high` or `critical`",
        ));
    }
    if categories.is_some_and(|items| {
        items.len() > 50 || items.iter().any(|item| item.trim().is_empty() || item.len() > 64)
    }) {
        problems.push(FieldError::new(
            "data_categories",
            "invalid",
            "must be at most 50 names of 1 to 64 characters",
        ));
    }
    if affected.is_some_and(|count| count < 0) {
        problems.push(FieldError::new("affected_count", "range", "must be 0 or more"));
    }
}

/// Giờ phải báo và các mốc nhắc của quy tắc.
pub(crate) fn deadline_rule(body: Option<&Value>) -> (i64, Vec<i64>) {
    let hours = body
        .and_then(|body| body.get("report_within_hours"))
        .and_then(Value::as_i64)
        .filter(|hours| *hours > 0)
        .unwrap_or(72);
    let mut marks: Vec<i64> = body
        .and_then(|body| body.get("remind_at_hours"))
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_i64).filter(|mark| *mark > 0).collect())
        .unwrap_or_else(|| vec![24, 48, 60, 68]);
    // Mốc hạn cũng là một lần nhắc (quá hạn chưa báo).
    marks.push(hours);
    marks.sort_unstable();
    marks.dedup();
    (hours, marks)
}

/// Mốc nhắc lớn nhất đã qua mà chưa nhắc; `None` khi không có.
pub(crate) fn due_mark(marks: &[i64], elapsed_hours: i64, reminded: i64) -> Option<i64> {
    marks.iter().copied().filter(|mark| *mark <= elapsed_hours && *mark > reminded).max()
}

pub(crate) fn incident_payload(row: &incident::Model) -> Value {
    json!({
        "incident_id": row.id, "severity": row.severity, "status": row.status,
        "report_due_at": row.report_due_at,
    })
}

impl ComplianceService {
    async fn incident_rule(&self, tenant: &TenantContext) -> Result<(i64, Vec<i64>), Error> {
        let rule = self.rule_for(tenant, "privacy.incident", Utc::now()).await?;
        let body = rule.and_then(|row| serde_json::from_str::<Value>(&row.body).ok());
        Ok(deadline_rule(body.as_ref()))
    }

    pub(crate) async fn open_incident(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        input: NewIncident,
    ) -> Result<incident::Model, Error> {
        principal.require(&INCIDENTS_MANAGE)?;
        let mut problems = Vec::new();
        check_text(
            Some(&input.title),
            Some(&input.description),
            Some(&input.severity),
            Some(&input.data_categories),
            input.affected_count,
            &mut problems,
        );
        let at = now();
        let detected_at = input.detected_at.unwrap_or(at);
        if detected_at > at + Duration::minutes(5) {
            problems.push(FieldError::new("detected_at", "range", "must not be in the future"));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let (hours, _) = self.incident_rule(tenant).await?;
        let row = incident::Model {
            id: Uuid::now_v7(),
            tenant_id: tenant.id().as_uuid(),
            title: input.title.trim().to_owned(),
            description: input.description,
            severity: input.severity,
            status: "open".to_owned(),
            detected_at,
            report_due_at: detected_at + Duration::hours(hours),
            reminded_hours: 0,
            authority_notified_at: None,
            authority_reference: None,
            subjects_notified_at: None,
            data_categories: json!(input.data_categories).to_string(),
            affected_count: input.affected_count,
            created_by: Some(principal.user_id()),
            created_at: at,
            updated_at: at,
        };
        let transaction = tenant.database().begin_write().await?;
        let saved = incident::ActiveModel::from(row).insert(&transaction).await?;
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                Some(format!("compliance.incident:{}", saved.id)),
                &events::IncidentOpened(incident_payload(&saved)),
            )
            .await?;
        transaction.commit().await?;
        Ok(saved)
    }

    pub(crate) async fn incident(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(incident::Model, Vec<incident_entry::Model>), Error> {
        principal.require(&INCIDENTS_MANAGE)?;
        let read = tenant.database().read();
        let row = incident::Entity::find_by_id(id)
            .scoped(tenant)
            .one(read)
            .await?
            .ok_or_else(incident_not_found)?;
        let entries = incident_entry::Entity::find()
            .scoped(tenant)
            .filter(incident_entry::Column::IncidentId.eq(id))
            .order_by_asc(incident_entry::Column::Id)
            .all(read)
            .await?;
        Ok((row, entries))
    }

    pub(crate) async fn list_incidents(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        status: Option<&str>,
        overdue: bool,
        page: &PageRequest,
    ) -> Result<Page<incident::Model>, Error> {
        principal.require(&INCIDENTS_MANAGE)?;
        let mut select = incident::Entity::find().scoped(tenant);
        if let Some(status) = status {
            select = select.filter(incident::Column::Status.eq(status));
        }
        if overdue {
            select = select
                .filter(incident::Column::Status.is_in(["open", "contained"]))
                .filter(incident::Column::AuthorityNotifiedAt.is_null())
                .filter(incident::Column::ReportDueAt.lt(Utc::now()));
        }
        fetch_page(select, incident::Column::Id, |row| row.id, page, tenant.database().read()).await
    }

    pub(crate) async fn update_incident(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        expected: Option<DateTime<Utc>>,
        change: IncidentChange,
    ) -> Result<incident::Model, Error> {
        principal.require(&INCIDENTS_MANAGE)?;
        let before = incident::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(incident_not_found)?;
        let mut problems = Vec::new();
        check_text(
            change.title.as_deref(),
            change.description.as_deref(),
            change.severity.as_deref(),
            change.data_categories.as_deref(),
            change.affected_count,
            &mut problems,
        );
        if change.status.as_deref().is_some_and(|status| !STATUSES.contains(&status)) {
            problems.push(FieldError::new(
                "status",
                "invalid",
                "must be `open`, `contained`, `reported` or `closed`",
            ));
        }
        if change.authority_reference.as_deref().is_some_and(|text| text.len() > 100) {
            problems.push(FieldError::new(
                "authority_reference",
                "length",
                "must be at most 100 characters",
            ));
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let mut row = before.clone();
        if let Some(title) = change.title {
            row.title = title.trim().to_owned();
        }
        if let Some(description) = change.description {
            row.description = description;
        }
        if let Some(severity) = change.severity {
            row.severity = severity;
        }
        if let Some(items) = change.data_categories {
            row.data_categories = json!(items).to_string();
        }
        if let Some(count) = change.affected_count {
            row.affected_count = Some(count);
        }
        if let Some(reference) = change.authority_reference {
            row.authority_reference = Some(reference);
        }
        if let Some(at) = change.subjects_notified_at {
            row.subjects_notified_at = Some(at);
        }
        if let Some(at) = change.authority_notified_at {
            row.authority_notified_at = Some(at);
            if change.status.is_none() && matches!(row.status.as_str(), "open" | "contained") {
                row.status = "reported".to_owned();
            }
        }
        if let Some(status) = change.status {
            if status == "reported" && row.authority_notified_at.is_none() {
                row.authority_notified_at = Some(now());
            }
            row.status = status;
        }
        row.updated_at = now();
        let transaction = tenant.database().begin_write().await?;
        let result = incident::Entity::update_many()
            .set(incident::ActiveModel::from(row.clone()).reset_all())
            .filter(incident::Column::TenantId.eq(tenant.id().as_uuid()))
            .filter(incident::Column::Id.eq(id))
            .filter(incident::Column::UpdatedAt.eq(expected.unwrap_or(before.updated_at)))
            .exec(&transaction)
            .await?;
        if result.rows_affected == 0 {
            return Err(autoxide_kernel::version::changed());
        }
        let mut payload = incident_payload(&row);
        payload["previous_status"] = json!(before.status);
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                Some(format!("compliance.incident:{id}")),
                &events::IncidentUpdated(payload),
            )
            .await?;
        transaction.commit().await?;
        Ok(row)
    }

    pub(crate) async fn add_incident_entry(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        body: &str,
    ) -> Result<incident_entry::Model, Error> {
        principal.require(&INCIDENTS_MANAGE)?;
        let exists = incident::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(incident_not_found)?;
        if body.trim().is_empty() || body.len() > 20_000 {
            return Err(Error::validation(vec![FieldError::new(
                "body",
                "length",
                "must be 1 to 20000 characters",
            )]));
        }
        let row = incident_entry::Model {
            id: Uuid::now_v7(),
            tenant_id: tenant.id().as_uuid(),
            incident_id: exists.id,
            body: body.to_owned(),
            author_user_id: Some(principal.user_id()),
            created_at: now(),
        };
        let transaction = tenant.database().begin_write().await?;
        let saved = incident_entry::ActiveModel::from(row).insert(&transaction).await?;
        incident::Entity::update_many()
            .col_expr(incident::Column::UpdatedAt, Expr::value(now()))
            .filter(incident::Column::Id.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        let mut payload = incident_payload(&exists);
        payload["previous_status"] = json!(exists.status);
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                Some(format!("compliance.incident:{id}")),
                &events::IncidentUpdated(payload),
            )
            .await?;
        transaction.commit().await?;
        Ok(saved)
    }

    /// Bản nháp thông báo gửi `audience` (`authority` hay `subjects`) bằng ngôn ngữ `language`.
    pub(crate) async fn incident_draft(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        audience: &str,
        language: Language,
    ) -> Result<(String, String), Error> {
        principal.require(&INCIDENTS_MANAGE)?;
        let template = match audience {
            "authority" => AUTHORITY_DRAFT,
            "subjects" => SUBJECTS_DRAFT,
            _ => {
                return Err(Error::validation(vec![FieldError::new(
                    "audience",
                    "invalid",
                    "must be `authority` or `subjects`",
                )]));
            }
        };
        let (row, _) = self.incident(tenant, principal, id).await?;
        let contact = self
            .policy(tenant, "privacy.contact")
            .await?
            .map_or_else(|| "(to complete)".to_owned(), |value| contact_text(&value));
        let categories: Vec<String> =
            serde_json::from_str(&row.data_categories).unwrap_or_default();
        let categories =
            if categories.is_empty() { "(to complete)".to_owned() } else { categories.join(", ") };
        let affected = row
            .affected_count
            .map_or_else(|| "(to complete)".to_owned(), |count| count.to_string());
        let detected = row.detected_at.format("%Y-%m-%d %H:%M").to_string();
        let rendered = template.render(
            &self.translations,
            language,
            &[
                ("controller", tenant.name()),
                ("contact", &contact),
                ("title", &row.title),
                ("description", &row.description),
                ("detected", &detected),
                ("categories", &categories),
                ("affected", &affected),
                ("status", &row.status),
            ],
        );
        Ok((rendered.subject, rendered.text))
    }

    /// Nhắc hạn báo cáo mọi sự cố chưa báo trong `database` (job 15 phút). Trả số lần nhắc.
    pub(crate) async fn remind_incidents(
        &self,
        database: &autoxide_kernel::database::Database,
    ) -> Result<u64, Error> {
        let at = Utc::now();
        let due: Vec<(Uuid, Uuid)> = incident::Entity::find()
            .select_only()
            .column(incident::Column::TenantId)
            .column(incident::Column::Id)
            .filter(incident::Column::Status.is_in(["open", "contained"]))
            .filter(incident::Column::AuthorityNotifiedAt.is_null())
            // Mốc nhắc sớm nhất: không trước 1 giờ sau phát hiện; quá hạn 7 ngày thì thôi nhắc
            // (doctor vẫn báo).
            .filter(incident::Column::DetectedAt.lt(at - Duration::hours(1)))
            .filter(incident::Column::ReportDueAt.gt(at - Duration::days(7)))
            .order_by_asc(incident::Column::Id)
            .limit(500)
            .into_tuple()
            .all(database.read())
            .await?;
        let mut sent = 0;
        for (tenant_id, id) in due {
            let Ok(tenant) = self.tenant(TenantId::from_uuid(tenant_id)).await else {
                continue;
            };
            let Some(row) = incident::Entity::find_by_id(id)
                .scoped(&tenant)
                .one(tenant.database().read())
                .await?
            else {
                continue;
            };
            let (_, marks) = self.incident_rule(&tenant).await?;
            let elapsed = (at - row.detected_at).num_hours();
            let Some(mark) = due_mark(&marks, elapsed, i64::from(row.reminded_hours)) else {
                continue;
            };
            let mark_value = i32::try_from(mark).unwrap_or(i32::MAX);
            let transaction = tenant.database().begin_write().await?;
            // Một mốc một lần, kể cả hai bộ chạy job cùng lúc.
            let claimed = incident::Entity::update_many()
                .col_expr(incident::Column::RemindedHours, Expr::value(mark_value))
                .filter(incident::Column::Id.eq(id))
                .filter(incident::Column::RemindedHours.eq(row.reminded_hours))
                .scoped(&tenant)
                .exec(&transaction)
                .await?
                .rows_affected;
            if claimed != 1 {
                continue;
            }
            let event = events::IncidentDeadlineNear(json!({
                "incident_id": row.id, "hours_since_detection": mark, "report_due_at": row.report_due_at,
            }));
            self.outbox
                .record(
                    &transaction,
                    &tenant,
                    Actor::system(),
                    Some(format!("compliance.incident:{id}")),
                    &event,
                )
                .await?;
            transaction.commit().await?;
            sent += 1;
            // Mail gửi sau khi đã ghi mốc (ngoài transaction): lỗi gửi không làm nhắc lại mốc này.
            self.mail_deadline(&tenant, &row, mark).await;
        }
        Ok(sent)
    }

    async fn mail_deadline(&self, tenant: &TenantContext, row: &incident::Model, mark: i64) {
        let Some(mailer) = self.mailer.clone() else {
            return;
        };
        let emails: Vec<String> = match self.policy(tenant, "incident.alert_emails").await {
            Ok(Some(Value::Array(items))) => {
                items.iter().filter_map(|item| item.as_str().map(str::to_owned)).collect()
            }
            _ => Vec::new(),
        };
        let language = match self.jurisdiction(tenant).await.as_deref() {
            Ok("vn") => Language::Vi,
            _ => Language::En,
        };
        let (hours, due, id) = (
            mark.to_string(),
            row.report_due_at.format("%Y-%m-%d %H:%M").to_string(),
            row.id.to_string(),
        );
        for email in emails {
            let mut mail = DEADLINE_MAIL.mail(
                &email,
                &self.translations,
                language,
                &[("hours", &hours), ("due", &due), ("title", &row.title), ("id", &id)],
            );
            mail.dedupe_key = Some(format!("compliance.incident:{}:{mark}:{email}", row.id));
            if let Err(error) = mailer.send(tenant, mail).await {
                tracing::warn!(incident_id = %row.id, error = %error, "cannot queue the incident reminder mail");
            }
        }
    }
}

fn contact_text(value: &Value) -> String {
    match value {
        Value::Object(fields) => fields
            .iter()
            .filter_map(|(key, value)| value.as_str().map(|text| format!("{key}: {text}")))
            .collect::<Vec<_>>()
            .join(", "),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// Job `compliance.incident_deadlines`.
pub(crate) struct DeadlinesJob(pub Arc<ComplianceService>);

#[async_trait::async_trait]
impl JobHandler for DeadlinesJob {
    async fn run(&self, job: &JobContext) -> Result<(), Error> {
        self.0.remind_incidents(&job.database).await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn han_72_gio_va_moc_nhac_tu_quy_tac() {
        let body = json!({"report_within_hours": 72, "remind_at_hours": [24, 48, 60, 68]});
        let (hours, marks) = deadline_rule(Some(&body));
        assert_eq!(hours, 72);
        assert_eq!(marks, vec![24, 48, 60, 68, 72]);
        assert_eq!(deadline_rule(None).0, 72);
    }

    #[test]
    fn moc_nhac_lon_nhat_chua_nhac() {
        let marks = [24, 48, 60, 68, 72];
        assert_eq!(due_mark(&marks, 10, 0), None);
        assert_eq!(due_mark(&marks, 50, 0), Some(48));
        assert_eq!(due_mark(&marks, 50, 48), None);
        assert_eq!(due_mark(&marks, 61, 48), Some(60));
        assert_eq!(due_mark(&marks, 100, 60), Some(72));
    }
}
