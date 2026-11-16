//! Hộp thư quản trị: liệt kê, đọc, đổi trạng thái, xoá bài gửi; xuất CSV; xuất / xoá mọi bài
//! của một email (quyền của chủ dữ liệu). Mỗi bài đọc kèm trường của ĐÚNG phiên bản lúc gửi.

use std::collections::{HashMap, HashSet};

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::event::Actor;
use autoxide_kernel::pagination::{Order, Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{ScopeExt, TenantContext};
use chrono::{DateTime, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::{export, form, form_version, submission};
use crate::events::{self, Change};
use crate::forms::Form;
use crate::forms::fields::{self, Field};
use crate::permissions::{FORMS_EXPORT, FORMS_READ, FORMS_REVIEW};
use crate::service::{FormsService, from_json, now, submission_not_found};
use crate::submissions::csv;

/// Where a submission is in the inbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SubmissionStatus {
    /// Not looked at yet.
    New,
    /// Looked at.
    Read,
    /// Dealt with; kept.
    Archived,
    /// Held by the anti-spam checks or marked by hand; deleted after
    /// `AUTOXIDE_FORMS_SPAM_RETENTION_DAYS`. Moving it out raises `forms.submission_received`.
    Spam,
}

impl SubmissionStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            SubmissionStatus::New => "new",
            SubmissionStatus::Read => "read",
            SubmissionStatus::Archived => "archived",
            SubmissionStatus::Spam => "spam",
        }
    }

    pub(crate) fn parse(raw: &str) -> Option<SubmissionStatus> {
        match raw {
            "new" => Some(SubmissionStatus::New),
            "read" => Some(SubmissionStatus::Read),
            "archived" => Some(SubmissionStatus::Archived),
            "spam" => Some(SubmissionStatus::Spam),
            _ => None,
        }
    }
}

/// Một bài gửi, kèm `key` của form và trường của đúng phiên bản lúc gửi.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Submission {
    pub row: submission::Model,
    pub form_key: String,
    pub fields: Vec<Field>,
    pub data: Map<String, Value>,
    pub consent: Option<Value>,
}

impl Submission {
    pub(crate) fn status(&self) -> SubmissionStatus {
        SubmissionStatus::parse(&self.row.status).unwrap_or(SubmissionStatus::New)
    }
}

/// Bộ lọc hộp thư.
#[derive(Debug, Clone, Default)]
pub(crate) struct InboxFilter {
    /// `None`: mọi trạng thái trừ `spam`.
    pub status: Option<SubmissionStatus>,
    pub search: Option<String>,
}

/// Tóm tắt bài gửi cho event: không có câu trả lời, không có email.
/// Số bài mỗi trang khi xuất CSV.
const EXPORT_PAGE: u32 = 200;

/// Cột của bản xuất CSV: dòng tiêu đề, và `key` của các trường theo thứ tự cột.
pub(crate) struct ExportColumns {
    pub header: Vec<String>,
    pub keys: Vec<String>,
}

/// Tên file của bản xuất: `<key>-submissions.csv`.
pub(crate) fn export_file_name(form: &Form) -> String {
    format!("{}-submissions.csv", form.row.key)
}

pub(crate) fn summary(row: &submission::Model, form_key: &str) -> Value {
    json!({
        "id": row.id,
        "form_id": row.form_id,
        "form_key": form_key,
        "status": row.status,
    })
}

/// Submissions of a form by status.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub(crate) struct SubmissionCounts {
    pub new: i64,
    pub read: i64,
    pub archived: i64,
    pub spam: i64,
}

impl FormsService {
    /// Số bài gửi theo trạng thái của từng form trong `form_ids`: một câu `GROUP BY` cho cả trang.
    /// Form chưa có bài nào thì vắng trong kết quả.
    pub(crate) async fn submission_counts(
        &self,
        tenant: &TenantContext,
        form_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, SubmissionCounts>, Error> {
        if form_ids.is_empty() {
            return Ok(HashMap::new());
        }
        let rows: Vec<(Uuid, String, i64)> = submission::Entity::find()
            .select_only()
            .column(submission::Column::FormId)
            .column(submission::Column::Status)
            .expr(sea_orm::sea_query::Func::count(Expr::col(submission::Column::Id)))
            .filter(submission::Column::FormId.is_in(form_ids.iter().copied()))
            .group_by(submission::Column::FormId)
            .group_by(submission::Column::Status)
            .scoped(tenant)
            .into_tuple()
            .all(tenant.database().read())
            .await?;
        let mut counts: HashMap<Uuid, SubmissionCounts> = HashMap::new();
        for (form_id, status, count) in rows {
            let entry = counts.entry(form_id).or_default();
            match SubmissionStatus::parse(&status) {
                Some(SubmissionStatus::New) => entry.new = count,
                Some(SubmissionStatus::Read) => entry.read = count,
                Some(SubmissionStatus::Archived) => entry.archived = count,
                Some(SubmissionStatus::Spam) => entry.spam = count,
                None => {}
            }
        }
        Ok(counts)
    }

    /// Một trang bài gửi của form `form_id`, mới trước (mặc định). Cần `forms.read`.
    pub(crate) async fn list_submissions(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        form_id: Uuid,
        filter: &InboxFilter,
        page: &PageRequest,
    ) -> Result<Page<Submission>, Error> {
        principal.require(&FORMS_READ)?;
        self.check_form_id(tenant, principal, &FORMS_READ, form_id).await?;
        let mut select = submission::Entity::find()
            .scoped(tenant)
            .filter(submission::Column::FormId.eq(form_id));
        select = match filter.status {
            Some(status) => select.filter(submission::Column::Status.eq(status.as_str())),
            None => select.filter(submission::Column::Status.ne(SubmissionStatus::Spam.as_str())),
        };
        if let Some(search) = filter.search.as_deref().filter(|search| !search.trim().is_empty()) {
            select = select
                .filter(autoxide_kernel::search::contains(submission::Column::SearchKey, search));
        }
        let Page { data, pagination } = fetch_page(
            select,
            submission::Column::Id,
            |row| row.id,
            page,
            tenant.database().read(),
        )
        .await?;
        Ok(Page { data: self.describe(tenant, data).await?, pagination })
    }

    /// Bài gửi `id`. Cần `forms.read`.
    pub(crate) async fn get_submission(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<Submission, Error> {
        principal.require(&FORMS_READ)?;
        let found = self.find_submission(tenant, id).await?;
        self.check_submission(tenant, principal, &FORMS_READ, &found).await?;
        Ok(found)
    }

    /// `permission` phủ form của bài gửi; không phủ → như không có bài (404).
    async fn check_submission(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        permission: &autoxide_kernel::access::Permission,
        found: &Submission,
    ) -> Result<(), Error> {
        let id = found.row.id;
        self.check_form_id(tenant, principal, permission, found.row.form_id).await.map_err(
            |error| {
                if error.code() == "forms.not_found" { submission_not_found(id) } else { error }
            },
        )
    }

    pub(crate) async fn find_submission(
        &self,
        tenant: &TenantContext,
        id: Uuid,
    ) -> Result<Submission, Error> {
        let row = submission::Entity::find_by_id(id)
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .ok_or_else(|| submission_not_found(id))?;
        self.describe(tenant, vec![row])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| Error::internal("describe returned nothing"))
    }

    /// Đổi trạng thái bài gửi `id`. Cần `forms.review`. Ra khỏi `spam` thì phát thêm
    /// `forms.submission_received` (thông báo cho quản trị viên như bài mới).
    pub(crate) async fn set_submission_status(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        status: SubmissionStatus,
        expected: Option<DateTime<Utc>>,
    ) -> Result<Submission, Error> {
        principal.require(&FORMS_REVIEW)?;
        let before = self.find_submission(tenant, id).await?;
        self.check_submission(tenant, principal, &FORMS_REVIEW, &before).await?;
        if expected.is_some_and(|expected| expected != before.row.updated_at) {
            return Err(autoxide_kernel::version::changed());
        }
        if before.status() == status {
            return Ok(before);
        }
        let now = now();
        let mut row = before.row.clone().into_active_model();
        row.status = Set(status.as_str().to_owned());
        if status != SubmissionStatus::Spam {
            row.spam_reason = Set(None);
        }
        row.updated_at = Set(now);
        let transaction = tenant.database().begin_write().await?;
        let update = submission::Entity::update(row)
            .validate()?
            .scoped(tenant)
            .filter(submission::Column::UpdatedAt.eq(expected.unwrap_or(before.row.updated_at)));
        let after = match update.exec(&transaction).await {
            Ok(after) => after,
            Err(sea_orm::DbErr::RecordNotUpdated) => {
                return Err(autoxide_kernel::version::changed());
            }
            Err(error) => return Err(error.into()),
        };
        let actor = Actor::from(principal);
        let subject = events::submission_subject(id);
        let change = Change {
            before: Some(summary(&before.row, &before.form_key)),
            after: Some(summary(&after, &before.form_key)),
        };
        self.outbox
            .record(
                &transaction,
                tenant,
                actor,
                subject.clone(),
                &events::SubmissionStatusChanged(change),
            )
            .await?;
        if before.status() == SubmissionStatus::Spam {
            let received = Change { before: None, after: Some(summary(&after, &before.form_key)) };
            self.outbox
                .record(&transaction, tenant, actor, subject, &events::SubmissionReceived(received))
                .await?;
        }
        transaction.commit().await?;
        Ok(Submission { row: after, ..before })
    }

    /// Xoá bài gửi `id`. Cần `forms.review`.
    pub(crate) async fn delete_submission(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        principal.require(&FORMS_REVIEW)?;
        let before = self.find_submission(tenant, id).await?;
        self.check_submission(tenant, principal, &FORMS_REVIEW, &before).await?;
        let transaction = tenant.database().begin_write().await?;
        let deleted =
            submission::Entity::delete_by_id(id).scoped(tenant).exec(&transaction).await?;
        if deleted.rows_affected == 0 {
            return Err(submission_not_found(id));
        }
        let change = Change { before: Some(summary(&before.row, &before.form_key)), after: None };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                events::submission_subject(id),
                &events::SubmissionDeleted(change),
            )
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    /// Mọi bài gửi của email `email` (yêu cầu xuất dữ liệu của chủ dữ liệu; xem
    /// [`rows_of_email`]). Cần `forms.export`.
    pub(crate) async fn submissions_of_email(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        email: &str,
    ) -> Result<Vec<Submission>, Error> {
        principal.require(&FORMS_EXPORT)?;
        self.require_whole_tenant(tenant, principal, &FORMS_EXPORT).await?;
        let email = checked_email(email)?;
        let mut rows = rows_of_email(tenant.database().read(), tenant, &email).await?;
        rows.sort_by_key(|row| row.id);
        self.describe(tenant, rows).await
    }

    /// Xoá hẳn mọi bài gửi của email `email` (yêu cầu xoá của chủ dữ liệu; xem
    /// [`rows_of_email`]). Cần `forms.export`. Event không mang email. Trả số bài đã xoá.
    pub(crate) async fn erase_email(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        email: &str,
    ) -> Result<u64, Error> {
        principal.require(&FORMS_EXPORT)?;
        self.require_whole_tenant(tenant, principal, &FORMS_EXPORT).await?;
        let email = checked_email(email)?;
        let transaction = tenant.database().begin_write().await?;
        let rows = rows_of_email(&transaction, tenant, &email).await?;
        if rows.is_empty() {
            return Ok(0);
        }
        let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
        let form_ids: Vec<Uuid> =
            rows.iter().map(|row| row.form_id).collect::<HashSet<_>>().into_iter().collect();
        let deleted = submission::Entity::delete_many()
            .scoped(tenant)
            .filter(submission::Column::Id.is_in(ids.clone()))
            .exec(&transaction)
            .await?;
        let change = Change {
            before: Some(json!({"submission_ids": ids, "count": deleted.rows_affected})),
            after: None,
        };
        self.outbox
            .record(
                &transaction,
                tenant,
                Actor::from(principal),
                None,
                &events::SubmissionsErased(change),
            )
            .await?;
        transaction.commit().await?;
        tracing::info!(count = deleted.rows_affected, "submissions of an email address erased");
        // File xuất của các form đó có thể còn chứa bài vừa xoá: xoá hết (sau transaction — nơi
        // lưu có thể ở xa). Xoá file lỗi thì dòng còn lại, job dọn hằng ngày thử lại.
        let exports = self.exports_of(tenant, form_ids).await?;
        self.purge_exports(tenant, exports).await?;
        Ok(deleted.rows_affected)
    }

    /// Bài gửi của form `form_id` dạng CSV (tên file, nội dung). Cần `forms.export`. `status`
    /// `None`: mọi trạng thái trừ `spam`. Form nhiều bài thì xuất qua job
    /// (`submissions/exports.rs`): cùng nội dung, từng byte.
    pub(crate) async fn export_csv(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        form_id: Uuid,
        status: Option<SubmissionStatus>,
    ) -> Result<(String, String), Error> {
        principal.require(&FORMS_EXPORT)?;
        let form = self.find_form(tenant, form_id).await?;
        self.check_form(tenant, principal, &FORMS_EXPORT, &form.row).await?;
        let columns = self.export_columns(tenant, &form).await?;
        let mut text = String::from(csv::BOM);
        text.push_str(&csv::line(&columns.header));
        let mut cursor = None;
        loop {
            let (rows, next) =
                self.export_page(tenant, form_id, status, &columns.keys, cursor).await?;
            for row in &rows {
                text.push_str(&csv::line(row));
            }
            match next {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        Ok((export_file_name(&form), text))
    }

    /// Cột của bản xuất: cột cố định, rồi trường của phiên bản hiện hành, rồi trường chỉ có ở
    /// phiên bản cũ.
    pub(crate) async fn export_columns(
        &self,
        tenant: &TenantContext,
        form: &Form,
    ) -> Result<ExportColumns, Error> {
        let form_id = form.row.id;
        let versions = self.versions(tenant, &[form_id]).await?;
        let mut keys: Vec<String> = form.fields.iter().map(|field| field.key.clone()).collect();
        let mut older: Vec<(&i32, &Vec<Field>)> = versions
            .iter()
            .filter(|((id, _), _)| *id == form_id)
            .map(|((_, v), f)| (v, f))
            .collect();
        older.sort_by(|a, b| b.0.cmp(a.0));
        for (_, fields) in older {
            for field in fields {
                if !keys.contains(&field.key) {
                    keys.push(field.key.clone());
                }
            }
        }
        let mut header: Vec<String> = [
            "id",
            "created_at",
            "status",
            "form_version",
            "locale",
            "email",
            "page_url",
            "referrer",
        ]
        .map(str::to_owned)
        .to_vec();
        header.extend(keys.iter().cloned());
        Ok(ExportColumns { header, keys })
    }

    /// Một trang (200 bài, cũ trước) của bản xuất, sau bài `after`: các dòng, và con trỏ của
    /// trang sau (`None`: hết).
    pub(crate) async fn export_page(
        &self,
        tenant: &TenantContext,
        form_id: Uuid,
        status: Option<SubmissionStatus>,
        keys: &[String],
        after: Option<Uuid>,
    ) -> Result<(Vec<Vec<String>>, Option<Uuid>), Error> {
        let mut page = PageRequest::first(EXPORT_PAGE, Order::Asc);
        if let Some(after) = after {
            page = page.after(after);
        }
        let mut select = submission::Entity::find()
            .scoped(tenant)
            .filter(submission::Column::FormId.eq(form_id));
        select = match status {
            Some(status) => select.filter(submission::Column::Status.eq(status.as_str())),
            None => select.filter(submission::Column::Status.ne(SubmissionStatus::Spam.as_str())),
        };
        let found = fetch_page(
            select,
            submission::Column::Id,
            |row| row.id,
            &page,
            tenant.database().read(),
        )
        .await?;
        let mut rows = Vec::with_capacity(found.data.len());
        for row in &found.data {
            let data: Map<String, Value> = from_json(&row.data)?;
            let mut cells = vec![
                row.id.to_string(),
                row.created_at.to_rfc3339(),
                row.status.clone(),
                row.form_version.to_string(),
                row.locale.clone().unwrap_or_default(),
                row.email.clone().unwrap_or_default(),
                row.page_url.clone().unwrap_or_default(),
                row.referrer.clone().unwrap_or_default(),
            ];
            cells.extend(
                keys.iter().map(|key| data.get(key).map(fields::display).unwrap_or_default()),
            );
            rows.push(cells);
        }
        let next = found.pagination.next_cursor.as_deref().and_then(|cursor| cursor.parse().ok());
        Ok((rows, next))
    }

    /// Đọc `rows` ra kèm `key` của form và trường của đúng phiên bản.
    pub(crate) async fn describe(
        &self,
        tenant: &TenantContext,
        rows: Vec<submission::Model>,
    ) -> Result<Vec<Submission>, Error> {
        let form_ids: Vec<Uuid> =
            rows.iter().map(|row| row.form_id).collect::<HashSet<_>>().into_iter().collect();
        let keys: HashMap<Uuid, String> = form::Entity::find()
            .scoped(tenant)
            .filter(form::Column::Id.is_in(form_ids.clone()))
            .all(tenant.database().read())
            .await?
            .into_iter()
            .map(|row| (row.id, row.key))
            .collect();
        let versions = self.versions(tenant, &form_ids).await?;
        rows.into_iter()
            .map(|row| {
                let fields =
                    versions.get(&(row.form_id, row.form_version)).cloned().unwrap_or_default();
                Ok(Submission {
                    form_key: keys.get(&row.form_id).cloned().unwrap_or_default(),
                    data: from_json(&row.data)?,
                    consent: row.consent.as_deref().map(from_json).transpose()?,
                    fields,
                    row,
                })
            })
            .collect()
    }

    /// Ảnh chụp trường của mọi phiên bản các form `form_ids`, theo (form, phiên bản).
    async fn versions(
        &self,
        tenant: &TenantContext,
        form_ids: &[Uuid],
    ) -> Result<HashMap<(Uuid, i32), Vec<Field>>, Error> {
        if form_ids.is_empty() {
            return Ok(HashMap::new());
        }
        form_version::Entity::find()
            .scoped(tenant)
            .filter(form_version::Column::FormId.is_in(form_ids.to_vec()))
            .all(tenant.database().read())
            .await?
            .into_iter()
            .map(|row| Ok(((row.form_id, row.version), from_json(&row.fields)?)))
            .collect()
    }

    /// Bỏ `created_by` khỏi form của một người dùng đã bị xoá.
    pub(crate) async fn forget_creator(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<u64, Error> {
        let updated = form::Entity::update_many()
            .col_expr(form::Column::CreatedBy, Expr::value(Option::<Uuid>::None))
            .scoped(tenant)
            .filter(form::Column::CreatedBy.eq(user_id))
            .exec(tenant.database().write())
            .await?;
        export::Entity::update_many()
            .col_expr(export::Column::RequestedBy, Expr::value(Option::<Uuid>::None))
            .scoped(tenant)
            .filter(export::Column::RequestedBy.eq(user_id))
            .exec(tenant.database().write())
            .await?;
        Ok(updated.rows_affected)
    }

    /// Form do người dùng `user_id` tạo (cho bản xuất dữ liệu cá nhân).
    pub(crate) async fn forms_created_by(
        &self,
        tenant: &TenantContext,
        user_id: Uuid,
    ) -> Result<Vec<Form>, Error> {
        form::Entity::find()
            .scoped(tenant)
            .filter(form::Column::CreatedBy.eq(user_id))
            .all(tenant.database().read())
            .await?
            .into_iter()
            .map(Form::parse)
            .collect()
    }
}

/// Bài gửi của email `email` (đã chuẩn hoá): email người gửi (cột `email`) là nó, HOẶC một câu
/// trả lời nào đó đúng bằng địa chỉ đó (vd trường email thứ hai "email người được giới thiệu").
///
/// Lọc thô trong database (cột `email`, hoặc `search_key` — chữ của mọi câu trả lời đã qua
/// `search::fold` — có chứa địa chỉ; `LIKE` chạy giống nhau trên SQLite và Postgres), rồi giữ
/// đúng những bài có câu trả lời BẰNG địa chỉ sau khi chuẩn hoá. Quét `search_key` không dùng được
/// index — chấp nhận được cho yêu cầu hiếm của chủ dữ liệu.
async fn rows_of_email<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    email: &str,
) -> Result<Vec<submission::Model>, Error> {
    let rows = submission::Entity::find()
        .scoped(tenant)
        .filter(
            Condition::any()
                .add(submission::Column::Email.eq(email))
                .add(autoxide_kernel::search::contains(submission::Column::SearchKey, email)),
        )
        .all(connection)
        .await?;
    let mut matched = Vec::with_capacity(rows.len());
    for row in rows {
        if row.email.as_deref() == Some(email) || answers_contain(&row.data, email)? {
            matched.push(row);
        }
    }
    Ok(matched)
}

/// Có câu trả lời nào (chuỗi, hay một phần tử của mảng) đúng bằng `email` sau khi chuẩn hoá.
fn answers_contain(data: &str, email: &str) -> Result<bool, Error> {
    let data: Map<String, Value> = from_json(data)?;
    let equal = |value: &Value| {
        value.as_str().is_some_and(|text| autoxide_kernel::email::normalize(text) == email)
    };
    Ok(data.values().any(|value| match value {
        Value::Array(items) => items.iter().any(equal),
        other => equal(other),
    }))
}

/// Email của yêu cầu xuất / xoá, đã chuẩn hoá; sai dạng thì 422.
fn checked_email(email: &str) -> Result<String, Error> {
    let mut problems = Vec::new();
    autoxide_kernel::email::checked(email, "email", &mut problems)
        .ok_or_else(|| Error::validation(problems))
}
