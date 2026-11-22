//! Sổ khối của trang dựng từ khối (bước 67, D-229, D-20261122-block-registry).
//!
//! Module khai khối nó cung cấp bằng [`Module::blocks`](crate::module::Module::blocks): tên
//! `<module>.<khối>`, nhóm, nhãn và gợi ý cho AI (câu tiếng Anh, dịch qua `text:` của module —
//! `cargo xtask check` chặn câu chưa dịch), JSON Schema của props (phần con ở [`schema`]), có cần
//! dữ liệu lúc đọc công khai không, có chứa khối con không, tính năng của gói cần có. Khai sai
//! (tên, schema) thì chương trình không khởi động.
//!
//! Khối cần dữ liệu (lưới sản phẩm, flash sale…) gắn một [`BlockData`] lúc `start` bằng
//! `StartContext::block_data`. Module dùng khối (`content`) giữ [`Blocks`] lấy từ
//! `StartContext::blocks` — sổ có cả khối của module khởi động SAU nó — và gọi
//! [`Blocks::resolve`]: kernel gom khối theo loại, gọi mỗi loại **một lần** (song song, có hạn
//! giờ), khối lỗi hay hết giờ thành `null`, trang vẫn trả.
//!
//! `GET /api/v1/kernel/blocks` liệt kê khối khách đang dùng được: của module đang chạy
//! (`AUTOXIDE_MODULES`), module có trong gói của khách và tính năng của khối có trong gói.

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

/// Hạn giờ của mỗi lần gọi [`BlockData::resolve`]: quá hạn thì khối thành `null`.
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

/// Một loại khối module khai qua [`Module::blocks`](crate::module::Module::blocks).
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct BlockType {
    /// `<module>.<khối>`, vd `catalog.product_grid`.
    pub name: &'static str,
    pub category: BlockCategory,
    /// Tên khối trong bộ chọn (tiếng Anh; bản dịch ở `text:<câu>` của module).
    pub label: &'static str,
    /// Một câu cho AI biết khi nào dùng khối (tiếng Anh; dịch như `label`).
    pub ai_hint: &'static str,
    /// JSON Schema của props — nguồn kiểm duy nhất (phần con ở [`schema`]).
    pub props_schema: &'static str,
    /// Cần [`BlockData`] lúc đọc công khai không.
    pub has_data: bool,
    pub children: Children,
    /// Tính năng của gói (bước 35) cần có để dùng khối; tính năng phải do chính module khai.
    pub feature: Option<Feature>,
}

impl BlockType {
    /// Khối lá, không cần dữ liệu, không cần tính năng.
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

    /// Khối cần dữ liệu lúc đọc công khai: module phải gắn [`BlockData`] lúc `start`.
    #[must_use]
    pub const fn with_data(mut self) -> Self {
        self.has_data = true;
        self
    }

    /// Khối chứa khối con.
    #[must_use]
    pub const fn with_children(mut self) -> Self {
        self.children = Children::Blocks;
        self
    }

    /// Chỉ khách có tính năng `feature` trong gói mới dùng được khối.
    #[must_use]
    pub const fn requires(mut self, feature: Feature) -> Self {
        self.feature = Some(feature);
        self
    }
}

/// Một khối trong trang, gửi cho [`BlockData::resolve`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BlockRef {
    /// `id` của khối trong trang (duy nhất trong trang).
    pub id: String,
    /// Props đã qua kiểm theo schema của khối.
    pub props: Value,
}

impl BlockRef {
    pub fn new(id: impl Into<String>, props: Value) -> Self {
        BlockRef { id: id.into(), props }
    }
}

/// Ngữ cảnh của một lần đọc dữ liệu khối.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct BlockRequest<'a> {
    pub tenant: &'a TenantContext,
    /// Website đang xem (bước 34): đọc công khai của module lọc theo nó như endpoint công khai.
    pub site: &'a Site,
    /// Ngôn ngữ của trang, vd `vi`.
    pub locale: &'a str,
}

impl<'a> BlockRequest<'a> {
    pub fn new(tenant: &'a TenantContext, site: &'a Site, locale: &'a str) -> Self {
        BlockRequest { tenant, site, locale }
    }
}

/// Dữ liệu cho khối lúc đọc công khai, theo lô.
//
// `#[async_trait]`: gọi hàm `async` qua `dyn Trait`.
#[async_trait::async_trait]
pub trait BlockData: Send + Sync {
    /// Dữ liệu cho mọi khối loại `block` của trang trong MỘT lần gọi (không N+1): kết quả cùng
    /// thứ tự, cùng số phần tử với `items`. Chỉ trả dữ liệu công khai; tham chiếu không còn công
    /// khai → `None` (giao diện ẩn khối, cả trang không lỗi).
    async fn resolve(
        &self,
        request: &BlockRequest<'_>,
        block: &str,
        items: &[BlockRef],
    ) -> Result<Vec<Option<Value>>, Error>;
}

/// Một khối đã khai, kèm module và schema đã đọc.
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

/// Sổ khối của mọi module. Clone rẻ; mọi bản clone dùng chung một sổ.
#[derive(Clone, Default)]
pub struct Blocks(Arc<Inner>);

impl Blocks {
    /// Kiểm khai báo của mọi module rồi điền sổ (một lần, lúc dựng `Modules`).
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

    /// Khối tên `name`, nếu có module đang chạy khai nó.
    pub fn get(&self, name: &str) -> Option<Arc<BlockInfo>> {
        self.0.declared.get().and_then(|blocks| blocks.get(name)).cloned()
    }

    /// Gắn `source` cho khối `name` của `module` (lúc `start`).
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

    /// Khối khai `with_data` mà module chưa gắn [`BlockData`] (kiểm sau khi mọi module khởi động).
    pub(crate) fn missing_sources(&self) -> Vec<&'static str> {
        let sources = self.0.sources.read().unwrap_or_else(PoisonError::into_inner);
        self.declared()
            .filter(|info| info.block.has_data && !sources.contains_key(info.block.name))
            .map(|info| info.block.name)
            .collect()
    }

    /// Khối khách `tenant` đang dùng được: module có trong gói, tính năng của khối có trong gói.
    /// Theo tên.
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

    /// Kiểm `props` của khối `name` theo schema của nó; lỗi có đường dẫn bắt đầu bằng `at`.
    /// Khối không có trong sổ: một lỗi `unknown` ở `at`.
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

    /// Dữ liệu của các khối `items` (`(tên khối, khối)`): mỗi loại khối có [`BlockData`] được gọi
    /// một lần, song song, hạn [`RESOLVE_TIMEOUT`]. Kết quả theo `id` của khối; khối lỗi, hết giờ
    /// hay tham chiếu không còn công khai là `null`. Khối không cần dữ liệu không có trong kết quả.
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
