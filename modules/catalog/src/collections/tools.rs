//! Tool MCP cho bộ sưu tập (bước 55): cùng service, cùng permission với REST (luật kiến trúc 8).
//! Tool không bật bộ sưu tập (`active`): chỉ tạo / sửa bản nháp, người dùng bật lên (D-109). Chữ do
//! agent viết mặc định mang dấu `ai_assisted`, như tool banner của `content`.

use autoxide_kernel::time::Timestamp;
use std::collections::BTreeMap;
use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::mcp::{PageInput, Tool};
use autoxide_kernel::pagination::Page;
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, etag};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use super::service::{CollectionDetail, CollectionPatch, CollectionText, NewCollection};
use crate::http::collections::{CollectionResponse, collection_response};
use crate::permissions::{COLLECTIONS_MANAGE, PRODUCTS_READ};
use crate::service::CatalogService;

/// Paging and filter of the collection list.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogListCollectionsInput)]
pub(crate) struct ListCollectionsInput {
    /// `draft`, `active` or `archived`.
    status: Option<String>,
    /// 1 to 200; defaults to 50.
    limit: Option<u32>,
    /// The `next_cursor` of the previous page.
    cursor: Option<String>,
    /// `true` to also get `pagination.total` (the number of matching items) on the first page.
    count: Option<bool>,
}

/// Texts of a collection in one language.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogCollectionTextInput)]
pub(crate) struct TextInput {
    title: String,
    slug: Option<String>,
    #[serde(default)]
    description: String,
    #[serde(default)]
    meta_title: String,
    #[serde(default)]
    meta_description: String,
}

/// A collection to draft. Without `id`: a new draft. With `id` and `version`: change it (fields
/// left out stay).
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = CatalogSaveCollectionInput)]
pub(crate) struct SaveCollectionInput {
    /// The collection to change; omit to create one.
    id: Option<Uuid>,
    /// Required with `id`: the `version` from `list_collections` or the previous save.
    version: Option<String>,
    /// To create: `manual` or `rule`.
    kind: Option<String>,
    /// To create: the owning unit; omit for the whole tenant.
    org_id: Option<Uuid>,
    /// Rule of a `rule` collection, e.g. `{"category_ids": ["…"], "price_max": 500000}`.
    #[schema(value_type = Option<Object>)]
    rules: Option<Value>,
    /// `manual`, `newest`, `price_asc`, `price_desc` or `best_selling`.
    sort: Option<String>,
    /// A public picture (media item id).
    media_id: Option<Uuid>,
    starts_at: Option<Timestamp>,
    ends_at: Option<Timestamp>,
    /// Texts by language (e.g. `{"vi": {"title": "…"}}`).
    #[serde(default)]
    #[schema(value_type = Object)]
    texts: BTreeMap<String, TextInput>,
    /// `manual` collections: the chosen products, in order (replaces the list).
    product_ids: Option<Vec<Uuid>>,
    /// `draft` or `archived`; a tool never makes a collection `active`.
    status: Option<String>,
    /// The texts came from AI; defaults to `true` for a tool.
    ai_assisted: Option<bool>,
}

/// A collection and its `version`.
#[derive(Debug, Serialize)]
struct Versioned {
    /// Send it back in `version` to change the collection.
    version: String,
    #[serde(flatten)]
    collection: CollectionResponse,
}

fn versioned(detail: CollectionDetail) -> Versioned {
    Versioned {
        version: etag(detail.collection.updated_at),
        collection: collection_response(detail),
    }
}

fn texts(input: BTreeMap<String, TextInput>) -> BTreeMap<String, CollectionText> {
    input
        .into_iter()
        .map(|(locale, text)| {
            let text = CollectionText {
                title: text.title,
                slug: text.slug,
                description: text.description,
                meta_title: text.meta_title,
                meta_description: text.meta_description,
            };
            (locale, text)
        })
        .collect()
}

async fn save(
    service: &CatalogService,
    tenant: &TenantContext,
    principal: &Principal,
    input: SaveCollectionInput,
) -> Result<Versioned, Error> {
    if input.status.as_deref() == Some("active") {
        return Err(Error::forbidden(
            "catalog.collection_activation_needs_person",
            "An AI tool only drafts collections; a person turns them on with `PATCH /api/v1/catalog/collections/<id>`.",
        ));
    }
    let ai_assisted = input.ai_assisted.unwrap_or(true);
    let detail = match input.id {
        Some(id) => {
            let version = input.version.as_deref().ok_or_else(|| {
                Error::validation(vec![FieldError::new(
                    "version",
                    "required",
                    "is needed with `id`",
                )])
            })?;
            let expected = IfMatch::from_value(version).expected()?;
            let patch = CollectionPatch {
                rules: input.rules,
                sort: input.sort,
                media_id: input.media_id.map(Some),
                status: input.status,
                starts_at: input.starts_at.map(Some),
                ends_at: input.ends_at.map(Some),
                texts: texts(input.texts),
                ai_assisted: Some(ai_assisted),
            };
            service.update_collection(tenant, principal, id, patch, expected).await?
        }
        None => {
            let kind = input.kind.ok_or_else(|| {
                Error::validation(vec![FieldError::new("kind", "required", "is needed to create")])
            })?;
            let new = NewCollection {
                org_id: input.org_id,
                kind,
                rules: input.rules,
                sort: input.sort,
                media_id: input.media_id,
                status: Some(input.status.unwrap_or_else(|| "draft".to_owned())),
                starts_at: input.starts_at,
                ends_at: input.ends_at,
                texts: texts(input.texts),
                ai_assisted,
            };
            service.create_collection(tenant, principal, new).await?
        }
    };
    let detail = match input.product_ids {
        Some(products) => {
            let id = detail.collection.id;
            let version = Some(detail.collection.updated_at);
            service.set_collection_items(tenant, principal, id, products, version).await?
        }
        None => detail,
    };
    Ok(versioned(detail))
}

pub(crate) fn all(service: &Arc<CatalogService>) -> Vec<Tool> {
    let (list, save_one) = (service.clone(), service.clone());
    vec![
        Tool::new(
            "list_collections",
            "List the product collections in your scope (manual or by rule), newest first, each \
             with its `version`.",
            move |tenant: TenantContext, principal: Principal, input: ListCollectionsInput| {
                let service = list.clone();
                async move {
                    let page = PageInput::new(input.limit, input.cursor, None)
                        .with_count(input.count.unwrap_or(false))
                        .page()?;
                    let found = service
                        .list_collections(&tenant, &principal, input.status.as_deref(), &page)
                        .await?;
                    Ok::<Page<Versioned>, Error>(found.map(versioned))
                }
            },
        )
        .requires(PRODUCTS_READ)
        .read_only(),
        Tool::new(
            "save_collection",
            "Draft a product collection, or change one (with `id` and `version`): texts and SEO by \
             language, a rule (categories, attributes, price range, new, on promotion) or chosen \
             products in order, and the sort. A tool never turns a collection on: a person sets it \
             `active`.",
            move |tenant: TenantContext, principal: Principal, input: SaveCollectionInput| {
                let service = save_one.clone();
                async move { save(&service, &tenant, &principal, input).await }
            },
        )
        .requires(COLLECTIONS_MANAGE),
    ]
}
