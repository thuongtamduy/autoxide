//! Tool MCP của module `media`. Mỗi tool: một kiểu đầu vào (kiểm như REST), permission như
//! endpoint REST tương ứng (service tự kiểm), thân là MỘT lời gọi service (luật kiến trúc 8).
//! Tải lên qua MCP (`media.upload`, D-095): nội dung base64 trong lời gọi tool, chỉ cho file
//! nhỏ ([`MAX_TOOL_BYTES`]) — lời gọi MCP còn chịu giới hạn thân request chung của máy chủ.
//! File lớn tải lên qua REST.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::pagination::{Page, PageQuery};
use autoxide_kernel::tenancy::TenantContext;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use crate::http::{
    AltSuggestionResponse, DuplicateItem, ItemsQuery, TagResponse, UploadedResponse, present,
};
use crate::library::tags::TagSort;
use crate::library::upload::NewUpload;
use crate::permissions::{MEDIA_READ, MEDIA_WRITE};
use crate::service::{ItemChanges, MediaService, Visibility, content_url, too_large};

/// Lớn nhất `media.upload` nhận (base64 to hơn khoảng một phần ba; lời gọi còn phải lọt giới
/// hạn thân request chung `AUTOXIDE_HTTP_BODY_LIMIT_BYTES`).
pub(crate) const MAX_TOOL_BYTES: u64 = 1024 * 1024;

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
    /// Only items whose name contains this text, ignoring case and accents.
    q: Option<String>,
    /// Only items with this tag, ignoring case.
    tag: Option<String>,
    /// `true`: only images missing a text alternative (empty `alt`, not `decorative`).
    alt_missing: Option<bool>,
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
    /// `true` marks a purely decorative image, whose empty `alt` is on purpose.
    decorative: Option<bool>,
    /// Move to this folder; `null` moves to the root.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Uuid>, nullable)]
    folder_id: Option<Option<Uuid>>,
    /// `public` (anyone with the link may load the content) or `private`.
    visibility: Option<String>,
    /// Replace the tags of the item; `[]` removes them all.
    tags: Option<Vec<String>>,
}

/// The text alternative of a media item in one language.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SetAltInput {
    /// Id of the media item.
    id: Uuid,
    /// The `updated_at` of the item as you last read it. The change is refused if the
    /// item changed since; read it again and retry.
    updated_at: DateTime<Utc>,
    /// BCP 47 language code, one the server offers (e.g. `vi`).
    locale: String,
    /// Text alternative in this language, up to 1000 characters; `null` removes the
    /// translation (readers in this language get the default `alt` again).
    alt: Option<String>,
}

/// A small file to add to the media library.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UploadMediaInput {
    /// File name, 1 to 255 characters, without `/` or `\`.
    name: String,
    /// The content, base64-encoded (standard alphabet, with padding); at most 1 MiB once
    /// decoded. Upload larger files with `POST /api/v1/media`.
    content_base64: String,
    /// Media type, e.g. `image/png`. Omit to let the server detect it.
    content_type: Option<String>,
    /// Folder to put the item in. Omit for the root.
    folder_id: Option<Uuid>,
    /// The organization unit that owns the item; omit when you work in one unit only.
    org_id: Option<Uuid>,
    /// Tags for the item.
    tags: Option<Vec<String>>,
}

/// Which tags to list.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListTagsInput {
    /// Only tags whose name contains this text, ignoring case and accents.
    q: Option<String>,
    /// Maximum number of tags to return, 1 to 200. Defaults to 50.
    #[schema(minimum = 1, maximum = 200)]
    limit: Option<u32>,
    /// Where to continue: the `next_cursor` of the previous page.
    cursor: Option<String>,
}

pub(crate) fn all(service: &Arc<MediaService>) -> Vec<Tool> {
    // Mỗi tool giữ một bản `Arc` của service (clone `Arc` chỉ tăng bộ đếm tham chiếu).
    let (list, get, update, trash) =
        (service.clone(), service.clone(), service.clone(), service.clone());
    let (upload, list_tags, alt) = (service.clone(), service.clone(), service.clone());
    let set_alt = service.clone();
    vec![
        Tool::new(
            "list",
            "List media items (images, video, audio, documents), newest first. Filter by \
             folder, type, visibility, trash, name or missing alt text; sort by `created_at`, `name` or `size`. \
             Follow `pagination.next_cursor` with the same filters for more.",
            move |tenant: TenantContext, principal: Principal, input: ListMediaInput| {
                let service = list.clone();
                async move {
                    let query = ItemsQuery {
                        folder_id: input.folder_id,
                        owner_org: None,
                        content_type: input.content_type,
                        visibility: input.visibility,
                        trashed: input.trashed.map(|trashed| trashed.to_string()),
                        q: input.q,
                        tag: input.tag,
                        alt_missing: input.alt_missing.map(|missing| missing.to_string()),
                        include: None,
                        sort: input.sort,
                    };
                    let paging = PageQuery::new(input.limit.map(|limit| limit.to_string()), input.cursor, input.order);
                    let mut problems = Vec::new();
                    let filter = query.filter(&mut problems);
                    let page = crate::http::page_request(paging, problems)?;
                    let page = service.list(&tenant, &principal, &filter, &page).await?;
                    crate::http::page_of(&service, &tenant, page).await
                }
            },
        )
        .requires(MEDIA_READ)
        .read_only(),
        Tool::new(
            "get",
            "Get a media item: name, type, size, image width and height, texts, visibility, \
             and the `url` of its content.",
            move |tenant: TenantContext, principal: Principal, input: MediaId| {
                let service = get.clone();
                async move {
                    let item = service.get(&tenant, &principal, input.id).await?;
                    crate::http::describe(&service, &tenant, item).await
                }
            },
        )
        .requires(MEDIA_READ)
        .read_only(),
        Tool::new(
            "update",
            "Change the details of a media item: name, alt text, title, caption, decorative \
             flag, folder or visibility. Pass the `updated_at` you read; the change is refused if someone \
             changed the item since.",
            move |tenant: TenantContext, principal: Principal, input: UpdateMediaInput| {
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
                        decorative: input.decorative,
                        folder_id: input.folder_id,
                        visibility,
                        tags: input.tags,
                    };
                    let item = service
                        .update(&tenant, &principal, input.id, changes, Some(input.updated_at))
                        .await?;
                    crate::http::describe(&service, &tenant, item).await
                }
            },
        )
        .requires(MEDIA_WRITE),
        Tool::new(
            "set_alt",
            "Set (or, with `alt: null`, remove) the text alternative of a media item in one \
             language, e.g. `vi` on a bilingual site; the item's own `alt` stays the default for \
             other languages. Pass the `updated_at` you read; the change is refused if someone \
             changed the item since.",
            move |tenant: TenantContext, principal: Principal, input: SetAltInput| {
                let service = set_alt.clone();
                async move {
                    let item = service
                        .set_alt(
                            &tenant,
                            &principal,
                            input.id,
                            &input.locale,
                            input.alt.as_deref(),
                            Some(input.updated_at),
                        )
                        .await?;
                    crate::http::describe(&service, &tenant, item).await
                }
            },
        )
        .requires(MEDIA_WRITE),
        Tool::new(
            "trash",
            "Move a media item to the trash. Its public link stops working; it can be \
             restored until the server deletes it for good (30 days by default).",
            move |tenant: TenantContext, principal: Principal, input: MediaId| {
                let service = trash.clone();
                async move {
                    let item = service.trash(&tenant, &principal, input.id).await?;
                    crate::http::describe(&service, &tenant, item).await
                }
            },
        )
        .requires(MEDIA_WRITE),
        Tool::new(
            "upload",
            "Add a small file (at most 1 MiB) to the media library, sent as base64. The server \
             checks the content really is of its type and reads image sizes. Upload larger \
             files with `POST /api/v1/media`. The answer lists items that already had the same \
             content in `duplicates`.",
            move |tenant: TenantContext, principal: Principal, input: UploadMediaInput| {
                let service = upload.clone();
                async move {
                    let bytes = STANDARD.decode(input.content_base64.trim()).map_err(|_| {
                        Error::validation(vec![FieldError::new(
                            "content_base64",
                            "invalid",
                            "must be base64 (standard alphabet, with padding)",
                        )])
                    })?;
                    if bytes.len() as u64 > MAX_TOOL_BYTES {
                        return Err(too_large(MAX_TOOL_BYTES));
                    }
                    let upload = NewUpload {
                        name: input.name,
                        folder_id: input.folder_id,
                        org_id: input.org_id,
                        content_type: input.content_type,
                        tags: input.tags.unwrap_or_default(),
                    };
                    let body = futures_util::stream::iter([Ok(bytes::Bytes::from(bytes))]);
                    let uploaded =
                        service.upload(&tenant, &principal, upload, Box::pin(body)).await?;
                    let duplicates = uploaded
                        .duplicates
                        .into_iter()
                        .map(|item| DuplicateItem {
                            url: content_url(&item),
                            id: item.id,
                            name: item.name,
                            folder_id: crate::entity::api_folder(item.folder_id),
                        })
                        .collect();
                    let item = crate::http::describe(&service, &tenant, uploaded.item).await?;
                    Ok(UploadedResponse { item, duplicates })
                }
            },
        )
        .requires(MEDIA_WRITE),
        Tool::new(
            "list_tags",
            "List media tags by name, each with the number of items carrying it.",
            move |tenant: TenantContext, principal: Principal, input: ListTagsInput| {
                let service = list_tags.clone();
                async move {
                    let paging = PageQuery::new(input.limit.map(|limit| limit.to_string()), input.cursor, Some("asc".to_owned()));
                    let page = crate::http::page_request(paging, Vec::new())?;
                    let tags = service
                        .list_tags(&tenant, &principal, input.q.as_deref(), TagSort::Name, &page)
                        .await?;
                    Ok::<Page<TagResponse>, Error>(tags.map(TagResponse::from))
                }
            },
        )
        .requires(MEDIA_READ)
        .read_only(),
        Tool::new(
            "suggest_alt",
            "Ask the server's AI model to suggest alt text for an image (JPEG, PNG, GIF, WebP) in a \
             language, optionally with context such as the post title. Saves nothing: store what \
             you accept with `update`. Uses AI tokens.",
            move |tenant: TenantContext, principal: Principal, input: SuggestAltInput| {
                let service = alt.clone();
                async move {
                    let suggestion = service
                        .suggest_alt(
                            &tenant,
                            &principal,
                            input.id,
                            &input.locale,
                            input.context.as_deref(),
                        )
                        .await?;
                    Ok::<AltSuggestionResponse, Error>(suggestion.into())
                }
            },
        )
        .requires(MEDIA_WRITE),
    ]
}

/// An image to describe.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SuggestAltInput {
    /// Id of the media item.
    id: Uuid,
    /// Language of the alt text, a BCP 47 code like `vi` or `en`.
    locale: String,
    /// Where the image is used (for example the post title), up to 500 characters.
    context: Option<String>,
}
