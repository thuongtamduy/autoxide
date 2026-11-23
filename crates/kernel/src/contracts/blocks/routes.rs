//! `GET /api/v1/kernel/blocks`: the blocks a tenant can use, with their props schema — the UI builds the
//! property form from here, and AI page builders read the same list.

use axum::extract::State;
use serde::Serialize;
use serde_json::Value;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use super::{BlockCategory, Blocks, Children};
use crate::access::Principal;
use crate::error::Error;
use crate::i18n::{Language, Translations};
use crate::response::{Data, Problem};
use crate::tenancy::TenantContext;

#[derive(Clone)]
pub(crate) struct BlocksState {
    pub blocks: Blocks,
    pub translations: Translations,
}

/// A block type that pages can use.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct BlockTypeView {
    /// `<module>.<block>`, e.g. `catalog.product_grid`.
    name: &'static str,
    /// The module that provides the block.
    module: &'static str,
    category: BlockCategory,
    /// Name of the block in the language of `Accept-Language`.
    label: String,
    /// One sentence on when to use the block, for people and AI page builders.
    ai_hint: String,
    /// JSON Schema (draft 2020-12 subset) of the block's `props`. Extra keywords: `x-responsive`
    /// (the value may be `{base, md?, lg?}` per screen size), `x-media` (an id from `media`),
    /// `x-link` (an http(s), mailto:, tel: link or a path on the site).
    #[schema(value_type = Object)]
    props_schema: Value,
    /// The public page carries data for this block in `blocks_data`.
    has_data: bool,
    children: Children,
    /// Plan feature the block needs; only blocks the tenant's plan includes are listed.
    feature: Option<&'static str>,
}

/// List the block types this tenant can use.
///
/// Any signed-in user. Blocks of modules that run on this server and are in the tenant's plan,
/// with their props schema; labels in the language of `Accept-Language`. Ordered by name.
#[utoipa::path(
    get,
    path = "/blocks",
    extensions(("x-permissions" = json!([]))),
    operation_id = "list_blocks",
    security(("bearer" = [])),
    responses(
        (status = 200, description = "The block types.", body = Data<Vec<BlockTypeView>>),
        (status = 401, description = "Not signed in; see `GET /api/v1/auth/me` for the codes.", body = Problem),
    )
)]
async fn list_blocks(
    State(state): State<BlocksState>,
    tenant: TenantContext,
    _principal: Principal,
    language: Language,
) -> Result<Data<Vec<BlockTypeView>>, Error> {
    let views = state
        .blocks
        .usable(&tenant)
        .await?
        .into_iter()
        .map(|info| BlockTypeView {
            name: info.block.name,
            module: info.module,
            category: info.block.category,
            label: state.translations.describe(info.block.label, language),
            ai_hint: state.translations.describe(info.block.ai_hint, language),
            props_schema: info.schema.clone(),
            has_data: info.block.has_data,
            children: info.block.children,
            feature: info.block.feature.map(|feature| feature.key()),
        })
        .collect();
    Ok(Data { data: views })
}

pub(crate) fn routes(state: BlocksState) -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(list_blocks)).with_state(state)
}
