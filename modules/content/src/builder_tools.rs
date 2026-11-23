//! Tool MCP của trang dựng từ khối (bước 67): khối, bố cục, bản chủ đề, loại nội dung, AI tạo trang,
//! sức khoẻ website. Cùng service và cùng kiểm quyền (kể cả theo nút) với REST — luật kiến trúc 8: agent
//! bên ngoài dựng trang được với đúng quyền như người, không hơn. Bản chủ đề lưu qua tool luôn là bản
//! nháp (bật cần người: `POST /theme-variants/{id}/activate`).
//!
//! Thân của bản chủ đề và loại nội dung nhận dạng object chung (`fields`), đọc bằng đúng kiểu của REST:
//! schema của tool không được lồng kiểu khác (`$ref`), và một kiểu cho một việc thì hai đường không lệch.

use std::sync::Arc;

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::mcp::Tool;
use autoxide_kernel::pagination::{Page, PageQuery};
use autoxide_kernel::tenancy::TenantContext;
use autoxide_kernel::version::{IfMatch, etag};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::ai::pages::TurnKind;
use crate::entries::patch::Operation;
use crate::http::ai_pages::{AiTurnResponse, response as turn_response};
use crate::http::health::{HealthResponse, response as health_response};
use crate::http::page_request;
use crate::http::theme_variants::{ThemeVariantRequest, ThemeVariantResponse, variant_response};
use crate::http::types::{ContentTypeRequest, ContentTypeResponse, response as type_response};
use crate::permissions::{
    CONTENT_READ, CONTENT_THEMES, CONTENT_TYPES, CONTENT_WRITE, CONTENT_WRITE_OWN,
};
use crate::service::ContentService;
use crate::theme::variants::VariantInput;
use crate::types::TypeInput;

/// Phiên bản mong đợi từ tham số `version` (bắt buộc khi sửa, như `If-Match` của REST).
fn expected(version: &str) -> Result<Option<autoxide_kernel::time::Timestamp>, Error> {
    IfMatch::from_value(version).expected()
}

/// Đọc object `fields` bằng kiểu REST `T`, kiểm như `ValidJson`.
fn fields<T: DeserializeOwned + Validate>(value: Value) -> Result<T, Error> {
    let parsed: T = serde_json::from_value(value).map_err(|error| {
        Error::validation(vec![
            FieldError::new("fields", "invalid", "{error}").with_param("error", error.to_string()),
        ])
    })?;
    parsed.validate()?;
    Ok(parsed)
}

fn page_of(
    limit: Option<u32>,
    cursor: Option<String>,
) -> Result<autoxide_kernel::pagination::PageRequest, Error> {
    page_request(PageQuery::new(limit.map(|limit| limit.to_string()), cursor, None), Vec::new())
}

/// A block type usable in layouts.
#[derive(Debug, Serialize)]
struct BlockView {
    name: &'static str,
    module: &'static str,
    /// One sentence on when to use the block.
    use_when: &'static str,
    has_data: bool,
    /// Whether the block holds other blocks in `children`.
    holds_blocks: bool,
    props_schema: Value,
}

/// A page's block layout with its `version`.
#[derive(Debug, Serialize)]
struct LayoutView {
    id: Uuid,
    /// Pass it back as `version` to `update_page_layout`.
    version: String,
    /// `true` when the draft body is a block layout (otherwise a rich text document).
    is_layout: bool,
    body: Value,
}

/// A value with its `version` (the `ETag` of REST).
#[derive(Debug, Serialize)]
struct Versioned<T> {
    version: String,
    #[serde(flatten)]
    data: T,
}

/// No arguments.
#[derive(Debug, Default, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct NoArgs {}

/// A page to read.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PageId {
    /// Id of the page.
    id: Uuid,
}

/// A change to a page's block layout: either the whole `layout` or JSON Patch `operations`.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateLayoutInput {
    /// Id of the page.
    id: Uuid,
    /// The `version` you read (`get_page_layout`), or `*`.
    version: String,
    /// The whole layout `{"type": "layout", "schema": 1, "sections": [block…]}`; blocks come from
    /// `list_blocks`.
    #[schema(value_type = Option<Object>)]
    layout: Option<Value>,
    /// JSON Patch operations (RFC 6902) on the layout, e.g.
    /// `[{"op": "replace", "path": "/sections/0/props/text", "value": "Sale"}]`.
    #[schema(value_type = Option<Vec<Object>>)]
    operations: Option<Value>,
    /// Keep this version in the history now.
    #[serde(default)]
    save_revision: bool,
}

/// Paging, optionally for one website.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListInput {
    /// The website (organization unit); left out: the whole tenant.
    org_id: Option<Uuid>,
    /// Maximum number of items, 1 to 200. Defaults to 50.
    limit: Option<u32>,
    /// Where to continue: the `next_cursor` of the previous page.
    cursor: Option<String>,
}

/// A theme variant to create (as a draft) or replace.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SaveVariantInput {
    /// Id of the variant to replace; left out: create a new draft variant.
    id: Option<Uuid>,
    /// Replace only: the `version` you read, or `*`.
    version: Option<String>,
    /// The same body as `POST /api/v1/content/theme-variants`: `org_id`, `name`, `overrides`,
    /// `effect`, `effect_intensity`, `schedule`, `priority`, `preset_key`, `year`.
    #[schema(value_type = Object)]
    fields: Value,
}

/// A custom content type to create or change.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SaveTypeInput {
    /// Key of the type to change; left out: create a new type.
    key: Option<String>,
    /// Change only: the `version` you read, or `*`.
    version: Option<String>,
    /// The same body as `POST /api/v1/content/types` (create) or `PATCH /types/{key}` (change):
    /// `key`, `labels`, `icon`, `routable`, `path_prefix`, `body`, `has_seo`, `listable`, `active`,
    /// `org_id`, `detail_layout`.
    #[schema(value_type = Object)]
    fields: Value,
}

/// A request to the AI page builder.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AiPageInput {
    /// Id of the page.
    id: Uuid,
    /// What the page should be, or what to change, in plain words (1 to 2000 characters).
    instruction: String,
    /// `edit_page` only: block ids to limit the change to.
    selection: Option<Vec<String>>,
}

/// Which website.
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SiteInput {
    /// The website (organization unit).
    org_id: Uuid,
}

pub(crate) fn all(service: &Arc<ContentService>) -> Vec<Tool> {
    let mut tools = layout_tools(service);
    tools.extend(site_tools(service));
    tools
}

fn layout_tools(service: &Arc<ContentService>) -> Vec<Tool> {
    let (blocks, get, update) = (service.clone(), service.clone(), service.clone());
    let (compose, edit) = (service.clone(), service.clone());
    vec![
        Tool::new(
            "list_blocks",
            "List the block types this tenant can use in page layouts, with when to use each and \
             its props JSON Schema. Only these blocks are accepted.",
            move |tenant: TenantContext, _: Principal, _: NoArgs| {
                let service = blocks.clone();
                async move {
                    let usable = service.usable_blocks(&tenant).await?;
                    Ok::<Vec<BlockView>, Error>(
                        usable
                            .values()
                            .map(|info| BlockView {
                                name: info.block.name,
                                module: info.module,
                                use_when: info.block.ai_hint,
                                has_data: info.block.has_data,
                                holds_blocks: info.block.children
                                    == autoxide_kernel::blocks::Children::Blocks,
                                props_schema: info.schema.clone(),
                            })
                            .collect(),
                    )
                }
            },
        )
        .requires(CONTENT_READ)
        .read_only(),
        Tool::new(
            "get_page_layout",
            "Get the draft body of a page (its block layout) with the `version` to pass to \
             `update_page_layout`.",
            move |tenant: TenantContext, principal: Principal, input: PageId| {
                let service = get.clone();
                async move {
                    let entry = service.get(&tenant, &principal, input.id).await?;
                    let body: Value = serde_json::from_str(&entry.body).unwrap_or(Value::Null);
                    Ok::<LayoutView, Error>(LayoutView {
                        id: entry.id,
                        version: etag(entry.updated_at),
                        is_layout: crate::entries::layout::is_layout(&body),
                        body,
                    })
                }
            },
        )
        .requires(CONTENT_READ)
        .read_only(),
        Tool::new(
            "update_page_layout",
            "Change the draft block layout of a page: send the whole `layout`, or JSON Patch \
             `operations` (at most 200, all or nothing). The result is checked like a person's edit \
             and stays a draft until published. Pass the `version` you read.",
            move |tenant: TenantContext, principal: Principal, input: UpdateLayoutInput| {
                let service = update.clone();
                async move {
                    let expected = expected(&input.version)?;
                    let entry = match (input.layout, input.operations) {
                        (Some(layout), None) => {
                            service
                                .replace_layout(
                                    &tenant,
                                    &principal,
                                    input.id,
                                    layout,
                                    input.save_revision,
                                    expected,
                                )
                                .await?
                        }
                        (None, Some(operations)) => {
                            let operations: Vec<Operation> = serde_json::from_value(operations)
                                .map_err(|error| {
                                    Error::validation(vec![
                                        FieldError::new("operations", "invalid", "{error}")
                                            .with_param("error", error.to_string()),
                                    ])
                                })?;
                            service
                                .patch_layout(&tenant, &principal, input.id, &operations, expected)
                                .await?
                        }
                        _ => {
                            return Err(Error::validation(vec![FieldError::new(
                                "layout",
                                "one_of",
                                "send exactly one of `layout` and `operations`",
                            )]));
                        }
                    };
                    let body: Value = serde_json::from_str(&entry.body).unwrap_or(Value::Null);
                    Ok::<LayoutView, Error>(LayoutView {
                        id: entry.id,
                        version: etag(entry.updated_at),
                        is_layout: true,
                        body,
                    })
                }
            },
        )
        .requires(CONTENT_WRITE)
        .or_requires(CONTENT_WRITE_OWN),
        Tool::new(
            "compose_page",
            "Ask the AI to build a page's block layout from a description. Runs in the background \
             and ends as a draft marked `ai_assisted`, never published; follow it with \
             `get_page_layout` or REST `GET /entries/{id}/ai/thread`. Counts toward the plan's \
             monthly AI page builds.",
            move |tenant: TenantContext, principal: Principal, input: AiPageInput| {
                let service = compose.clone();
                async move {
                    let turn = service
                        .request_ai_page(
                            &tenant,
                            &principal,
                            input.id,
                            TurnKind::Compose,
                            &input.instruction,
                            None,
                        )
                        .await?;
                    Ok::<AiTurnResponse, Error>(turn_response(turn))
                }
            },
        )
        .requires(CONTENT_WRITE)
        .or_requires(CONTENT_WRITE_OWN),
        Tool::new(
            "edit_page",
            "Ask the AI to change a page's block layout in words (\"make the hero red\"); \
             `selection` limits it to some blocks. Runs in the background; the result is a new \
             draft and the turn keeps the revision before it to undo.",
            move |tenant: TenantContext, principal: Principal, input: AiPageInput| {
                let service = edit.clone();
                async move {
                    let turn = service
                        .request_ai_page(
                            &tenant,
                            &principal,
                            input.id,
                            TurnKind::Edit,
                            &input.instruction,
                            input.selection,
                        )
                        .await?;
                    Ok::<AiTurnResponse, Error>(turn_response(turn))
                }
            },
        )
        .requires(CONTENT_WRITE)
        .or_requires(CONTENT_WRITE_OWN),
    ]
}

fn site_tools(service: &Arc<ContentService>) -> Vec<Tool> {
    let (variants, save_variant) = (service.clone(), service.clone());
    let (types, save_type, health) = (service.clone(), service.clone(), service.clone());
    vec![
        Tool::new(
            "list_theme_variants",
            "List the scheduled theme variants (seasons, holidays, hours) of a website.",
            move |tenant: TenantContext, principal: Principal, input: ListInput| {
                let service = variants.clone();
                async move {
                    let org = input
                        .org_id
                        .unwrap_or_else(|| autoxide_kernel::tenancy::root_of(tenant.id()).as_uuid());
                    let page = page_of(input.limit, input.cursor)?;
                    let found = service.list_variants(&tenant, &principal, org, &page).await?;
                    Ok::<Page<ThemeVariantResponse>, Error>(found.map(variant_response))
                }
            },
        )
        .requires(CONTENT_THEMES)
        .read_only(),
        Tool::new(
            "save_theme_variant",
            "Create a theme variant as a draft, or replace one (pass `id` and `version`). It is \
             never switched on by this tool: a person activates it.",
            move |tenant: TenantContext, principal: Principal, input: SaveVariantInput| {
                let service = save_variant.clone();
                async move {
                    let body: VariantInput = fields::<ThemeVariantRequest>(input.fields)?.into();
                    let row = match input.id {
                        None => service.create_variant(&tenant, &principal, body).await?,
                        Some(id) => {
                            let version = input.version.as_deref().unwrap_or_default();
                            let expected = expected(version)?;
                            service.put_variant(&tenant, &principal, id, body, expected).await?
                        }
                    };
                    let version = etag(row.updated_at);
                    Ok::<Versioned<ThemeVariantResponse>, Error>(Versioned {
                        version,
                        data: variant_response(row),
                    })
                }
            },
        )
        .requires(CONTENT_THEMES),
        Tool::new(
            "list_types",
            "List the tenant's custom content types (team, projects, branches…).",
            move |tenant: TenantContext, principal: Principal, input: ListInput| {
                let service = types.clone();
                async move {
                    let page = page_of(input.limit, input.cursor)?;
                    let found = service.list_types(&tenant, &principal, &page).await?;
                    Ok::<Page<ContentTypeResponse>, Error>(found.map(type_response))
                }
            },
        )
        .requires(CONTENT_READ)
        .read_only(),
        Tool::new(
            "save_type",
            "Create a custom content type, or change one (pass `key` and `version`). Its own fields \
             are custom fields of target `content.type.<key>`.",
            move |tenant: TenantContext, principal: Principal, input: SaveTypeInput| {
                let service = save_type.clone();
                async move {
                    let body: TypeInput = fields::<ContentTypeRequest>(input.fields)?.into();
                    let row = match input.key {
                        None => service.create_type(&tenant, &principal, body).await?,
                        Some(key) => {
                            let version = input.version.as_deref().unwrap_or_default();
                            let expected = expected(version)?;
                            let body = TypeInput { org_id: None, ..body };
                            service.update_type(&tenant, &principal, &key, body, expected).await?
                        }
                    };
                    let version = etag(row.updated_at);
                    Ok::<Versioned<ContentTypeResponse>, Error>(Versioned {
                        version,
                        data: type_response(row),
                    })
                }
            },
        )
        .requires(CONTENT_TYPES),
        Tool::new(
            "site_health",
            "Get the last health check of a website: missing SEO descriptions or image alt texts, \
             duplicate titles, broken links, hidden embedded records, finished countdowns and missing \
             addresses asked for often, each with how to fix it. Checked every day; `null` before the \
             first check.",
            move |tenant: TenantContext, principal: Principal, input: SiteInput| {
                let service = health.clone();
                async move {
                    let report = service.site_health(&tenant, &principal, input.org_id).await?;
                    Ok::<Option<HealthResponse>, Error>(report.map(health_response))
                }
            },
        )
        .requires(CONTENT_READ)
        .read_only(),
    ]
}
