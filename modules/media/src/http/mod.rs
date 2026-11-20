//! Endpoint REST của module `media`, dưới `/api/v1/media`. Handler chỉ lấy dữ liệu vào,
//! gọi service và trả kết quả; mọi logic (kể cả kiểm permission) nằm ở service. Mọi route
//! đăng ký qua `routes!` để có tài liệu OpenAPI (luật kiến trúc 6).

mod assist;
mod content;
mod folders;
mod items;
mod tags;
mod uploads;

use std::sync::Arc;

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::pagination::{PageQuery, PageRequest};
use autoxide_kernel::tenancy::TenantContext;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

use crate::entity::api_folder;
use crate::images::variants::VariantPair;
use crate::library::folders::Folder;
use crate::service::{FolderFilter, Item, ItemDetails, MediaService, Visibility, content_url};

pub(crate) use assist::AltSuggestionResponse;
pub(crate) use folders::FolderResponse;
pub(crate) use items::{DuplicateItem, ItemsQuery, UploadedResponse};
pub(crate) use tags::TagResponse;
pub(crate) use uploads::{StartMediaUploadRequest, present_uploaded};

pub(crate) fn routes(service: Arc<MediaService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(items::list_items, items::upload_item))
        .routes(routes!(items::library_summary))
        .routes(routes!(uploads::start_media_upload))
        .routes(routes!(uploads::complete_media_upload))
        .routes(routes!(folders::list_folders, folders::create_folder))
        .routes(routes!(folders::get_folder, folders::update_folder, folders::delete_folder))
        .routes(routes!(tags::list_tags))
        .routes(routes!(tags::get_tag, tags::rename_tag, tags::delete_tag))
        .routes(routes!(items::get_item, items::update_item, items::trash_item))
        .routes(routes!(items::restore_item))
        .routes(routes!(items::purge_item))
        .routes(routes!(assist::suggest_alt))
        .routes(routes!(items::set_alt, items::delete_alt))
        .routes(routes!(content::get_content, content::head_content, content::replace_content))
        .with_state(service)
}

/// A media item. Its content is at `url`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct MediaItemResponse {
    id: Uuid,
    /// The folder holding the item; `null` for the root.
    folder_id: Option<Uuid>,
    /// File name, given back on download.
    #[schema(example = "team-photo.jpg")]
    name: String,
    /// Real media type of the content (checked against its first bytes at upload).
    #[schema(example = "image/jpeg")]
    content_type: String,
    /// Size in bytes.
    #[schema(example = 482133)]
    size: u64,
    /// SHA-256 of the content, as 64 hex digits.
    sha256: String,
    /// Width in pixels; only for raster images (JPEG, PNG, GIF, WebP, AVIF).
    #[schema(example = 1920)]
    width: Option<i32>,
    /// Height in pixels; only for raster images.
    #[schema(example = 1080)]
    height: Option<i32>,
    /// Text alternative for screen readers (the `alt` of an `<img>`); empty when unset. This is
    /// the default language's text, used for any language without a translation in `alts`.
    alt: String,
    /// Translations of `alt` by language (BCP 47 code → text), set with
    /// `PUT /api/v1/media/{id}/alts/{locale}`. Show `alts[locale]` when present, else `alt`.
    #[schema(example = json!({"vi": "Bếp từ đôi màu đen trên mặt bếp gỗ"}))]
    alts: std::collections::BTreeMap<String, String>,
    /// Title; empty when unset.
    title: String,
    /// Caption; empty when unset.
    caption: String,
    /// `true` for a purely decorative image: its empty `alt` is on purpose (render `alt=""`), so
    /// it does not count as missing one.
    decorative: bool,
    /// The default `alt` was written from an AI suggestion: label it where it is shown (Vietnam's
    /// Law on Artificial Intelligence 134/2025/QH15).
    alt_ai_assisted: bool,
    visibility: Visibility,
    /// Version of the content; changes when the content is replaced.
    version_id: Uuid,
    /// Where to load the content, with its version: the address changes when the content
    /// does, so it may be cached for a long time. Public items load without signing in.
    #[schema(
        example = "/api/v1/media/0193a0c4-7d1e-7c2a-9b1e-3f2d5c6b7a81/content?v=0193a0c4-7d1e-7c2a-9b1e-3f2d5c6b7a82"
    )]
    url: String,
    /// The user who uploaded the item; `null` once that account is deleted.
    uploaded_by: Option<Uuid>,
    /// Display name of `uploaded_by`, with `include=uploader` on the list: `null` when the
    /// account no longer exists; absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    uploaded_by_name: Option<Option<String>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    /// When the item was moved to the trash; `null` when it is not in the trash.
    trashed_at: Option<DateTime<Utc>>,
    /// Tags of the item, sorted by name.
    #[schema(example = json!(["logo", "brand"]))]
    tags: Vec<String>,
    /// Smaller copies of a JPEG, PNG, GIF or WebP image, narrowest first, for `srcset`.
    /// Made in the background shortly after an upload or a content change: empty until then,
    /// and for other types.
    variants: Vec<VariantResponse>,
}

/// A smaller copy of an image.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub(crate) struct VariantResponse {
    #[schema(example = 640)]
    width: i32,
    #[schema(example = 360)]
    height: i32,
    /// `image/jpeg`, or `image/png` for images with transparency.
    #[schema(example = "image/jpeg")]
    content_type: String,
    /// Size in bytes.
    size: u64,
    /// Where to load it: the item's `url` with `&w=<width>`.
    url: String,
    /// The same copy as WebP, usually far smaller: `url` with `&format=webp`. Offer it in a
    /// `<picture>` `<source type="image/webp">`, or in `srcset` when every browser you serve
    /// reads WebP. `null` while it is not made yet (images uploaded before WebP copies existed
    /// get one after `autoxide task media.refresh_variants`).
    #[schema(
        example = "/api/v1/media/0190f5a2-7c4e-7d1a-9b3c-2f1e0d9c8b7a/content?v=0190f5a2-7c4e-7d1a-9b3c-2f1e0d9c8b7b&w=640&format=webp"
    )]
    webp_url: Option<String>,
    /// Size of the WebP copy in bytes; `null` like `webp_url`.
    webp_size: Option<u64>,
    /// The same copy as AVIF, usually a third smaller than WebP: `url` with `&format=avif`. Offer
    /// it in a `<source type="image/avif">` before the WebP one. Made in the background only when
    /// the server sets `AUTOXIDE_MEDIA_AVIF=true`; `null` until then.
    #[schema(
        example = "/api/v1/media/0190f5a2-7c4e-7d1a-9b3c-2f1e0d9c8b7a/content?v=0190f5a2-7c4e-7d1a-9b3c-2f1e0d9c8b7b&w=640&format=avif"
    )]
    avif_url: Option<String>,
    /// Size of the AVIF copy in bytes; `null` like `avif_url`.
    avif_size: Option<u64>,
}

impl VariantResponse {
    fn new(item_url: &str, copy: VariantPair) -> Self {
        let VariantPair { main, webp, avif } = copy;
        VariantResponse {
            avif_url: avif.as_ref().map(|avif| format!("{item_url}&w={}&format=avif", avif.width)),
            avif_size: avif.map(|avif| u64::try_from(avif.size).unwrap_or_default()),
            url: format!("{item_url}&w={}", main.width),
            webp_url: webp.as_ref().map(|webp| format!("{item_url}&w={}&format=webp", webp.width)),
            webp_size: webp.map(|webp| u64::try_from(webp.size).unwrap_or_default()),
            width: main.width,
            height: main.height,
            content_type: main.content_type,
            size: u64::try_from(main.size).unwrap_or_default(),
        }
    }
}

impl From<ItemDetails> for MediaItemResponse {
    fn from(details: ItemDetails) -> Self {
        let ItemDetails { item, tags, alts, variants } = details;
        let url = content_url(&item);
        MediaItemResponse {
            tags,
            alts,
            variants: variants
                .into_iter()
                .map(|variant| VariantResponse::new(&url, variant))
                .collect(),
            url,
            visibility: Visibility::of(&item),
            id: item.id,
            folder_id: api_folder(item.folder_id),
            name: item.name,
            content_type: item.content_type,
            size: u64::try_from(item.size).unwrap_or_default(),
            sha256: item.sha256,
            width: item.width,
            height: item.height,
            alt: item.alt,
            title: item.title,
            caption: item.caption,
            decorative: item.decorative,
            alt_ai_assisted: item.alt_ai_assisted,
            version_id: item.version_id,
            uploaded_by: item.uploaded_by,
            uploaded_by_name: None,
            created_at: item.created_at,
            updated_at: item.updated_at,
            trashed_at: item.trashed_at,
        }
    }
}

impl From<Folder> for FolderResponse {
    fn from(folder: Folder) -> Self {
        FolderResponse {
            id: folder.id,
            parent_id: api_folder(folder.parent_id),
            name: folder.name,
            item_count: 0,
            created_at: folder.created_at,
            updated_at: folder.updated_at,
        }
    }
}

/// Item như API trả về (kèm nhãn và ảnh thu nhỏ).
pub(crate) async fn describe(
    service: &MediaService,
    tenant: &TenantContext,
    item: Item,
) -> Result<MediaItemResponse, Error> {
    Ok(service.describe_one(tenant, item).await?.into())
}

/// Item kèm `ETag` (phiên bản của bản ghi).
pub(crate) async fn tagged(
    service: &MediaService,
    tenant: &TenantContext,
    item: Item,
) -> Result<autoxide_kernel::version::Tagged<MediaItemResponse>, Error> {
    let version = item.updated_at;
    Ok(autoxide_kernel::version::Tagged { version, data: describe(service, tenant, item).await? })
}

/// Một trang item như API trả về; nhãn và ảnh thu nhỏ của cả trang đọc chung một lần.
pub(crate) async fn page_of(
    service: &MediaService,
    tenant: &TenantContext,
    page: autoxide_kernel::pagination::Page<Item>,
) -> Result<autoxide_kernel::pagination::Page<MediaItemResponse>, Error> {
    let autoxide_kernel::pagination::Page { data, pagination } = page;
    let data =
        service.describe(tenant, data).await?.into_iter().map(MediaItemResponse::from).collect();
    Ok(autoxide_kernel::pagination::Page { data, pagination })
}

/// Tham số `folder_id` / `parent_id` của danh sách: `root` hay một UUID.
fn folder_filter(raw: Option<&str>, field: &str, problems: &mut Vec<FieldError>) -> FolderFilter {
    match raw.map(str::trim) {
        None | Some("") => FolderFilter::Any,
        Some("root") => FolderFilter::Root,
        Some(raw) => Uuid::parse_str(raw).map(FolderFilter::Id).unwrap_or_else(|_| {
            problems.push(FieldError::new(field, "invalid", "must be `root` or a folder id"));
            FolderFilter::Any
        }),
    }
}

/// Yêu cầu trang, gộp lỗi của nó với lỗi của bộ lọc để báo một lần.
pub(crate) fn page_request(
    query: PageQuery,
    mut problems: Vec<FieldError>,
) -> Result<PageRequest, Error> {
    let page = PageRequest::try_from(query);
    if let Err(error) = &page {
        problems.extend(error.fields().iter().cloned());
    }
    match page {
        Ok(page) if problems.is_empty() => Ok(page),
        _ => Err(Error::validation(problems)),
    }
}

/// Tham số `sort`: một giá trị của `T` (`snake_case`).
fn sort<T: for<'de> Deserialize<'de> + Default>(
    raw: Option<&str>,
    allowed: &str,
    problems: &mut Vec<FieldError>,
) -> T {
    match raw.map(str::trim) {
        None | Some("") => T::default(),
        Some(raw) => serde_json::from_value(serde_json::Value::String(raw.to_owned()))
            .unwrap_or_else(|_| {
                problems.push(
                    FieldError::new("sort", "invalid", "must be one of {allowed}")
                        .with_param("allowed", allowed),
                );
                T::default()
            }),
    }
}

// Trường JSON có ba trạng thái: không gửi (`None`), `null` (`Some(None)`), có giá trị.
// `#[serde(default)]` lo trường hợp không gửi; hàm này lo hai trường hợp còn lại.
pub(crate) fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[cfg(test)]
mod tests;

impl MediaItemResponse {
    /// Người tải lên (để tra tên theo lô).
    pub(crate) fn uploader(&self) -> Option<Uuid> {
        self.uploaded_by
    }

    /// Kèm tên người tải lên (`include=uploader`): `null` khi tài khoản không còn.
    pub(crate) fn with_uploader_name(
        self,
        names: &std::collections::HashMap<Uuid, String>,
    ) -> Self {
        let uploaded_by_name = self.uploaded_by.map(|id| names.get(&id).cloned());
        MediaItemResponse { uploaded_by_name, ..self }
    }
}
