//! Block registry for pages built from blocks (step 67, D-229, D-20261122-block-registry).
//!
//! A module declares the blocks it provides with [`Module::blocks`](crate::module::Module::blocks): the name
//! `<module>.<block>`, group, label and hint for AI (English sentences, translated through the module's
//! `text:` — `cargo xtask check` rejects untranslated sentences), the JSON Schema of the props (sub-parts in
//! [`schema`]), whether it needs data on public reads, whether it can contain child blocks, and the plan
//! feature it requires. A wrong declaration (name, schema) stops the program from starting.
//!
//! A block that needs data (product grid, flash sale…) attaches a [`BlockData`] at `start` with
//! `StartContext::block_data`. A module that uses blocks (`content`) keeps the [`Blocks`] obtained from
//! `StartContext::blocks` — the registry also holds the blocks of modules that start AFTER it — and calls
//! [`Blocks::resolve`]: the kernel groups blocks by type and calls each type **once** (in parallel, with a
//! timeout); a block that fails or times out becomes `null` and the page is still returned.
//!
//! `GET /api/v1/kernel/blocks` lists the blocks a tenant can use: those of running modules
//! (`AUTOXIDE_MODULES`), modules included in the tenant's plan, and blocks whose feature is in the plan.

mod routes;
pub mod schema;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, OnceLock, PoisonError, RwLock};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use utoipa::ToSchema;

use crate::entitlement::{EntitlementsHandle, Feature};
use crate::error::{Error, FieldError};
use crate::tenancy::{Site, TenantContext};

pub(crate) use routes::{BlocksState, routes};

/// Timeout of each [`BlockData::resolve`] call: past it the block becomes `null`.
pub const RESOLVE_TIMEOUT: Duration = Duration::from_secs(2);

// Nhóm của khối trong bộ chọn khối.
/// Group of a block in the block picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BlockCategory {
    /// Structure: sections, columns, spacers.
    Layout,
    /// Text: headings, paragraphs, buttons, FAQ.
    Text,
    /// Images, videos, galleries.
    Media,
    /// Leads: announcement bars, countdowns, sign-up popups, forms.
    Lead,
    /// Other content of the website, embedded.
    Embed,
    /// Selling: products, promotions, cart.
    Commerce,
    /// Community: comments, reviews.
    Social,
    /// Navigation: menus, logo, search box.
    Navigation,
    /// Legal: the owner information footer.
    Legal,
}

// Khối có chứa khối con không.
/// Whether a block holds other blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Children {
    /// A leaf block.
    None,
    /// Holds blocks (sections, columns, tabs).
    Blocks,
}

/// A block type that a module declares through [`Module::blocks`](crate::module::Module::blocks).
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct BlockType {
    /// `<module>.<block>`, e.g. `catalog.product_grid`.
    pub name: &'static str,
    pub category: BlockCategory,
    /// Block name in the picker (English; the translation is under the module's `text:<sentence>`).
    pub label: &'static str,
    /// One sentence telling AI when to use the block (English; translated like `label`).
    pub ai_hint: &'static str,
    /// JSON Schema of the props — the only source of validation (sub-parts in [`schema`]).
    pub props_schema: &'static str,
    /// Whether it needs [`BlockData`] on public reads.
    pub has_data: bool,
    pub children: Children,
    /// Plan feature (step 35) required to use the block; the feature must be declared by the module itself.
    pub feature: Option<Feature>,
}

impl BlockType {
    /// A leaf block: needs no data and no feature.
    pub const fn new(
        name: &'static str,
        category: BlockCategory,
        label: &'static str,
        ai_hint: &'static str,
        props_schema: &'static str,
    ) -> Self {
        BlockType {
            name,
            category,
            label,
            ai_hint,
            props_schema,
            has_data: false,
            children: Children::None,
            feature: None,
        }
    }

    /// A block that needs data on public reads: the module must attach a [`BlockData`] at `start`.
    #[must_use]
    pub const fn with_data(mut self) -> Self {
        self.has_data = true;
        self
    }

    /// A block that contains child blocks.
    #[must_use]
    pub const fn with_children(mut self) -> Self {
        self.children = Children::Blocks;
        self
    }

    /// Only tenants with the feature `feature` in their plan can use the block.
    #[must_use]
    pub const fn requires(mut self, feature: Feature) -> Self {
        self.feature = Some(feature);
        self
    }
}

/// A block in a page, passed to [`BlockData::resolve`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BlockRef {
    /// `id` of the block in the page (unique within the page).
    pub id: String,
    /// Props already validated against the block's schema.
    pub props: Value,
}

impl BlockRef {
    pub fn new(id: impl Into<String>, props: Value) -> Self {
        BlockRef { id: id.into(), props }
    }
}

/// Context of one block data read.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct BlockRequest<'a> {
    pub tenant: &'a TenantContext,
    /// The website being viewed (step 34): the module's public read filters by it like a public endpoint.
    pub site: &'a Site,
    /// Language of the page, e.g. `vi`.
    pub locale: &'a str,
}

impl<'a> BlockRequest<'a> {
    pub fn new(tenant: &'a TenantContext, site: &'a Site, locale: &'a str) -> Self {
        BlockRequest { tenant, site, locale }
    }
}

/// Data for blocks on public reads, in batches.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait BlockData: Send + Sync {
    /// Data for every block of type `block` in the page in ONE call (no N+1): the result has the same order
    /// and the same number of elements as `items`. Returns public data only; a reference that is no longer
    /// public → `None` (the UI hides the block, the page as a whole does not fail).
    async fn resolve(
        &self,
        request: &BlockRequest<'_>,
        block: &str,
        items: &[BlockRef],
    ) -> Result<Vec<Option<Value>>, Error>;
}

/// A declared block, with its module and the parsed schema.
#[derive(Debug)]
#[non_exhaustive]
pub struct BlockInfo {
    pub module: &'static str,
    pub block: BlockType,
    pub schema: Value,
}

#[derive(Default)]
struct Inner {
    declared: OnceLock<BTreeMap<&'static str, Arc<BlockInfo>>>,
    // Ghi chỉ lúc khởi động; không giữ khoá qua `await`.
    sources: RwLock<HashMap<&'static str, Arc<dyn BlockData>>>,
    entitlements: OnceLock<EntitlementsHandle>,
}

/// Block registry of all modules. Cheap to clone; all clones share one registry.
#[derive(Clone, Default)]
pub struct Blocks(Arc<Inner>);

impl Blocks {
    /// Validates the declarations of all modules and then fills the registry (once, when `Modules` is built).
    pub(crate) fn fill(
        &self,
        declared: Vec<(&'static str, Vec<Feature>, Vec<BlockType>)>,
        entitlements: EntitlementsHandle,
        problems: &mut Vec<String>,
    ) {
        let mut blocks = BTreeMap::new();
        for (module, features, types) in declared {
            for block in types {
                let own = block
                    .name
                    .strip_prefix(module)
                    .and_then(|rest| rest.strip_prefix('.'))
                    .is_some_and(crate::module::is_identifier);
                if !own {
                    problems.push(format!(
                        "block `{}` of module `{module}` must be named `{module}.<lower_case_name>`",
                        block.name
                    ));
                }
                if block.label.trim().is_empty() || block.ai_hint.trim().is_empty() {
                    problems.push(format!("block `{}` needs a label and an AI hint", block.name));
                }
                if let Some(feature) = block.feature
                    && !features.iter().any(|known| known.key() == feature.key())
                {
                    problems.push(format!(
                        "block `{}` requires feature `{}`, which module `{module}` does not declare",
                        block.name,
                        feature.key()
                    ));
                }
                let schema = match serde_json::from_str::<Value>(block.props_schema) {
                    Ok(schema) => {
                        problems.extend(schema::declaration_problems(block.name, &schema));
                        schema
                    }
                    Err(error) => {
                        problems.push(format!(
                            "props schema of block `{}` is not JSON: {error}",
                            block.name
                        ));
                        Value::Null
                    }
                };
                let info = Arc::new(BlockInfo { module, block, schema });
                if blocks.insert(block.name, info).is_some() {
                    problems.push(format!("block `{}` is declared twice", block.name));
                }
            }
        }
        let _ = self.0.declared.set(blocks);
        let _ = self.0.entitlements.set(entitlements);
    }

    fn declared(&self) -> impl Iterator<Item = &Arc<BlockInfo>> {
        self.0.declared.get().into_iter().flat_map(BTreeMap::values)
    }

    /// The block named `name`, if a running module declares it.
    pub fn get(&self, name: &str) -> Option<Arc<BlockInfo>> {
        self.0.declared.get().and_then(|blocks| blocks.get(name)).cloned()
    }

    /// Attaches `source` to the block `name` of `module` (at `start`).
    pub(crate) fn register(
        &self,
        module: &'static str,
        name: &'static str,
        source: Arc<dyn BlockData>,
    ) -> Result<(), Error> {
        let info = self.get(name).filter(|info| info.module == module && info.block.has_data);
        if info.is_none() {
            return Err(Error::internal(format!(
                "module `{module}` registers block data for `{name}`, which it does not declare \
                 with `BlockType::with_data`"
            )));
        }
        let mut sources = self.0.sources.write().unwrap_or_else(PoisonError::into_inner);
        if sources.insert(name, source).is_some() {
            return Err(Error::internal(format!("block data for `{name}` is registered twice")));
        }
        Ok(())
    }

    /// Blocks declared `with_data` to which the module has not attached a [`BlockData`] (checked after all modules start).
    pub(crate) fn missing_sources(&self) -> Vec<&'static str> {
        let sources = self.0.sources.read().unwrap_or_else(PoisonError::into_inner);
        self.declared()
            .filter(|info| info.block.has_data && !sources.contains_key(info.block.name))
            .map(|info| info.block.name)
            .collect()
    }

    /// Blocks the tenant `tenant` can use: the module is in the plan and the block's feature is in the plan.
    /// Ordered by name.
    pub async fn usable(&self, tenant: &TenantContext) -> Result<Vec<Arc<BlockInfo>>, Error> {
        let Some(entitlements) = self.0.entitlements.get() else {
            return Ok(self.declared().cloned().collect());
        };
        let snapshot = entitlements.snapshot(tenant.id()).await?;
        let catalog = entitlements.catalog();
        Ok(self
            .declared()
            .filter(|info| {
                (catalog.is_always_enabled(info.module) || snapshot.has_module(info.module))
                    && info.block.feature.is_none_or(|feature| snapshot.has_feature(feature.key()))
            })
            .cloned()
            .collect())
    }

    /// Validates `props` of the block `name` against its schema; errors carry a path starting at `at`.
    /// A block not in the registry: one `unknown` error at `at`.
    pub fn check_props(&self, name: &str, props: &Value, at: &str) -> Vec<FieldError> {
        let mut found = Vec::new();
        match self.get(name) {
            Some(info) => schema::check(&info.schema, props, at, &mut found),
            None => found.push(
                FieldError::new(at, "unknown", "block `{block}` is not available")
                    .with_param("block", name),
            ),
        }
        found
    }

    /// Data of the blocks `items` (`(block name, block)`): each block type that has a [`BlockData`] is called
    /// once, in parallel, with the timeout [`RESOLVE_TIMEOUT`]. The result is keyed by the block's `id`; a
    /// block that fails, times out or whose reference is no longer public is `null`. A block that needs no
    /// data is absent from the result.
    pub async fn resolve(
        &self,
        request: &BlockRequest<'_>,
        items: &[(&str, BlockRef)],
    ) -> BTreeMap<String, Value> {
        let mut groups: BTreeMap<&str, (Arc<dyn BlockData>, Vec<BlockRef>)> = BTreeMap::new();
        {
            let sources = self.0.sources.read().unwrap_or_else(PoisonError::into_inner);
            for (name, item) in items {
                if let Some(source) = sources.get(name) {
                    groups
                        .entry(name)
                        .or_insert_with(|| (source.clone(), Vec::new()))
                        .1
                        .push(item.clone());
                }
            }
        }
        let answers = futures_util::future::join_all(groups.into_iter().map(
            |(name, (source, refs))| async move {
                let answer =
                    tokio::time::timeout(RESOLVE_TIMEOUT, source.resolve(request, name, &refs))
                        .await;
                (name, refs, answer)
            },
        ))
        .await;
        let mut resolved = BTreeMap::new();
        for (name, refs, answer) in answers {
            let values = match answer {
                Ok(Ok(values)) if values.len() == refs.len() => values,
                Ok(Ok(_)) => {
                    tracing::warn!(block = name, "block data returned a wrong number of items");
                    Vec::new()
                }
                Ok(Err(error)) => {
                    tracing::warn!(block = name, error = %error, "block data failed");
                    Vec::new()
                }
                Err(_) => {
                    tracing::warn!(block = name, "block data timed out");
                    Vec::new()
                }
            };
            let mut values = values.into_iter();
            for item in refs {
                resolved.insert(item.id, values.next().flatten().unwrap_or(Value::Null));
            }
        }
        resolved
    }
}
