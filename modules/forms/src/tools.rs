//! Tool MCP của module `forms`. Mỗi tool: một kiểu đầu vào, permission như endpoint REST tương
//! ứng (service tự kiểm), thân là lời gọi service như REST (luật kiến trúc 8). Không có tool gửi
//! form: MCP luôn có người gọi đăng nhập, còn gửi form là việc của người lạ trên website.
//!
//! Kiểu đầu vào riêng cho MCP, chỉ trường phẳng (JSON Schema của tool không `$ref`). Tool đổi
//! trạng thái nhận `version` — giá trị `ETag` tool đọc trả về — y như `If-Match` của REST.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::pagination::{Page, PageQuery};
use autoxide_kernel::version::{IfMatch, etag};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::http::{FormResponse, SubmissionResponse, choice, page_request};
use crate::inbox::{InboxFilter, Submission, SubmissionStatus};
use crate::permissions::{FORMS_READ, FORMS_REVIEW};
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
    let (list, get, submissions, submission, status) =
        (service.clone(), service.clone(), service.clone(), service.clone(), service.clone());
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
            "Get a form: its fields (keys, types, labels, options), status, version and \
             settings.",
            move |principal: Principal, input: FormId| {
                let service = get.clone();
                async move {
                    Ok::<FormResponse, Error>(service.get_form(&principal, input.id).await?.into())
                }
            },
        )
        .requires(FORMS_READ)
        .read_only(),
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
