//! Tool MCP cho banner (bước 55): cùng service, cùng permission với REST (luật kiến trúc 8). Tool
//! không bật banner: chỉ tạo / sửa bản nháp, người dùng bật lên (D-109). Chữ do agent viết mặc
//! định mang dấu `ai_assisted`.

use autoxide_kernel::time::Timestamp;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::pagination::{Page, PageQuery};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, etag};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use super::items::{BannerFields, BannerFilter, BannerPatch, Link, NewBanner, PageTarget, Via};
use crate::http::{BannerResponse, banner_response, page_request};
use crate::permissions::{CONTENT_BANNERS, CONTENT_READ};
use crate::service::ContentService;

/// Filters and paging of the banner list; omitted fields do not filter.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListBannersInput {
    /// Key of a placement, e.g. `home_hero`.
    placement: Option<String>,
    /// Only banners of this organization unit.
    org_id: Option<Uuid>,
    /// `draft`, `active` or `archived`.
    status: Option<String>,
    /// Only this language, e.g. `vi`.
    locale: Option<String>,
    /// 1 to 200; defaults to 50.
    #[schema(minimum = 1, maximum = 200)]
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `true` to also get `pagination.total` (the number of matching items) on the first page.
    count: Option<bool>,
    /// `desc` (newest first, the default) or `asc`.
    order: Option<String>,
}

/// A banner to draft. Without `id`: a new draft in `placement`. With `id` and `version`: change
/// that banner (fields left out stay as they are).
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SaveBannerInput {
    /// The banner to change; omit to create one.
    id: Option<Uuid>,
    /// Required with `id`: the `version` from `list_banners` or the previous save.
    version: Option<String>,
    /// Key of the placement (to create), e.g. `home_hero`.
    placement: Option<String>,
    /// The organization unit (store) it belongs to (to create); omit for every store.
    org_id: Option<Uuid>,
    /// Language (to create); omitted: the default language.
    locale: Option<String>,
    /// `image` (needs `media_id`) or `text`.
    kind: Option<String>,
    /// At most 120 characters.
    title: Option<String>,
    /// At most 300 characters.
    subtitle: Option<String>,
    /// Text of the button, at most 60 characters.
    cta_label: Option<String>,
    /// Text banners: `primary`, `secondary`, `accent` or `neutral`.
    tone: Option<String>,
    /// A public picture (media item id) with the placement's ratio and size.
    media_id: Option<Uuid>,
    /// Picture for phones.
    mobile_media_id: Option<Uuid>,
    /// Text alternative of the picture.
    alt: Option<String>,
    /// Only on these kinds of pages (e.g. `category`).
    page_kinds: Option<Vec<String>>,
    /// Only on these pages, each `<kind>:<id>` of the object directory (e.g.
    /// `catalog.category:0190…`).
    page_targets: Option<Vec<String>>,
    /// Link to an address (`https://…` or a path `/…`).
    link_url: Option<String>,
    /// Link to an object instead: its kind (e.g. `catalog.product`) and `link_id`.
    link_kind: Option<String>,
    link_id: Option<String>,
    /// Shown from (RFC 3339).
    starts_at: Option<Timestamp>,
    /// Shown until (RFC 3339).
    ends_at: Option<Timestamp>,
    /// `draft` or `archived`; a tool never makes a banner `active`.
    status: Option<String>,
    /// The text came from AI; defaults to `true` for a tool.
    ai_assisted: Option<bool>,
}

/// The new order of a placement's banners.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct OrderPlacementInput {
    /// Key of the placement.
    key: String,
    /// Banners first shown first; others keep their order after them.
    banner_ids: Vec<Uuid>,
    /// The placement's `version` (from `save_banner` / `order_placement`, or the `ETag` of
    /// `GET /api/v1/content/placements/{key}`), or `*`.
    version: String,
}

/// A banner and its `version`.
#[derive(Debug, Serialize)]
struct Versioned {
    /// Send it back in `version` to change the banner.
    version: String,
    #[serde(flatten)]
    banner: BannerResponse,
}

/// The order result.
#[derive(Debug, Serialize)]
struct Ordered {
    key: String,
    /// The placement's new `version`.
    version: String,
}

fn link(input: &SaveBannerInput, problems: &mut Vec<FieldError>) -> Option<Link> {
    match (&input.link_url, &input.link_kind, &input.link_id) {
        (None, None, None) => None,
        (Some(url), None, None) => Some(Link::Url(url.clone())),
        (None, Some(kind), Some(id)) => Some(Link::Ref { kind: kind.clone(), id: id.clone() }),
        _ => {
            problems.push(FieldError::new(
                "link_url",
                "invalid",
                "give either `link_url`, or both `link_kind` and `link_id`",
            ));
            None
        }
    }
}

fn pages(raw: Option<Vec<String>>, problems: &mut Vec<FieldError>) -> Option<Vec<PageTarget>> {
    raw.map(|raw| {
        raw.iter()
            .enumerate()
            .filter_map(|(index, value)| match value.rsplit_once(':') {
                Some((kind, id)) if !kind.is_empty() && !id.is_empty() => {
                    Some(PageTarget { kind: kind.to_owned(), id: id.to_owned() })
                }
                _ => {
                    problems.push(FieldError::new(
                        format!("page_targets[{index}]"),
                        "invalid",
                        "must be `<kind>:<id>`",
                    ));
                    None
                }
            })
            .collect()
    })
}

async fn save(
    service: &ContentService,
    tenant: &TenantContext,
    principal: &Principal,
    input: SaveBannerInput,
) -> Result<Versioned, Error> {
    let mut problems = Vec::new();
    let link = link(&input, &mut problems);
    let page_targets = pages(input.page_targets.clone(), &mut problems);
    let ai_assisted = input.ai_assisted.unwrap_or(true);
    let (row, placement) = match input.id {
        Some(id) => {
            let Some(version) = input.version.as_deref() else {
                problems.push(FieldError::new("version", "required", "is needed with `id`"));
                return Err(Error::validation(problems));
            };
            if !problems.is_empty() {
                return Err(Error::validation(problems));
            }
            let patch = BannerPatch {
                kind: input.kind,
                title: input.title,
                subtitle: input.subtitle,
                cta_label: input.cta_label,
                tone: input.tone.map(Some),
                media_id: input.media_id.map(Some),
                mobile_media_id: input.mobile_media_id.map(Some),
                alt: input.alt,
                page_kinds: input.page_kinds,
                page_targets,
                link,
                starts_at: input.starts_at.map(Some),
                ends_at: input.ends_at.map(Some),
                status: input.status,
                ai_assisted: Some(ai_assisted),
            };
            let expected = IfMatch::from_value(version).expected()?;
            service.update_banner(tenant, principal, id, patch, expected, Via::Tool).await?
        }
        None => {
            let Some(placement) = input.placement else {
                problems.push(FieldError::new("placement", "required", "is needed to create"));
                return Err(Error::validation(problems));
            };
            if !problems.is_empty() {
                return Err(Error::validation(problems));
            }
            let new = NewBanner {
                placement,
                org_id: input.org_id,
                fields: BannerFields {
                    locale: input.locale,
                    kind: input.kind,
                    title: input.title.unwrap_or_default(),
                    subtitle: input.subtitle.unwrap_or_default(),
                    cta_label: input.cta_label.unwrap_or_default(),
                    tone: input.tone,
                    media_id: input.media_id,
                    mobile_media_id: input.mobile_media_id,
                    alt: input.alt.unwrap_or_default(),
                    page_kinds: input.page_kinds.unwrap_or_default(),
                    page_targets: page_targets.unwrap_or_default(),
                    link: link.unwrap_or_default(),
                    starts_at: input.starts_at,
                    ends_at: input.ends_at,
                    status: None,
                    ai_assisted,
                },
            };
            service.create_banner(tenant, principal, new, Via::Tool).await?
        }
    };
    Ok(Versioned { version: etag(row.updated_at), banner: banner_response(row, &placement.key) })
}

pub(crate) fn all(service: &Arc<ContentService>) -> Vec<Tool> {
    let (list, save_one, order) = (service.clone(), service.clone(), service.clone());
    vec![
        Tool::new(
            "list_banners",
            "List banners of display placements (home hero, top strip…) within your reach, newest \
             first, each with its `version`. Filter by placement key, unit, status or language.",
            move |tenant: TenantContext, principal: Principal, input: ListBannersInput| {
                let service = list.clone();
                async move {
                    let paging = PageQuery::new(
                        input.limit.map(|limit| limit.to_string()),
                        input.cursor,
                        input.order,
                    )
                    .with_count(input.count.unwrap_or(false));
                    let page = page_request(paging, Vec::new())?;
                    let filter = BannerFilter {
                        placement: input.placement,
                        org_id: input.org_id,
                        status: input.status,
                        locale: input.locale,
                    };
                    let Page { data, pagination } =
                        service.list_banners(&tenant, &principal, filter, &page).await?;
                    let keys = service.placement_keys(&tenant).await?;
                    let data = data
                        .into_iter()
                        .map(|row| {
                            let key = keys.get(&row.placement_id).cloned().unwrap_or_default();
                            Versioned {
                                version: etag(row.updated_at),
                                banner: banner_response(row, &key),
                            }
                        })
                        .collect::<Vec<_>>();
                    Ok::<_, Error>(Page { data, pagination })
                }
            },
        )
        .requires(CONTENT_BANNERS)
        .or_requires(CONTENT_READ)
        .read_only(),
        Tool::new(
            "save_banner",
            "Draft a banner, or change one (with `id` and `version`). Pictures must be public and \
             fit the placement's ratio and size; links point to an address or to an object such \
             as `catalog.product`. A tool never turns a banner on: a person sets it `active`.",
            move |tenant: TenantContext, principal: Principal, input: SaveBannerInput| {
                let service = save_one.clone();
                async move { save(&service, &tenant, &principal, input).await }
            },
        )
        .requires(CONTENT_BANNERS),
        Tool::new(
            "order_placement",
            "Set the order of the banners of a placement at once (first listed shown first).",
            move |tenant: TenantContext, principal: Principal, input: OrderPlacementInput| {
                let service = order.clone();
                async move {
                    let expected = IfMatch::from_value(&input.version).expected()?;
                    let row = service
                        .order_placement(
                            &tenant,
                            &principal,
                            &input.key,
                            input.banner_ids,
                            expected,
                        )
                        .await?;
                    Ok::<_, Error>(Ordered { key: row.key, version: etag(row.updated_at) })
                }
            },
        )
        .requires(CONTENT_BANNERS),
    ]
}
