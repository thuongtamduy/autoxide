//! Tool MCP của module `forms`. Mỗi tool: một kiểu đầu vào, permission như endpoint REST tương
//! ứng (service tự kiểm), thân là lời gọi service như REST (luật kiến trúc 8). Không có tool gửi
//! form: MCP luôn có người gọi đăng nhập, còn gửi form là việc của người lạ trên website.
//!
//! Kiểu đầu vào riêng cho MCP, chỉ trường phẳng (JSON Schema của tool không `$ref`): danh sách
//! trường và lời cảm ơn khai là object chung, tool tự đọc thành kiểu thật và báo lỗi theo đường
//! dẫn (`fields[2]`); service kiểm phần còn lại y như REST. Tool đổi trạng thái bài gửi nhận
//! `version`, tool sửa form nhận `etag` — giá trị `ETag` tool đọc trả về — y như `If-Match` của
//! REST.
//!
//! Không có tool xoá form: xoá form xoá luôn mọi bài gửi, không hoàn tác được — agent đóng form
//! bằng `forms.update` (`status: closed`); xoá vẫn làm được qua REST (D-128).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::error::FieldError;
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::pagination::{Page, PageQuery};
use autoxide_kernel::version::{IfMatch, etag};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::fields::{Field, Localized};
use crate::forms::{Form, FormInput, FormStatus};
use crate::http::{FormResponse, SubmissionResponse, choice, page_request, present};
use crate::inbox::{InboxFilter, Submission, SubmissionStatus};
use crate::permissions::{FORMS_MANAGE, FORMS_READ, FORMS_REVIEW};
use crate::service::FormsService;

const STATUSES: &str = "`new`, `read`, `archived`, `spam`";

/// Bài gửi kèm `version` (giá trị `ETag`) để gửi lại khi đổi trạng thái.
#[derive(Debug, Serialize)]
struct Versioned {
    /// Gửi lại trong `version` của tool đổi trạng thái.
    version: String,
    #[serde(flatten)]
    submission: SubmissionResponse,
}

fn versioned(found: Submission) -> Versioned {
    Versioned { version: etag(found.row.updated_at), submission: found.into() }
}

/// Form kèm `etag` (giá trị `ETag`) để gửi lại khi sửa. Không gọi là `version` như tool khác:
/// form đã có trường `version` (phiên bản danh sách trường) — hai trường cùng tên sẽ đè nhau.
#[derive(Debug, Serialize)]
struct VersionedForm {
    /// Gửi lại trong `etag` của `forms.update`.
    etag: String,
    #[serde(flatten)]
    form: FormResponse,
}

fn versioned_form(form: Form) -> VersionedForm {
    VersionedForm { etag: etag(form.row.updated_at), form: form.into() }
}

const FORM_STATUSES: &str = "`open`, `closed`";

/// Đọc giá trị JSON chung thành kiểu thật; sai hình dạng thì thêm lỗi ở `path`.
fn decode<T: DeserializeOwned>(
    value: Value,
    path: String,
    problems: &mut Vec<FieldError>,
) -> Option<T> {
    serde_json::from_value(value)
        .map_err(|error| problems.push(FieldError::new(path, "invalid", error.to_string())))
        .ok()
}

/// Danh sách trường, mỗi phần tử báo lỗi ở `fields[i]`.
fn decode_fields(raw: Vec<Value>, problems: &mut Vec<FieldError>) -> Vec<Field> {
    raw.into_iter()
        .enumerate()
        .filter_map(|(index, value)| decode(value, format!("fields[{index}]"), problems))
        .collect()
}

// Hướng dẫn hình dạng trường, dùng trong mô tả của hai tool (mô tả phải là `&'static str`,
// nên ghép bằng `concat!` lúc biên dịch).
macro_rules! fields_help {
    () => {
        "`fields` is the whole list, in display order; each field is an \
     object `{key, type, label, help?, placeholder?, required?, options?, min_length?, \
     max_length?, min?, max?}`: `key` a lower-case letter then lower-case letters, digits or \
     `_`; `type` one of `text`, `email`, `phone`, `textarea`, `number`, `select`, `radio`, \
     `checkbox`, `date`, `hidden`, `consent` (at most one); `label`, `help`, `placeholder` \
     are `{\"en\": …, \"vi\": …}`; `options` (only `select`, `radio`, `checkbox`) is a \
     list of `{value, label?}`. `success_message` is `{\"en\": …, \"vi\": …}`."
    };
}

/// Filters and paging of the form list.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListFormsInput {
    /// `open` or `closed`; omitted: both.
    status: Option<String>,
    /// Maximum number of forms to return, 1 to 200. Defaults to 50.
    #[schema(minimum = 1, maximum = 200)]
    limit: Option<u32>,
    /// Where to continue: the `next_cursor` of the previous page.
    cursor: Option<String>,
}

/// A new form. Same rules as `POST /api/v1/forms`.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateFormInput {
    /// Lower-case letters and digits in groups joined by `-`, at most 50 characters; unique.
    key: String,
    /// Name for admins, 1 to 200 characters.
    name: String,
    /// `open` (the default) or `closed`.
    status: Option<String>,
    /// The fields, in display order (see the tool description for their shape).
    #[schema(value_type = Vec<Object>)]
    fields: Vec<Value>,
    /// Websites allowed to send the form from a browser, like `https://example.com`; empty
    /// or omitted: any.
    allowed_origins: Option<Vec<String>>,
    /// Shown after sending: `{"en": …, "vi": …}`.
    #[schema(value_type = Option<Object>)]
    success_message: Option<Value>,
    /// Addresses mailed for each new submission, up to 20.
    notify_emails: Option<Vec<String>>,
    /// Key of the `email` field used as `Reply-To`; omitted: the first `email` field.
    reply_to_field: Option<String>,
    /// Public submissions per hour from one network block; omitted: the server default.
    ip_limit_per_hour: Option<u32>,
    /// Public submissions per hour from one email address; omitted: the server default.
    email_limit_per_hour: Option<u32>,
    /// Days submissions are kept (`0`: until deleted); omitted: the server default.
    retention_days: Option<u32>,
}

/// Changes to a form. Omitted fields stay as they are; `null` clears the nullable ones.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateFormInput {
    /// Id of the form.
    id: Uuid,
    /// The `etag` you read (or `*`). Refused with `precondition_failed` if the form changed
    /// since: read it again and retry. (Not the form's `version`, which counts field changes.)
    etag: String,
    key: Option<String>,
    name: Option<String>,
    /// `open` or `closed` (closed forms refuse submissions with `forms.closed`).
    status: Option<String>,
    /// The whole new list of fields; a change makes a new form version, and old
    /// submissions keep the labels of their own version.
    #[schema(value_type = Option<Vec<Object>>)]
    fields: Option<Vec<Value>>,
    allowed_origins: Option<Vec<String>>,
    #[schema(value_type = Option<Object>)]
    success_message: Option<Value>,
    notify_emails: Option<Vec<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>, nullable)]
    reply_to_field: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<u32>, nullable)]
    ip_limit_per_hour: Option<Option<u32>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<u32>, nullable)]
    email_limit_per_hour: Option<Option<u32>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<u32>, nullable)]
    retention_days: Option<Option<u32>>,
}

/// A form to read.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FormId {
    /// Id of the form.
    id: Uuid,
}

/// Filters and paging of a form's submissions.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListSubmissionsInput {
    /// Id of the form.
    form_id: Uuid,
    /// `new`, `read`, `archived` or `spam`; omitted: every status but `spam`.
    status: Option<String>,
    /// Only submissions whose answers contain this text, ignoring case and accents.
    q: Option<String>,
    /// Maximum number of submissions to return, 1 to 200. Defaults to 50.
    #[schema(minimum = 1, maximum = 200)]
    limit: Option<u32>,
    /// Where to continue: the `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `desc` (newest first, the default) or `asc`.
    order: Option<String>,
}

/// A submission to read.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SubmissionId {
    /// Id of the submission.
    id: Uuid,
}

/// A new status for a submission.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetStatusInput {
    /// Id of the submission.
    id: Uuid,
    /// `new`, `read`, `archived` or `spam`.
    status: String,
    /// The `version` you read (or `*`). Refused with `precondition_failed` if the submission
    /// changed since: read it again and retry.
    version: String,
}

pub(crate) fn all(service: &Arc<FormsService>) -> Vec<Tool> {
    // Mỗi tool giữ một bản `Arc` của service (clone `Arc` chỉ tăng bộ đếm tham chiếu).
    let (list, get, create, update) =
        (service.clone(), service.clone(), service.clone(), service.clone());
    let (submissions, submission, status) = (service.clone(), service.clone(), service.clone());
    vec![
        Tool::new(
            "list",
            "List forms, newest first, with their fields and settings. Follow \
             `pagination.next_cursor` for more.",
            move |principal: Principal, input: ListFormsInput| {
                let service = list.clone();
                async move {
                    let mut problems = Vec::new();
                    let state = choice(
                        input.status.as_deref(),
                        "status",
                        "`open`, `closed`",
                        &mut problems,
                    );
                    let paging = PageQuery {
                        limit: input.limit.map(|limit| limit.to_string()),
                        cursor: input.cursor,
                        order: None,
                    };
                    let page = page_request(paging, problems)?;
                    let found = service.list_forms(&principal, state, &page).await?;
                    Ok::<Page<FormResponse>, Error>(found.map(FormResponse::from))
                }
            },
        )
        .requires(FORMS_READ)
        .read_only(),
        Tool::new(
            "get",
            "Get a form: its fields (keys, types, labels, options), status, settings, and its \
             `etag`, needed to change it.",
            move |principal: Principal, input: FormId| {
                let service = get.clone();
                async move {
                    Ok::<VersionedForm, Error>(versioned_form(
                        service.get_form(&principal, input.id).await?,
                    ))
                }
            },
        )
        .requires(FORMS_READ)
        .read_only(),
        Tool::new(
            "create",
            concat!(
                "Create a form (open unless `status` is `closed`); websites show it from \
                 `GET /api/v1/forms/public/{key}`. Every problem is reported at once. ",
                fields_help!()
            ),
            move |principal: Principal, input: CreateFormInput| {
                let service = create.clone();
                async move {
                    let mut problems = Vec::new();
                    let status: Option<FormStatus> =
                        choice(input.status.as_deref(), "status", FORM_STATUSES, &mut problems);
                    let fields = decode_fields(input.fields, &mut problems);
                    let success_message: Option<Localized> =
                        input.success_message.and_then(|value| {
                            decode(value, "success_message".to_owned(), &mut problems)
                        });
                    if !problems.is_empty() {
                        return Err(Error::validation(problems));
                    }
                    let form = FormInput {
                        key: Some(input.key),
                        name: Some(input.name),
                        status,
                        fields: Some(fields),
                        allowed_origins: input.allowed_origins,
                        success_message,
                        notify_emails: input.notify_emails,
                        reply_to_field: input.reply_to_field.map(Some),
                        ip_limit_per_hour: input.ip_limit_per_hour.map(Some),
                        email_limit_per_hour: input.email_limit_per_hour.map(Some),
                        retention_days: input.retention_days.map(Some),
                    };
                    Ok(versioned_form(service.create_form(&principal, form).await?))
                }
            },
        )
        .requires(FORMS_MANAGE),
        Tool::new(
            "update",
            concat!(
                "Change a form: only the given fields change. Close it with `status: closed` \
                 (there is no delete tool: deleting a form also deletes its submissions). Pass \
                 the `etag` you read. ",
                fields_help!()
            ),
            move |principal: Principal, input: UpdateFormInput| {
                let service = update.clone();
                async move {
                    let mut problems = Vec::new();
                    let status: Option<FormStatus> =
                        choice(input.status.as_deref(), "status", FORM_STATUSES, &mut problems);
                    let fields = input.fields.map(|raw| decode_fields(raw, &mut problems));
                    let success_message: Option<Localized> =
                        input.success_message.and_then(|value| {
                            decode(value, "success_message".to_owned(), &mut problems)
                        });
                    if !problems.is_empty() {
                        return Err(Error::validation(problems));
                    }
                    let expected = IfMatch::from_value(&input.etag).expected()?;
                    let form = FormInput {
                        key: input.key,
                        name: input.name,
                        status,
                        fields,
                        allowed_origins: input.allowed_origins,
                        success_message,
                        notify_emails: input.notify_emails,
                        reply_to_field: input.reply_to_field,
                        ip_limit_per_hour: input.ip_limit_per_hour,
                        email_limit_per_hour: input.email_limit_per_hour,
                        retention_days: input.retention_days,
                    };
                    let form = service.update_form(&principal, input.id, form, expected).await?;
                    Ok(versioned_form(form))
                }
            },
        )
        .requires(FORMS_MANAGE),
        Tool::new(
            "list_submissions",
            "List the submissions of a form, newest first, with their answers and the labels \
             of the form version they were sent with. Spam is left out unless `status` is \
             `spam`. Follow `pagination.next_cursor` with the same filters for more.",
            move |principal: Principal, input: ListSubmissionsInput| {
                let service = submissions.clone();
                async move {
                    let mut problems = Vec::new();
                    let filter = InboxFilter {
                        status: choice(input.status.as_deref(), "status", STATUSES, &mut problems),
                        search: input.q,
                    };
                    let page = page_request(
                        PageQuery {
                            limit: input.limit.map(|limit| limit.to_string()),
                            cursor: input.cursor,
                            order: input.order,
                        },
                        problems,
                    )?;
                    let found =
                        service.list_submissions(&principal, input.form_id, &filter, &page).await?;
                    Ok::<Page<SubmissionResponse>, Error>(found.map(SubmissionResponse::from))
                }
            },
        )
        .requires(FORMS_READ)
        .read_only(),
        Tool::new(
            "get_submission",
            "Get a submission: answers with labels, sender email, status, and its `version`, \
             needed to change its status.",
            move |principal: Principal, input: SubmissionId| {
                let service = submission.clone();
                async move {
                    Ok::<Versioned, Error>(versioned(
                        service.get_submission(&principal, input.id).await?,
                    ))
                }
            },
        )
        .requires(FORMS_READ)
        .read_only(),
        Tool::new(
            "set_submission_status",
            "Mark a submission `read`, `archived`, `spam`, or `new` (not spam after all: the \
             notification mail is then sent as for a new submission). Pass the `version` you \
             read.",
            move |principal: Principal, input: SetStatusInput| {
                let service = status.clone();
                async move {
                    let mut problems = Vec::new();
                    let parsed: Option<SubmissionStatus> =
                        choice(Some(&input.status), "status", STATUSES, &mut problems);
                    let Some(parsed) = parsed.filter(|_| problems.is_empty()) else {
                        return Err(Error::validation(problems));
                    };
                    let expected = IfMatch::from_value(&input.version).expected()?;
                    let found = service
                        .set_submission_status(&principal, input.id, parsed, expected)
                        .await?;
                    Ok(versioned(found))
                }
            },
        )
        .requires(FORMS_REVIEW),
    ]
}
