//! Định nghĩa form: tạo, đọc, liệt kê, sửa, xoá. Đổi danh sách trường thì tăng `version` và
//! chụp lại trường vào `forms_form_versions`: bài gửi cũ luôn đọc được với nhãn lúc gửi.

pub(crate) mod fields;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::TenantContext;
use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel, PaginatorTrait,
    QueryFilter, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::{form, form_version, submission};
use crate::events::{self, Change};
use crate::forms::fields::{Field, FieldType, Localized};
use crate::permissions::{FORMS_MANAGE, FORMS_READ};
use crate::service::{
    FormsService, form_not_found, from_json, is_unique_violation, now, to_i32, to_json,
};

const KEY_MAX: usize = 50;
const NAME_MAX: usize = 200;
const MESSAGE_MAX: usize = 1000;
const ORIGINS_MAX: usize = 20;
const NOTIFY_MAX: usize = 20;
const LIMIT_MAX: u32 = 100_000;
const RETENTION_MAX: u32 = 36_500;

/// Whether a form takes submissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum FormStatus {
    /// Takes submissions.
    Open,
    /// Shown as closed; submissions are refused with `forms.closed`.
    Closed,
}

impl FormStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            FormStatus::Open => "open",
            FormStatus::Closed => "closed",
        }
    }
}

/// Một form, cột JSON đã đọc ra.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Form {
    pub row: form::Model,
    pub fields: Vec<Field>,
    pub allowed_origins: Vec<String>,
    pub success_message: Localized,
    pub notify_emails: Vec<String>,
}

impl Form {
    pub(crate) fn parse(row: form::Model) -> Result<Form, Error> {
        Ok(Form {
            fields: from_json(&row.fields)?,
            allowed_origins: from_json(&row.allowed_origins)?,
            success_message: from_json(&row.success_message)?,
            notify_emails: from_json(&row.notify_emails)?,
            row,
        })
    }

    pub(crate) fn status(&self) -> FormStatus {
        if self.row.status == "open" { FormStatus::Open } else { FormStatus::Closed }
    }

    /// Phạm vi của token form (`GET /api/v1/antispam/token?scope=…`).
    pub(crate) fn scope(&self) -> String {
        scope(&self.row.key)
    }

    /// Tóm tắt cho event: không có email nhận thông báo hay trường.
    fn summary(&self) -> Value {
        json!({
            "id": self.row.id,
            "key": self.row.key,
            "name": self.row.name,
            "status": self.row.status,
            "version": self.row.version,
        })
    }
}

pub(crate) fn scope(key: &str) -> String {
    format!("forms:{key}")
}

/// Các trường sửa được; `None` giữ nguyên (khi tạo: giá trị mặc định).
#[derive(Debug, Clone, Default)]
pub(crate) struct FormInput {
    pub key: Option<String>,
    pub name: Option<String>,
    pub status: Option<FormStatus>,
    pub fields: Option<Vec<Field>>,
    pub allowed_origins: Option<Vec<String>>,
    pub success_message: Option<Localized>,
    pub notify_emails: Option<Vec<String>>,
    /// `Some(None)`: bỏ.
    pub reply_to_field: Option<Option<String>>,
    pub ip_limit_per_hour: Option<Option<u32>>,
    pub email_limit_per_hour: Option<Option<u32>>,
    pub retention_days: Option<Option<u32>>,
}

impl FormsService {
    /// Tạo form (mở nhận bài ngay, trừ khi `status` là `closed`). Cần `forms.manage`; đặt
    /// `notify_emails` cần thêm `forms.read`.
    pub(crate) async fn create_form(
        &self,
        principal: &Principal,
        input: FormInput,
    ) -> Result<Form, Error> {
        principal.require(&FORMS_MANAGE)?;
        let mut problems = Vec::new();
        for (name, missing) in [
            ("key", input.key.is_none()),
            ("name", input.name.is_none()),
            ("fields", input.fields.is_none()),
        ] {
            if missing {
                problems.push(FieldError::new(name, "required", "is required"));
            }
        }
        let now = now();
        let blank = form::Model {
            id: Uuid::now_v7(),
            key: String::new(),
            name: String::new(),
            status: FormStatus::Open.as_str().to_owned(),
            fields: "[]".to_owned(),
            version: 1,
            allowed_origins: "[]".to_owned(),
            success_message: "{}".to_owned(),
            notify_emails: "[]".to_owned(),
            reply_to_field: None,
            ip_limit_per_hour: None,
            email_limit_per_hour: None,
            retention_days: None,
            created_by: Some(principal.user_id()),
            created_at: now,
            updated_at: now,
        };
        let mut candidate = Form::parse(blank)?;
        let sets_notify = input.notify_emails.as_ref().is_some_and(|emails| !emails.is_empty());
        if sets_notify {
            principal.require(&FORMS_READ)?;
        }
        self.apply(&mut candidate, input, &mut problems)?;
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let transaction = self.database.begin_write().await?;
        if key_taken(&transaction, &candidate.row.key, None).await? {
            return Err(key_conflict(&candidate.row.key));
        }
        let inserted = candidate.row.clone().into_active_model().insert(&transaction).await;
        let row = match inserted {
            Ok(row) => row,
            Err(error) if is_unique_violation(&error) => {
                return Err(key_conflict(&candidate.row.key));
            }
            Err(error) => return Err(error.into()),
        };
        snapshot(&transaction, &row, now).await?;
        let created = Form::parse(row)?;
        let event = events::FormCreated(Change { before: None, after: Some(created.summary()) });
        self.outbox
            .record(
                &transaction,
                &TenantContext::legacy_default(&self.database),
                Actor::from(principal),
                events::form_subject(created.row.id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        tracing::info!(form_id = %created.row.id, "form created");
        Ok(created)
    }

    /// Form `id`. Cần `forms.read`.
    pub(crate) async fn get_form(&self, principal: &Principal, id: Uuid) -> Result<Form, Error> {
        principal.require(&FORMS_READ)?;
        self.find_form(id).await
    }

    pub(crate) async fn find_form(&self, id: Uuid) -> Result<Form, Error> {
        let row = form::Entity::find_by_id(id)
            .one(self.database.read())
            .await?
            .ok_or_else(form_not_found)?;
        Form::parse(row)
    }

    /// Form có `key` (cho endpoint công khai).
    pub(crate) async fn find_by_key(&self, key: &str) -> Result<Form, Error> {
        let row = form::Entity::find()
            .filter(form::Column::Key.eq(key.trim()))
            .one(self.database.read())
            .await?
            .ok_or_else(form_not_found)?;
        Form::parse(row)
    }

    /// Một trang form, mới trước (mặc định). Cần `forms.read`.
    pub(crate) async fn list_forms(
        &self,
        principal: &Principal,
        status: Option<FormStatus>,
        page: &PageRequest,
    ) -> Result<Page<Form>, Error> {
        principal.require(&FORMS_READ)?;
        let mut select = form::Entity::find();
        if let Some(status) = status {
            select = select.filter(form::Column::Status.eq(status.as_str()));
        }
        let found =
            fetch_page(select, form::Column::Id, |row| row.id, page, self.database.read()).await?;
        let Page { data, pagination } = found;
        let data = data.into_iter().map(Form::parse).collect::<Result<Vec<_>, _>>()?;
        Ok(Page { data, pagination })
    }

    /// Sửa form `id`. Cần `forms.manage`; đổi `notify_emails` cần thêm `forms.read` (mail báo
    /// chứa câu trả lời: không có quyền đọc bài thì không được tự chuyển bài về hộp thư mình).
    /// Đổi trường thì tăng `version`.
    pub(crate) async fn update_form(
        &self,
        principal: &Principal,
        id: Uuid,
        input: FormInput,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Form, Error> {
        principal.require(&FORMS_MANAGE)?;
        let before = self.find_form(id).await?;
        if expected.is_some_and(|expected| expected != before.row.updated_at) {
            return Err(autoxide_kernel::version::changed());
        }
        let mut problems = Vec::new();
        let mut candidate = before.clone();
        let notify_given = input.notify_emails.is_some();
        self.apply(&mut candidate, input, &mut problems)?;
        // So danh sách đã chuẩn hoá; gửi lại đúng danh sách cũ (vd client gửi cả form) thì không
        // cần thêm quyền.
        let notify_problem =
            problems.iter().any(|problem| problem.field.starts_with("notify_emails"));
        if notify_given && (candidate.notify_emails != before.notify_emails || notify_problem) {
            principal.require(&FORMS_READ)?;
        }
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let now = now();
        let fields_changed = candidate.fields != before.fields;
        if fields_changed {
            candidate.row.version = before.row.version + 1;
        }
        candidate.row.updated_at = now;
        let transaction = self.database.begin_write().await?;
        if candidate.row.key != before.row.key
            && key_taken(&transaction, &candidate.row.key, Some(id)).await?
        {
            return Err(key_conflict(&candidate.row.key));
        }
        let mut row = before.row.clone().into_active_model();
        let next = candidate.row.clone();
        row.key = Set(next.key);
        row.name = Set(next.name);
        row.status = Set(next.status);
        row.fields = Set(next.fields);
        row.version = Set(next.version);
        row.allowed_origins = Set(next.allowed_origins);
        row.success_message = Set(next.success_message);
        row.notify_emails = Set(next.notify_emails);
        row.reply_to_field = Set(next.reply_to_field);
        row.ip_limit_per_hour = Set(next.ip_limit_per_hour);
        row.email_limit_per_hour = Set(next.email_limit_per_hour);
        row.retention_days = Set(next.retention_days);
        row.updated_at = Set(now);
        // Phiên bản đã đọc (`If-Match`), hay ít nhất "chưa ai sửa từ lúc đọc ở trên".
        let update = form::Entity::update(row)
            .validate()?
            .filter(form::Column::UpdatedAt.eq(expected.unwrap_or(before.row.updated_at)));
        let after = match update.exec(&transaction).await {
            Ok(after) => after,
            Err(sea_orm::DbErr::RecordNotUpdated) => {
                return Err(autoxide_kernel::version::changed());
            }
            Err(error) if is_unique_violation(&error) => {
                return Err(key_conflict(&candidate.row.key));
            }
            Err(error) => return Err(error.into()),
        };
        if fields_changed {
            snapshot(&transaction, &after, now).await?;
        }
        let after = Form::parse(after)?;
        let mut summary = after.summary();
        if let Some(object) = summary.as_object_mut() {
            object.insert("fields_changed".to_owned(), json!(fields_changed));
        }
        let event =
            events::FormUpdated(Change { before: Some(before.summary()), after: Some(summary) });
        self.outbox
            .record(
                &transaction,
                &TenantContext::legacy_default(&self.database),
                Actor::from(principal),
                events::form_subject(id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        Ok(after)
    }

    /// Xoá form `id` cùng mọi phiên bản và mọi bài gửi của nó. Cần `forms.manage`.
    pub(crate) async fn delete_form(
        &self,
        principal: &Principal,
        id: Uuid,
        expected: Option<DateTime<Utc>>,
    ) -> Result<(), Error> {
        principal.require(&FORMS_MANAGE)?;
        let before = self.find_form(id).await?;
        if expected.is_some_and(|expected| expected != before.row.updated_at) {
            return Err(autoxide_kernel::version::changed());
        }
        let transaction = self.database.begin_write().await?;
        let deleted = form::Entity::delete_many()
            .filter(form::Column::Id.eq(id))
            .filter(form::Column::UpdatedAt.eq(expected.unwrap_or(before.row.updated_at)))
            .exec(&transaction)
            .await?;
        if deleted.rows_affected == 0 {
            return Err(autoxide_kernel::version::changed());
        }
        form_version::Entity::delete_many()
            .filter(form_version::Column::FormId.eq(id))
            .exec(&transaction)
            .await?;
        let submissions = submission::Entity::delete_many()
            .filter(submission::Column::FormId.eq(id))
            .exec(&transaction)
            .await?;
        let mut summary = before.summary();
        if let Some(object) = summary.as_object_mut() {
            object.insert("submissions_deleted".to_owned(), json!(submissions.rows_affected));
        }
        let event = events::FormDeleted(Change { before: Some(summary), after: None });
        self.outbox
            .record(
                &transaction,
                &TenantContext::legacy_default(&self.database),
                Actor::from(principal),
                events::form_subject(id),
                &event,
            )
            .await?;
        transaction.commit().await?;
        tracing::info!(form_id = %id, submissions = submissions.rows_affected, "form deleted");
        // File xuất của form chứa bài vừa xoá (sau transaction: nơi lưu có thể ở xa).
        let exports = self.exports_of(vec![id]).await?;
        self.purge_exports(exports).await?;
        Ok(())
    }

    /// Áp `input` lên `candidate` rồi kiểm toàn bộ (mọi lỗi vào `problems`).
    fn apply(
        &self,
        candidate: &mut Form,
        input: FormInput,
        problems: &mut Vec<FieldError>,
    ) -> Result<(), Error> {
        if let Some(key) = input.key {
            let key = key.trim().to_owned();
            if !is_valid_form_key(&key) {
                problems.push(FieldError::new(
                    "key",
                    "invalid",
                    "must be lower-case letters and digits in groups joined by `-`, at most {key_max} characters",
                ).with_param("key_max", KEY_MAX));
            }
            candidate.row.key = key;
        }
        if let Some(name) = input.name {
            let name = name.trim().to_owned();
            if name.is_empty()
                || name.chars().count() > NAME_MAX
                || name.chars().any(char::is_control)
            {
                problems.push(
                    FieldError::new(
                        "name",
                        "length",
                        "must be 1 to {name_max} characters, on one line",
                    )
                    .with_param("name_max", NAME_MAX),
                );
            }
            candidate.row.name = name;
        }
        if let Some(status) = input.status {
            candidate.row.status = status.as_str().to_owned();
        }
        if let Some(fields) = input.fields {
            candidate.fields = fields::check_definition(&fields, self.config.max_fields, problems);
        }
        if let Some(origins) = input.allowed_origins {
            if origins.len() > ORIGINS_MAX {
                problems.push(
                    FieldError::new(
                        "allowed_origins",
                        "length",
                        "must have at most {origins_max} origins",
                    )
                    .with_param("origins_max", ORIGINS_MAX),
                );
            }
            let mut clean = Vec::new();
            for (index, origin) in origins.iter().enumerate() {
                match normalize_origin(origin) {
                    Some(origin) if !clean.contains(&origin) => clean.push(origin),
                    Some(_) => {}
                    None => problems.push(FieldError::new(
                        format!("allowed_origins[{index}]"),
                        "invalid",
                        "must be `https://host` or `http://host:port`, without a path",
                    )),
                }
            }
            candidate.allowed_origins = clean;
        }
        if let Some(message) = input.success_message {
            candidate.success_message =
                message.normalized("success_message", MESSAGE_MAX, problems);
        }
        if let Some(emails) = input.notify_emails {
            if emails.len() > NOTIFY_MAX {
                problems.push(
                    FieldError::new(
                        "notify_emails",
                        "length",
                        "must have at most {notify_max} addresses",
                    )
                    .with_param("notify_max", NOTIFY_MAX),
                );
            }
            let mut clean = Vec::new();
            for (index, email) in emails.iter().enumerate() {
                let email = autoxide_kernel::email::normalize(email);
                if !autoxide_kernel::email::is_valid(&email) {
                    problems.push(FieldError::new(
                        format!("notify_emails[{index}]"),
                        "email",
                        "must be an email address",
                    ));
                } else if !clean.contains(&email) {
                    clean.push(email);
                }
            }
            candidate.notify_emails = clean;
        }
        if let Some(reply_to) = input.reply_to_field {
            candidate.row.reply_to_field =
                reply_to.map(|key| key.trim().to_owned()).filter(|key| !key.is_empty());
        }
        // Kiểm cả khi chỉ đổi trường: trường email được chọn có thể vừa bị bỏ.
        if let Some(key) = &candidate.row.reply_to_field
            && !candidate
                .fields
                .iter()
                .any(|field| &field.key == key && field.kind == FieldType::Email)
        {
            problems.push(FieldError::new(
                "reply_to_field",
                "invalid",
                "must be the key of an `email` field of the form",
            ));
        }
        for (name, value, target) in [
            ("ip_limit_per_hour", input.ip_limit_per_hour, &mut candidate.row.ip_limit_per_hour),
            (
                "email_limit_per_hour",
                input.email_limit_per_hour,
                &mut candidate.row.email_limit_per_hour,
            ),
        ] {
            if let Some(value) = value {
                if value.is_some_and(|value| value == 0 || value > LIMIT_MAX) {
                    problems.push(
                        FieldError::new(name, "range", "must be 1 to {limit_max}")
                            .with_param("limit_max", LIMIT_MAX),
                    );
                }
                *target = value.map(|value| to_i32(value.min(LIMIT_MAX))).transpose()?;
            }
        }
        if let Some(days) = input.retention_days {
            if days.is_some_and(|days| days > RETENTION_MAX) {
                problems.push(
                    FieldError::new(
                        "retention_days",
                        "range",
                        "must be 0 (keep) to {retention_max}",
                    )
                    .with_param("retention_max", RETENTION_MAX),
                );
            }
            candidate.row.retention_days =
                days.map(|days| to_i32(days.min(RETENTION_MAX))).transpose()?;
        }
        candidate.row.fields = to_json(&candidate.fields)?;
        candidate.row.allowed_origins = to_json(&candidate.allowed_origins)?;
        candidate.row.success_message = to_json(&candidate.success_message)?;
        candidate.row.notify_emails = to_json(&candidate.notify_emails)?;
        Ok(())
    }
}

/// Ghi ảnh chụp trường của phiên bản hiện hành của `row`.
async fn snapshot<C: ConnectionTrait>(
    connection: &C,
    row: &form::Model,
    now: DateTime<Utc>,
) -> Result<(), Error> {
    form_version::Model {
        id: Uuid::now_v7(),
        form_id: row.id,
        version: row.version,
        fields: row.fields.clone(),
        created_at: now,
    }
    .into_active_model()
    .insert(connection)
    .await?;
    Ok(())
}

async fn key_taken<C: ConnectionTrait>(
    connection: &C,
    key: &str,
    except: Option<Uuid>,
) -> Result<bool, Error> {
    let mut select = form::Entity::find().filter(form::Column::Key.eq(key));
    if let Some(id) = except {
        select = select.filter(form::Column::Id.ne(id));
    }
    Ok(select.count(connection).await? > 0)
}

fn key_conflict(key: &str) -> Error {
    Error::conflict("forms.key_taken", "Another form already uses the key `{key}`. Choose another.")
        .with_param("key", key)
}

/// `key` của form: chữ thường và số, nhóm nối bằng `-`, tối đa 50 ký tự (nằm trong URL công
/// khai và trong phạm vi token `forms:<key>`).
pub(crate) fn is_valid_form_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= KEY_MAX
        && key.split('-').all(|part| {
            !part.is_empty() && part.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

/// `scheme://host[:port]` viết thường, không có `/` cuối; `None` nếu không phải origin
/// `http(s)` hợp lệ (có đường dẫn, query, thông tin đăng nhập…).
pub(crate) fn normalize_origin(raw: &str) -> Option<String> {
    let lower = raw.trim().trim_end_matches('/').to_ascii_lowercase();
    let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"))?;
    let valid = !rest.is_empty()
        && rest.len() <= 255
        && rest
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'));
    valid.then_some(lower)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_va_origin() {
        assert!(is_valid_form_key("contact-us-2"));
        for bad in ["", "Contact", "a--b", "-a", "a_b", &"a".repeat(51)] {
            assert!(!is_valid_form_key(bad), "{bad}");
        }
        assert_eq!(
            normalize_origin("HTTPS://Example.com/").as_deref(),
            Some("https://example.com")
        );
        assert_eq!(
            normalize_origin("http://localhost:3000").as_deref(),
            Some("http://localhost:3000")
        );
        for bad in ["example.com", "https://example.com/path", "ftp://x", "https://a@b", "null"] {
            assert_eq!(normalize_origin(bad), None, "{bad}");
        }
    }
}
