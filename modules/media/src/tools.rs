//! Tool MCP của module `media`. Mỗi tool: một kiểu đầu vào (kiểm như REST), permission như
//! endpoint REST tương ứng (service tự kiểm), thân là MỘT lời gọi service (luật kiến trúc 8).
//! Không có tool tải lên ở bước này: nội dung nhị phân qua JSON-RPC phải mã hoá base64,
//! nặng và dễ vượt giới hạn — AI agent tải lên qua REST (D-090).

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::pagination::{Page, PageQuery};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::http::{ItemsQuery, MediaItemResponse, present};
use crate::permissions::{MEDIA_READ, MEDIA_WRITE};
use crate::service::{ItemChanges, MediaService, Visibility};

/// Filters, order and paging of the media list; omitted fields do not filter.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListMediaInput {
    /// Only items in this folder (not its subfolders): a folder id, or `root`.
    folder_id: Option<String>,
    /// Only this media type (`image/png`) or family (`image/*`).
    #[serde(rename = "type")]
    #[schema(rename = "type")]
    content_type: Option<String>,
    /// `public` or `private`.
    visibility: Option<String>,
    /// `true`: only items in the trash. Default: only items not in the trash.
    trashed: Option<bool>,
    /// Only items whose name contains this text, ignoring case.
    q: Option<String>,
    /// `created_at` (default), `name` or `size`.
    sort: Option<String>,
    /// Maximum number of items to return, 1 to 200. Defaults to 50.
    #[schema(minimum = 1, maximum = 200)]
    limit: Option<u32>,
    /// Where to continue: the `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `desc` (default) or `asc`.
    order: Option<String>,
}

/// A media item to act on.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MediaId {
    /// Id of the media item.
    id: Uuid,
}

/// Changes to a media item; omitted fields stay as they are.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateMediaInput {
    /// Id of the media item.
    id: Uuid,
    /// The `updated_at` of the item as you last read it. The change is refused if the
    /// item changed since; read it again and retry.
    updated_at: DateTime<Utc>,
    /// New file name, 1 to 255 characters.
    name: Option<String>,
    /// Text alternative for screen readers, up to 1000 characters; `""` clears it.
    alt: Option<String>,
    /// Title, up to 255 characters; `""` clears it.
    title: Option<String>,
    /// Caption, up to 2000 characters; `""` clears it.
    caption: Option<String>,
    /// Move to this folder; `null` moves to the root.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Uuid>, nullable)]
    folder_id: Option<Option<Uuid>>,
    /// `public` (anyone with the link may load the content) or `private`.
    visibility: Option<String>,
}

pub(crate) fn all(service: &Arc<MediaService>) -> Vec<Tool> {
    // Mỗi tool giữ một bản `Arc` của service (clone `Arc` chỉ tăng bộ đếm tham chiếu).
    let (list, get, update, trash) =
        (service.clone(), service.clone(), service.clone(), service.clone());
    vec![
        Tool::new(
            "list",
            "List media items (images, video, audio, documents), newest first. Filter by \
             folder, type, visibility, trash or name; sort by `created_at`, `name` or `size`. \
             Follow `pagination.next_cursor` with the same filters for more.",
            move |principal: Principal, input: ListMediaInput| {
                let service = list.clone();
                async move {
                    let query = ItemsQuery {
                        folder_id: input.folder_id,
                        content_type: input.content_type,
                        visibility: input.visibility,
                        trashed: input.trashed.map(|trashed| trashed.to_string()),
                        q: input.q,
                        sort: input.sort,
                    };
                    let paging = PageQuery {
                        limit: input.limit.map(|limit| limit.to_string()),
                        cursor: input.cursor,
                        order: input.order,
                    };
                    let mut problems = Vec::new();
                    let filter = query.filter(&mut problems);
                    let page = crate::http::page_request(paging, problems)?;
                    let page = service.list(&principal, &filter, &page).await?;
                    Ok::<Page<MediaItemResponse>, Error>(page.map(MediaItemResponse::from))
                }
            },
        )
        .requires(MEDIA_READ)
        .read_only(),
        Tool::new(
            "get",
            "Get a media item: name, type, size, image width and height, texts, visibility, \
             and the `url` of its content.",
            move |principal: Principal, input: MediaId| {
                let service = get.clone();
                async move {
                    Ok::<_, Error>(MediaItemResponse::from(
                        service.get(&principal, input.id).await?,
                    ))
                }
            },
        )
        .requires(MEDIA_READ)
        .read_only(),
        Tool::new(
            "update",
            "Change the details of a media item: name, alt text, title, caption, folder or \
             visibility. Pass the `updated_at` you read; the change is refused if someone \
             changed the item since.",
            move |principal: Principal, input: UpdateMediaInput| {
                let service = update.clone();
                async move {
                    let visibility = match input.visibility.as_deref() {
                        None => None,
                        Some("public") => Some(Visibility::Public),
                        Some("private") => Some(Visibility::Private),
                        Some(_) => {
                            return Err(Error::validation(vec![FieldError::new(
                                "visibility",
                                "invalid",
                                "must be `public` or `private`",
                            )]));
                        }
                    };
                    let changes = ItemChanges {
                        name: input.name,
                        alt: input.alt,
                        title: input.title,
                        caption: input.caption,
                        folder_id: input.folder_id,
                        visibility,
                    };
                    let item = service
                        .update(&principal, input.id, changes, Some(input.updated_at))
                        .await?;
                    Ok(MediaItemResponse::from(item))
                }
            },
        )
        .requires(MEDIA_WRITE),
        Tool::new(
            "trash",
            "Move a media item to the trash. Its public link stops working; it can be \
             restored until the server deletes it for good (30 days by default).",
            move |principal: Principal, input: MediaId| {
                let service = trash.clone();
                async move {
                    Ok::<_, Error>(MediaItemResponse::from(
                        service.trash(&principal, input.id).await?,
                    ))
                }
            },
        )
        .requires(MEDIA_WRITE),
    ]
}
