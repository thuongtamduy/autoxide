//! Endpoint quản lý banner: `/api/v1/content/banners` (bước 55, D-236). Cần `content.banners` ở nút
//! của banner.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::http::ValidJson;
use autoxide_kernel::pagination::{Page, PageQuery};
use autoxide_kernel::response::{Created, Data, Problem};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, Tagged};
use autoxide_spike_macros::Schema as ToSchema;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::IntoParams;
use uuid::Uuid;
use validator::Validate;

use super::{page_request, present};
use crate::banners::items::{
    Banner, BannerFields, BannerFilter, BannerPatch, Link, NewBanner, PageTarget, Via, link_of,
    page_targets,
};
use crate::banners::strings;
use crate::service::ContentService;

/// Where a banner links to: give `url`, or `kind` and `id`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BannerLink {
    /// An `https://` or `http://` address, or a path on the website starting with `/`.
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    /// A type of the object directory, e.g. `content.entry`, `content.term`,
    /// `catalog.product`, `catalog.collection`, `promotions.promotion` — whatever the installed
    /// modules provide (`GET /api/v1/kernel/resources` lists them). A banner pointing to a
    /// promotion is archived when the promotion ends.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "catalog.product")]
    kind: Option<String>,
    /// Id of that object.
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
}

impl BannerLink {
    fn into_link(self, problems: &mut Vec<FieldError>) -> Link {
        match (self.url, self.kind, self.id) {
            (Some(url), None, None) => Link::Url(url),
            (None, Some(kind), Some(id)) => Link::Ref { kind, id },
            _ => {
                problems.push(FieldError::new(
                    "link",
                    "invalid",
                    "give either `url`, or both `kind` and `id`",
                ));
                Link::None
            }
        }
    }

    fn of(link: Link) -> Option<Self> {
        match link {
            Link::None => None,
            Link::Url(url) => Some(BannerLink { url: Some(url), kind: None, id: None }),
            Link::Ref { kind, id } => {
                Some(BannerLink { url: None, kind: Some(kind), id: Some(id) })
            }
        }
    }
}

/// A page the banner shows on: an object of the object directory (e.g. a product category).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BannerPage {
    #[schema(example = "catalog.category")]
    kind: String,
    id: String,
}

/// A banner of a display placement.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub(crate) struct BannerResponse {
    id: Uuid,
    /// Key of its placement.
    #[schema(example = "home_hero")]
    placement: String,
    /// Organization unit it belongs to; the tenant's id means every store of the tenant.
    org_id: Uuid,
    locale: String,
    /// `image` or `text`.
    kind: String,
    title: String,
    subtitle: String,
    /// Text of the button.
    cta_label: String,
    /// Text banners: a colour token of the theme (`primary`, `secondary`, `accent`, `neutral`).
    tone: Option<String>,
    /// Picture (media item id).
    media_id: Option<Uuid>,
    /// Picture for phones; `null`: the same picture.
    mobile_media_id: Option<Uuid>,
    alt: String,
    /// Only on these kinds of pages; empty: every kind of its placement.
    page_kinds: Vec<String>,
    /// Only on these pages; empty: every page.
    page_targets: Vec<BannerPage>,
    link: Option<BannerLink>,
    /// Shown from (included); `null`: at once.
    starts_at: Option<Timestamp>,
    /// Shown until (excluded); `null`: no end.
    ends_at: Option<Timestamp>,
    /// Order in the placement, smallest first.
    position: i32,
    /// `draft`, `active` or `archived`.
    status: String,
    /// The text was written from an AI suggestion (Vietnam's Law on Artificial Intelligence
    /// 134/2025/QH15): label it.
    ai_assisted: bool,
    created_by: Option<Uuid>,
    created_at: Timestamp,
    updated_at: Timestamp,
}

pub(crate) fn banner_response(row: Banner, placement: &str) -> BannerResponse {
    BannerResponse {
        link: BannerLink::of(link_of(&row)),
        page_kinds: strings(&row.page_kinds),
        page_targets: page_targets(&row.page_targets)
            .into_iter()
            .map(|page| BannerPage { kind: page.kind, id: page.id })
            .collect(),
        id: row.id,
        placement: placement.to_owned(),
        org_id: row.org_id,
        locale: row.locale,
        kind: row.kind,
        title: row.title,
        subtitle: row.subtitle,
        cta_label: row.cta_label,
        tone: row.tone,
        media_id: row.media_id,
        mobile_media_id: row.mobile_media_id,
        alt: row.alt,
        starts_at: row.starts_at,
        ends_at: row.ends_at,
        position: row.position,
        status: row.status,
        ai_assisted: row.ai_assisted,
        created_by: row.created_by,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

/// A new banner.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateBannerRequest {
    /// Key of the placement, e.g. `home_hero`.
    placement: String,
    /// Organization unit (store) it belongs to; omitted: every store of the tenant.
    org_id: Option<Uuid>,
    /// Omitted: the default language.
    locale: Option<String>,
    /// `image` (default) or `text`; the placement must take it.
    kind: Option<String>,
    /// At most 120 characters; required for a text banner.
    #[serde(default)]
    title: String,
    /// At most 300 characters.
    #[serde(default)]
    subtitle: String,
    /// Text of the button, at most 60 characters.
    #[serde(default)]
    cta_label: String,
    /// Text banners only: `primary`, `secondary`, `accent` or `neutral` (default) — never a
    /// colour code, so a new theme recolours it.
    tone: Option<String>,
    /// Image banners: a public picture with the placement's ratio and size.
    media_id: Option<Uuid>,
    /// Optional picture for phones (the placement's `mobile_ratio`).
    mobile_media_id: Option<Uuid>,
    /// At most 300 characters; empty: the picture's own `alt`.
    #[serde(default)]
    alt: String,
    /// Only on these kinds of pages (within those of the placement).
    #[serde(default)]
    page_kinds: Vec<String>,
    /// Only on these pages, at most 50; each must exist.
    #[serde(default)]
    page_targets: Vec<BannerPage>,
    link: Option<BannerLink>,
    starts_at: Option<Timestamp>,
    ends_at: Option<Timestamp>,
    /// `draft` (default) or `active`.
    status: Option<String>,
    /// Set when the text came from an AI suggestion.
    #[serde(default)]
    ai_assisted: bool,
}

/// Changes to a banner: fields left out stay; `null` clears a field that may be empty.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateBannerRequest {
    kind: Option<String>,
    title: Option<String>,
    subtitle: Option<String>,
    cta_label: Option<String>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    tone: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Uuid>)]
    media_id: Option<Option<Uuid>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Uuid>)]
    mobile_media_id: Option<Option<Uuid>>,
    alt: Option<String>,
    page_kinds: Option<Vec<String>>,
    page_targets: Option<Vec<BannerPage>>,
    /// `null` removes the link.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<BannerLink>)]
    link: Option<Option<BannerLink>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Timestamp>)]
    starts_at: Option<Option<Timestamp>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Timestamp>)]
    ends_at: Option<Option<Timestamp>>,
    /// `draft`, `active` or `archived`.
    status: Option<String>,
    ai_assisted: Option<bool>,
}

fn pages(pages: Vec<BannerPage>) -> Vec<PageTarget> {
    pages.into_iter().map(|page| PageTarget { kind: page.kind, id: page.id }).collect()
}

impl CreateBannerRequest {
    pub(crate) fn into_new(self) -> Result<NewBanner, Error> {
        let mut problems = Vec::new();
        let link = self.link.map_or(Link::None, |link| link.into_link(&mut problems));
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        Ok(NewBanner {
            placement: self.placement,
            org_id: self.org_id,
            fields: BannerFields {
                locale: self.locale,
                kind: self.kind,
                title: self.title,
                subtitle: self.subtitle,
                cta_label: self.cta_label,
                tone: self.tone,
                media_id: self.media_id,
                mobile_media_id: self.mobile_media_id,
                alt: self.alt,
                page_kinds: self.page_kinds,
                page_targets: pages(self.page_targets),
                link,
                starts_at: self.starts_at,
                ends_at: self.ends_at,
                status: self.status,
                ai_assisted: self.ai_assisted,
            },
        })
    }
}

impl UpdateBannerRequest {
    pub(crate) fn into_patch(self) -> Result<BannerPatch, Error> {
        let mut problems = Vec::new();
        let link =
            self.link.map(|link| link.map_or(Link::None, |link| link.into_link(&mut problems)));
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        Ok(BannerPatch {
            kind: self.kind,
            title: self.title,
            subtitle: self.subtitle,
            cta_label: self.cta_label,
            tone: self.tone,
            media_id: self.media_id,
            mobile_media_id: self.mobile_media_id,
            alt: self.alt,
            page_kinds: self.page_kinds,
            page_targets: self.page_targets.map(pages),
            link,
            starts_at: self.starts_at,
            ends_at: self.ends_at,
            status: self.status,
            ai_assisted: self.ai_assisted,
        })
    }
}

/// Which banners to list.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct BannersQuery {
    /// Only this placement (key).
    placement: Option<String>,
    /// Only this organization unit.
    org_id: Option<Uuid>,
    /// Only `draft`, `active` or `archived`.
    status: Option<String>,
    /// Only this language.
    locale: Option<String>,
}

/// List banners.
///
/// Requires `content.banners` (or `content.read`); only banners of units within reach are listed.
#[utoipa::path(
    get,
    path = "/banners",
    extensions(("x-permissions" = json!(["content.banners", "content.read"]))),
    operation_id = "list_content_banners",
    security(("bearer" = [])),
    params(BannersQuery, PageQuery),
    responses(
        (status = 200, description = "One page of banners, by id.", body = Page<BannerResponse>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.banners`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No placement has the key in `placement`; code `content.placement_not_found`.", body = Problem),
        (status = 422, description = "A filter or paging parameter is invalid; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn list_banners(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Query(query): Query<BannersQuery>,
    Query(paging): Query<PageQuery>,
) -> Result<Page<BannerResponse>, Error> {
    let page = page_request(paging, Vec::new())?;
    let filter = BannerFilter {
        placement: query.placement,
        org_id: query.org_id,
        status: query.status,
        locale: query.locale,
    };
    let found = service.list_banners(&tenant, &principal, filter, &page).await?;
    let keys = service.placement_keys(&tenant).await?;
    Ok(found.map(|row| {
        let key = keys.get(&row.placement_id).cloned().unwrap_or_default();
        banner_response(row, &key)
    }))
}

/// Create a banner.
///
/// Requires `content.banners` for its organization unit. It goes last in its placement. Retry
/// safely with an `Idempotency-Key` header.
#[utoipa::path(
    post,
    path = "/banners",
    extensions(("x-permissions" = json!(["content.banners"]))),
    operation_id = "create_content_banner",
    security(("bearer" = [])),
    request_body = CreateBannerRequest,
    responses(
        (status = 201, description = "Created.", body = Data<BannerResponse>, headers(("ETag" = String, description = "Version of the banner."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.banners` for the unit; code `permission_denied` or `org_out_of_scope`.", body = Problem),
        (status = 404, description = "No placement has this key; code `content.placement_not_found`.", body = Problem),
        (status = 409, description = "The placement is full for this unit and language; code `content.placement_full`.", body = Problem),
        (status = 422, description = "A value is invalid. When every problem is of one kind the code says which: `content.banner_kind_not_allowed`, `content.banner_image_size_invalid`, `content.media_not_found`, `content.media_not_public`, `content.page_kind_not_allowed`, `content.page_target_not_found`, `content.link_kind_unavailable`, `content.link_target_missing`, `content.banner_window_invalid`; see `errors`.", body = Problem),
    )
)]
pub(crate) async fn create_banner(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    ValidJson(input): ValidJson<CreateBannerRequest>,
) -> Result<Created<BannerResponse>, Error> {
    let (row, placement) =
        service.create_banner(&tenant, &principal, input.into_new()?, Via::Rest).await?;
    let version = row.updated_at;
    Ok(Created::new(format!("/content/banners/{}", row.id), banner_response(row, &placement.key))
        .tagged(version))
}

/// Get a banner.
///
/// Requires `content.banners` for its organization unit.
#[utoipa::path(
    get,
    path = "/banners/{id}",
    extensions(("x-permissions" = json!(["content.banners"]))),
    operation_id = "get_content_banner",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the banner.")),
    responses(
        (status = 200, description = "The banner.", body = Data<BannerResponse>, headers(("ETag" = String, description = "Version of the banner: send it in `If-Match` to change it."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.banners`; code `permission_denied`.", body = Problem),
        (status = 404, description = "No banner has this id within reach; code `content.banner_not_found`.", body = Problem),
    )
)]
pub(crate) async fn get_banner(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<Tagged<BannerResponse>, Error> {
    let (row, placement) = service.get_banner(&tenant, &principal, id).await?;
    Ok(Tagged { version: row.updated_at, data: banner_response(row, &placement.key) })
}

/// Change a banner.
///
/// Requires `content.banners` for its organization unit. Fields left out stay as they are.
/// `If-Match` is required. Set `status` to `active` to show it, `archived` to retire it.
#[utoipa::path(
    patch,
    path = "/banners/{id}",
    extensions(("x-permissions" = json!(["content.banners"]))),
    operation_id = "update_content_banner",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the banner."), ("If-Match" = String, Header, description = "Required: the `ETag` you read (or `*`).")),
    request_body = UpdateBannerRequest,
    responses(
        (status = 200, description = "The banner after the change.", body = Data<BannerResponse>, headers(("ETag" = String, description = "New version of the banner."))),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.banners` for the unit; code `permission_denied`.", body = Problem),
        (status = 404, description = "No banner has this id within reach; code `content.banner_not_found`.", body = Problem),
        (status = 409, description = "Bringing it back would overfill the placement; code `content.placement_full`.", body = Problem),
        (status = 412, description = "The banner changed since you read it; code `precondition_failed`.", body = Problem),
        (status = 422, description = "A value is invalid; same codes as creating; see `errors`.", body = Problem),
        (status = 428, description = "`If-Match` is missing; code `precondition_required`.", body = Problem),
    )
)]
pub(crate) async fn update_banner(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
    if_match: IfMatch,
    ValidJson(input): ValidJson<UpdateBannerRequest>,
) -> Result<Tagged<BannerResponse>, Error> {
    let expected = if_match.required()?;
    let (row, placement) = service
        .update_banner(&tenant, &principal, id, input.into_patch()?, expected, Via::Rest)
        .await?;
    Ok(Tagged { version: row.updated_at, data: banner_response(row, &placement.key) })
}

/// Delete a banner.
///
/// Requires `content.banners` for its organization unit. To keep its history, archive it
/// instead (`PATCH` with `status` = `archived`).
#[utoipa::path(
    delete,
    path = "/banners/{id}",
    extensions(("x-permissions" = json!(["content.banners"]))),
    operation_id = "delete_content_banner",
    security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Id of the banner.")),
    responses(
        (status = 204, description = "Deleted."),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
        (status = 403, description = "Missing permission `content.banners` for the unit; code `permission_denied`.", body = Problem),
        (status = 404, description = "No banner has this id within reach; code `content.banner_not_found`.", body = Problem),
    )
)]
pub(crate) async fn delete_banner(
    State(service): State<Arc<ContentService>>,
    tenant: TenantContext,
    principal: Principal,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Error> {
    service.delete_banner(&tenant, &principal, id).await?;
    Ok(StatusCode::NO_CONTENT)
}
