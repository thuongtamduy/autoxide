//! Hàng số: file của biến thể (`/variants/{id}/digital-assets`, `/digital-assets/{id}`), quyền tải
//! của đơn cho người quản lý (`/orders/{id}/downloads`), quyền tải của người mua đang đăng nhập
//! (`/downloads`) và tải công khai bằng token (`/public/downloads/…`).

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery, PageRequest};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;
use validator::Validate;

use crate::digital::{FileLink, GrantView, NewAsset, token_path};
use crate::entity::digital_asset;
use crate::service::CatalogService;

pub(crate) fn routes(service: Arc<CatalogService>) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_assets, attach_asset))
        .routes(routes!(remove_asset))
        .routes(routes!(list_order_downloads))
        .routes(routes!(revoke_order_downloads))
        .routes(routes!(reissue_order_downloads))
        .routes(routes!(list_my_downloads))
        .routes(routes!(use_my_download))
        .routes(routes!(use_download_token))
        .routes(routes!(download_file))
        .with_state(service)
}

/// A file of a digital product variant.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = CatalogDigitalAsset)]
pub(crate) struct AssetResponse {
    pub id: Uuid,
    pub product_id: Uuid,
    pub variant_id: Uuid,
    /// Name the buyer's download is saved as.
    pub file_name: String,
    pub content_type: String,
    /// Size in bytes.
    pub size: i64,
    /// SHA-256 of the content, lower-case hex.
    pub sha256: String,
    /// Downloads each buyer gets for each order line.
    pub download_limit: i64,
    /// Days a buyer can download after paying.
    pub grant_days: i64,
    pub created_at: Timestamp,
}

impl From<digital_asset::Model> for AssetResponse {
    fn from(row: digital_asset::Model) -> Self {
        AssetResponse {
            id: row.id,
            product_id: row.product_id,
            variant_id: row.variant_id,
            file_name: row.file_name,
            content_type: row.content_type,
            size: row.size,
            sha256: row.sha256,
            download_limit: row.download_limit,
            grant_days: row.grant_days,
            created_at: row.created_at,
        }
    }
}

/// Attach a file to a digital variant.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[schema(as = CatalogAttachDigitalAsset)]
pub(crate) struct AttachAssetRequest {
    /// A file you uploaded with `POST /api/v1/files` (or `/api/v1/files/uploads`). Its content is
    /// copied: deleting the uploaded file later does not affect buyers.
    pub file_id: Uuid,
    /// Downloads each buyer gets, 1 to 1000; omitted: `AUTOXIDE_CATALOG_DEFAULT_DOWNLOAD_LIMIT`.
    pub download_limit: Option<i64>,
    /// Days a buyer can download after paying, 1 to 3650; omitted:
    /// `AUTOXIDE_CATALOG_DEFAULT_GRANT_DAYS`.
    pub grant_days: Option<i64>,
}

/// The right of a buyer to download one file of one order line.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = CatalogDownloadGrant)]
pub(crate) struct GrantResponse {
    pub id: Uuid,
    pub order_id: Uuid,
    pub order_line_id: Uuid,
    pub asset_id: Uuid,
    pub variant_id: Uuid,
    pub file_name: String,
    /// Downloads left.
    pub remaining: i64,
    pub expires_at: Timestamp,
    /// Set when the right was revoked (order cancelled or refunded, or by staff).
    pub revoked_at: Option<Timestamp>,
    pub created_at: Timestamp,
    /// Only right after the token was issued: the path to `POST` to spend one download, without
    /// signing in. It cannot be read again later; send it to the buyer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_path: Option<String>,
}

impl From<GrantView> for GrantResponse {
    fn from(view: GrantView) -> Self {
        let GrantView { grant, file_name, token } = view;
        GrantResponse {
            id: grant.id,
            order_id: grant.order_id,
            order_line_id: grant.order_line_id,
            asset_id: grant.asset_id,
            variant_id: grant.variant_id,
            file_name,
            remaining: grant.remaining,
            expires_at: grant.expires_at,
            revoked_at: grant.revoked_at,
            created_at: grant.created_at,
            token_path: token.as_deref().map(token_path),
        }
    }
}

/// A short-lived address of the file, after one download was spent.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = CatalogDownloadLink)]
pub(crate) struct LinkResponse {
    /// Path (under the same host) to `GET` the file; works without signing in until `expires_at`,
    /// accepts `Range`, and does not spend more downloads.
    pub url: String,
    pub expires_at: Timestamp,
    /// Downloads left after this one.
    pub remaining: i64,
    pub file_name: String,
}

impl From<FileLink> for LinkResponse {
    fn from(link: FileLink) -> Self {
        LinkResponse {
            url: link.url,
            expires_at: link.expires_at,
            remaining: link.remaining,
            file_name: link.file_name,
        }
    }
}

/// How many rights were revoked.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = CatalogDownloadsRevoked)]
pub(crate) struct RevokedResponse {
    pub revoked: u64,
}

/// List the files of a digital variant.
///
/// Requires `catalog.products.read` on the unit owning the product.
#[utoipa::path(
    get,
    path = "/variants/{id}/digital-assets",
    extensions(("x-permissions" = json!(["catalog.products.read"]))),
    operation_id = "list_catalog_digital_assets",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the variant.")),
    responses(
        (status = 200, description = "The files, oldest first.", body = Data<Vec<AssetResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.read`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No variant with this id in your scope; code `catalog.variant_not_found`.", body = Problem),
    )
)]
async fn list_assets(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<Vec<AssetResponse>>, Error> {
    let rows = service.list_assets(&tenant, &principal, id).await?;
    Ok(Data { data: rows.into_iter().map(AssetResponse::from).collect() })
}

/// Attach a file to a variant of a digital product.
///
/// Requires `catalog.products.write` on the unit owning the product, plan feature
/// `catalog.digital`, and the right to see the uploaded file (you uploaded it, or you have
/// `files.manage`). Upload the file first with `POST /api/v1/files`; its content is copied, so the
/// product keeps working if the uploaded file is deleted. A digital product needs at least one
/// file before it can be put on sale.
#[utoipa::path(
    post,
    path = "/variants/{id}/digital-assets",
    extensions(("x-permissions" = json!(["catalog.products.write"]))),
    operation_id = "attach_catalog_digital_asset",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the variant.")),
    request_body = AttachAssetRequest,
    responses(
        (status = 201, description = "Attached.", body = Data<AssetResponse>, headers(("Location" = String, description = "Path of the variant's files."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.write` (code `permission_denied`), or the plan lacks `catalog.digital` (code `feature_not_enabled`).", body = Problem),
        (status = 404, description = "No variant with this id in your scope (code `catalog.variant_not_found`), or no file `file_id` you may see (code `files.file_not_found`).", body = Problem),
        (status = 422, description = "The product is not of kind `digital` (code `catalog.not_digital`), or a value is out of range; see `errors`.", body = Problem),
        (status = 503, description = "Module `files` is not installed; code `catalog.files_unavailable` or `catalog.storage_unavailable`.", body = Problem),
    )
)]
async fn attach_asset(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    ValidJson(input): ValidJson<AttachAssetRequest>,
) -> Result<Created<AssetResponse>, Error> {
    let input = NewAsset {
        file_id: input.file_id,
        download_limit: input.download_limit,
        grant_days: input.grant_days,
    };
    let row = service.attach_asset(&tenant, &principal, id, input).await?;
    Ok(Created::new(format!("/catalog/variants/{id}/digital-assets"), AssetResponse::from(row)))
}

/// Remove a file from a digital variant.
///
/// Requires `catalog.products.write` on the unit owning the product. Refused while buyers can
/// still download it, and for the last file of a product on sale.
#[utoipa::path(
    delete,
    path = "/digital-assets/{id}",
    extensions(("x-permissions" = json!(["catalog.products.write"]))),
    operation_id = "remove_catalog_digital_asset",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the file.")),
    responses(
        (status = 204, description = "Removed."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.products.write`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No such file in your scope; code `catalog.digital_asset_not_found`.", body = Problem),
        (status = 409, description = "Buyers can still download it (code `catalog.digital_asset_in_use`: revoke or wait), or it is the last file of a product on sale (code `catalog.digital_asset_missing`).", body = Problem),
    )
)]
async fn remove_asset(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.remove_asset(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// List the download rights of an order.
///
/// Requires `catalog.downloads.manage` on the unit owning the products; only the rights you
/// manage are listed. Tokens are never shown here: use the reissue endpoint to send new ones.
#[utoipa::path(
    get,
    path = "/orders/{id}/downloads",
    extensions(("x-permissions" = json!(["catalog.downloads.manage"]))),
    operation_id = "list_catalog_order_downloads",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the order (module `orders`).")),
    responses(
        (status = 200, description = "The rights.", body = Data<Vec<GrantResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.downloads.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "The order has no download right you manage; code `catalog.download_not_found`.", body = Problem),
    )
)]
async fn list_order_downloads(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<Vec<GrantResponse>>, Error> {
    let rows = service.order_grants(&tenant, &principal, id).await?;
    Ok(Data { data: rows.into_iter().map(GrantResponse::from).collect() })
}

/// Revoke the download rights of an order.
///
/// Requires `catalog.downloads.manage` on the unit owning the products. Tokens and file
/// addresses already handed out stop working at once. Cancelling or refunding the order in module
/// `orders` does this by itself.
#[utoipa::path(
    post,
    path = "/orders/{id}/downloads/revoke",
    extensions(("x-permissions" = json!(["catalog.downloads.manage"]))),
    operation_id = "revoke_catalog_order_downloads",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the order (module `orders`).")),
    responses(
        (status = 200, description = "Revoked; `revoked` is 0 when they already were.", body = Data<RevokedResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.downloads.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "The order has no download right you manage; code `catalog.download_not_found`.", body = Problem),
    )
)]
async fn revoke_order_downloads(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<RevokedResponse>, Error> {
    let revoked = service.revoke_order_grants(&tenant, &principal, id).await?;
    Ok(Data { data: RevokedResponse { revoked } })
}

/// Give new download tokens for an order.
///
/// Requires `catalog.downloads.manage` on the unit owning the products. For rights still usable
/// (not revoked, not expired): a new token replaces the old one, downloads left and expiry stay.
/// The answer is the only time the new `token_path` is shown — send it to the buyer (e.g. a
/// buyer without an account who lost the link).
#[utoipa::path(
    post,
    path = "/orders/{id}/downloads/reissue",
    extensions(("x-permissions" = json!(["catalog.downloads.manage"]))),
    operation_id = "reissue_catalog_order_downloads",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the order (module `orders`).")),
    responses(
        (status = 200, description = "The rights with their new `token_path`.", body = Data<Vec<GrantResponse>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing `catalog.downloads.manage`; code `permission_denied`.", body = Problem),
        (status = 404, description = "The order has no download right you manage; code `catalog.download_not_found`.", body = Problem),
    )
)]
async fn reissue_order_downloads(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<Vec<GrantResponse>>, Error> {
    let rows = service.reissue_order_grants(&tenant, &principal, id).await?;
    Ok(Data { data: rows.into_iter().map(GrantResponse::from).collect() })
}

/// List your download rights.
///
/// Any signed-in buyer; only the rights of your own paid orders, newest first.
#[utoipa::path(
    get,
    path = "/downloads",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_my_catalog_downloads",
    security(("bearer" = [])),
    params(PageQuery),
    responses(
        (status = 200, description = "One page of rights.", body = Page<GrantResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 422, description = "A paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
async fn list_my_downloads(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(page): Query<PageQuery>,
) -> Result<Page<GrantResponse>, Error> {
    let page = PageRequest::try_from(page)?;
    Ok(service.own_grants(&tenant, &principal, &page).await?.map(GrantResponse::from))
}

/// Spend one download of one of your rights.
///
/// Any signed-in buyer, for a right of their own order. Answers a short-lived address of the file.
#[utoipa::path(
    post,
    path = "/downloads/{id}",
    extensions(("x-permissions" = json!([]))),
    operation_id = "use_my_catalog_download",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the download right.")),
    responses(
        (status = 200, description = "One download spent; `url` serves the file.", body = Data<LinkResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 404, description = "No usable right of yours with this id (expired or revoked); code `catalog.download_not_found`.", body = Problem),
        (status = 409, description = "Every download was used; code `catalog.download_limit_reached`.", body = Problem),
    )
)]
async fn use_my_download(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Data<LinkResponse>, Error> {
    let link = service.use_own_grant(&tenant, &principal, id).await?;
    Ok(Data { data: link.into() })
}

/// Spend one download with a token (public).
///
/// No sign-in needed: the token of the download link the buyer received is the proof. Answers a
/// short-lived address of the file (`AUTOXIDE_CATALOG_DOWNLOAD_LINK_SECONDS`).
#[utoipa::path(
    post,
    path = "/public/downloads/{token}",
    extensions(("x-public" = json!(true))),
    operation_id = "use_catalog_download_token",
    security(()),
    params(("token" = String, Path, description = "Token of the download link.")),
    responses(
        (status = 200, description = "One download spent; `url` serves the file.", body = Data<LinkResponse>),
        (status = 404, description = "The token is wrong, expired or revoked; code `catalog.download_not_found`.", body = Problem),
        (status = 409, description = "Every download was used; code `catalog.download_limit_reached`.", body = Problem),
    )
)]
async fn use_download_token(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    Path(token): Path<String>,
) -> Result<Data<LinkResponse>, Error> {
    let link = service.use_token(&tenant, &token).await?;
    Ok(Data { data: link.into() })
}

/// Signature of a file address.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct SignedQuery {
    /// Expiry, Unix seconds.
    pub exp: i64,
    /// Signature.
    pub sig: String,
}

/// Download the file of a signed address (public).
///
/// No sign-in needed; the address comes from spending a download and is valid until `exp`. Send
/// `Range: bytes=…` to resume (one range). Does not spend a download.
#[utoipa::path(
    get,
    path = "/public/downloads/{id}/file",
    extensions(("x-public" = json!(true))),
    operation_id = "download_catalog_digital_file",
    security(()),
    params(("id" = Uuid, Path, description = "Id of the download right."), SignedQuery),
    responses(
        (status = 200, description = "The whole file.", content_type = "application/octet-stream", headers(("Content-Disposition" = String, description = "`attachment` with the file name."))),
        (status = 206, description = "The range asked for.", content_type = "application/octet-stream", headers(("Content-Range" = String, description = "`bytes start-end/size`."))),
        (status = 404, description = "The address is wrong or expired, or the right was revoked; code `catalog.download_not_found`.", body = Problem),
        (status = 416, description = "The range is outside the file; code `catalog.range_not_satisfiable`.", body = Problem),
    )
)]
async fn download_file(
    State(service): State<Arc<CatalogService>>,
    tenant: TenantContext,
    Path(id): Path<Uuid>,
    Query(query): Query<SignedQuery>,
    headers: HeaderMap,
) -> Result<Response, Error> {
    let range = headers.get(header::RANGE).and_then(|value| value.to_str().ok());
    let served = service.signed_file(&tenant, id, query.exp, &query.sig, range).await?;
    let total = served.asset.size;
    let status = if served.range.is_some() { StatusCode::PARTIAL_CONTENT } else { StatusCode::OK };
    let mut response = (status, Body::from_stream(served.file.content)).into_response();
    let headers = response.headers_mut();
    let content_type = HeaderValue::from_str(&served.asset.content_type)
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    headers.insert(header::CONTENT_TYPE, content_type);
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(served.file.size));
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    if let Some(range) = &served.range
        && let Ok(value) =
            HeaderValue::from_str(&format!("bytes {}-{}/{total}", range.start, range.end - 1))
    {
        headers.insert(header::CONTENT_RANGE, value);
    }
    headers.insert(header::CONTENT_DISPOSITION, disposition(&served.asset.file_name));
    Ok(response)
}

/// `Content-Disposition: attachment` với tên file: bản ASCII thay ký tự lạ bằng `_`, bản UTF-8
/// theo RFC 5987 (`filename*`) cho tên có dấu.
fn disposition(name: &str) -> HeaderValue {
    let ascii: String = name
        .chars()
        .map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' { c } else { '_' })
        .collect();
    let encoded: String = name
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b".-_~".contains(&byte) {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect();
    HeaderValue::from_str(&format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}"))
        .unwrap_or_else(|_| HeaderValue::from_static("attachment"))
}
