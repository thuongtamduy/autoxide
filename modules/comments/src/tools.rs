//! Tool MCP của module `comments` (đề bài F): xem hàng chờ, xem bình luận, duyệt / từ chối / đánh
//! spam, mở / đóng đích. Mỗi tool: một kiểu đầu vào phẳng (JSON Schema của tool không `$ref`),
//! permission như endpoint REST tương ứng (service tự kiểm), thân là lời gọi service như REST
//! (luật kiến trúc 8). Tool đổi trạng thái nhận `version` — giá trị `ETag` tool đọc trả về — y như
//! `If-Match` của REST.
//!
//! Không có tool viết bình luận (MCP dành cho kiểm duyệt; bình luận là việc của người đọc trên
//! website) và không có tool xoá hẳn (không hoàn tác được; làm qua REST).

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

use crate::http::{CommentDetail, TargetResponse, choice, page_request};
use crate::moderation::QueueFilter;
use crate::permissions::{COMMENTS_MANAGE, COMMENTS_MODERATE, COMMENTS_READ};
use crate::service::{Comment, CommentsService, STATUSES, Status};
use crate::targets::TargetChange;

/// Bình luận kèm `version` (giá trị `ETag`) để gửi lại khi đổi trạng thái.
#[derive(Debug, Serialize)]
struct Versioned {
    /// Gửi lại trong `version` của `comments.moderate`.
    version: String,
    #[serde(flatten)]
    comment: CommentDetail,
}

fn versioned(found: Comment) -> Versioned {
    Versioned { version: etag(found.row.updated_at), comment: found.into() }
}

/// Filters and paging of the moderation queue.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct QueueInput {
    /// `pending` (the default), `published`, `rejected`, `spam` or `deleted`.
    status: Option<String>,
    /// Only comments on targets of this kind, e.g. `content.entry`.
    target_type: Option<String>,
    /// Only comments on this target.
    target_id: Option<String>,
    /// Only comments whose text or author name contains this, ignoring case and accents.
    q: Option<String>,
    /// Maximum number of comments to return, 1 to 200. Defaults to 50.
    #[schema(minimum = 1, maximum = 200)]
    limit: Option<u32>,
    /// Where to continue: the `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `desc` (newest first, the default) or `asc`.
    order: Option<String>,
}

/// A comment to read.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CommentId {
    /// Id of the comment.
    id: Uuid,
}

/// A moderation decision.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModerateInput {
    /// Id of the comment.
    id: Uuid,
    /// `published` (approve), `rejected`, `spam` or `pending` (hold again).
    status: String,
    /// The `version` you read (or `*`). Refused with `precondition_failed` if the comment
    /// changed since: read it again and retry.
    version: String,
}

/// A new comment state for a target.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TargetInput {
    /// Kind of the target, e.g. `content.entry`.
    target_type: String,
    /// Id of the target in its module.
    target_id: String,
    /// `open` or `closed`.
    state: String,
}

pub(crate) fn all(service: &Arc<CommentsService>) -> Vec<Tool> {
    // Mỗi tool giữ một bản `Arc` của service (clone `Arc` chỉ tăng bộ đếm tham chiếu).
    let (queue, get, moderate, target) =
        (service.clone(), service.clone(), service.clone(), service.clone());
    vec![
        Tool::new(
            "queue",
            "List comments for moderation, newest first: by default those waiting for approval \
             (`pending`), with their text, author, email, hold reason and report count. Follow \
             `pagination.next_cursor` with the same filters for more.",
            move |principal: Principal, input: QueueInput| {
                let service = queue.clone();
                async move {
                    let mut problems = Vec::new();
                    let filter = QueueFilter {
                        status: choice(input.status.as_deref(), "status", STATUSES, &mut problems),
                        target_type: input.target_type,
                        target_id: input.target_id,
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
                    let found = service.queue(&principal, &filter, &page).await?;
                    Ok::<Page<CommentDetail>, Error>(found.map(CommentDetail::from))
                }
            },
        )
        .requires(COMMENTS_READ)
        .read_only(),
        Tool::new(
            "get",
            "Get a comment in any status with its moderation details and its `version`, needed \
             to approve, reject or mark it as spam.",
            move |principal: Principal, input: CommentId| {
                let service = get.clone();
                async move {
                    Ok::<Versioned, Error>(versioned(service.get(&principal, input.id).await?))
                }
            },
        )
        .requires(COMMENTS_READ)
        .read_only(),
        Tool::new(
            "moderate",
            "Approve (`published`), reject (`rejected`), mark as spam (`spam`) or hold again \
             (`pending`) a comment. Approving shows it and mails people following the comment \
             it answers. Pass the `version` you read.",
            move |principal: Principal, input: ModerateInput| {
                let service = moderate.clone();
                async move {
                    let mut problems = Vec::new();
                    let status: Option<Status> =
                        choice(Some(&input.status), "status", STATUSES, &mut problems);
                    let Some(status) = status.filter(|_| problems.is_empty()) else {
                        return Err(Error::validation(problems));
                    };
                    let expected = IfMatch::from_value(&input.version).expected()?;
                    let found = service.set_status(&principal, input.id, status, expected).await?;
                    Ok(versioned(found))
                }
            },
        )
        .requires(COMMENTS_MODERATE),
        Tool::new(
            "set_target_state",
            "Open or close the comments of a target (`open` / `closed`); closed targets take no \
             new comments, existing ones stay shown.",
            move |principal: Principal, input: TargetInput| {
                let service = target.clone();
                async move {
                    let change =
                        TargetChange { state: Some(input.state), ..TargetChange::default() };
                    let found = service
                        .set_target(&principal, &input.target_type, &input.target_id, change, None)
                        .await?;
                    Ok::<TargetResponse, Error>(TargetResponse::new(found, true))
                }
            },
        )
        .requires(COMMENTS_MANAGE),
    ]
}
