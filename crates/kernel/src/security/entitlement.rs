//! Quyền dùng theo khách (bước 35, D-323): module khai **tính năng** ([`Feature`]) và **hạn mức**
//! ([`Limit`]); một module (`plans`) công bố [`Entitlements`] — quyền dùng hiện tại của mỗi khách
//! (gói, gói thêm, ghi đè); mọi module hỏi qua [`EntitlementsHandle`].
//!
//! Cách nghĩ mượn Stripe Entitlements: ứng dụng chỉ hỏi "khách này đang được dùng gì", không suy từ
//! hoá đơn.
//!
//! - **Không có bên cung cấp** (không cài `plans`) hay khách **không có thuê bao và không có gói
//!   mặc định**: không giới hạn gì — mô hình một khách (chế độ `single`) không đổi.
//! - **Khách nền tảng** (khách vận hành) luôn không giới hạn: nhà vận hành không tự khoá mình.
//! - Module [`always_enabled`](crate::module::Module::always_enabled) (nền: `auth`, `files`…) không
//!   bao giờ bị gói chặn.
//!
//! Tay cầm có trước khi bên cung cấp khởi động (module đăng ký trước `plans` vẫn giữ được nó): kernel
//! gắn bên cung cấp vào sau khi mọi module đã khởi động.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};

use crate::error::Error;
use crate::tenancy::{DEFAULT_TENANT, TenantContext, TenantId};

/// Một tính năng bật / tắt theo gói, khoá `<module>.<tên>` (vd `content.ai_suggestions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Feature {
    key: &'static str,
    description: &'static str,
}

impl Feature {
    /// Tính năng `key` (`<module>.<tên>`), mô tả bằng tiếng Anh cho nhà vận hành soạn gói.
    pub const fn new(key: &'static str, description: &'static str) -> Self {
        Feature { key, description }
    }

    pub const fn key(&self) -> &'static str {
        self.key
    }

    pub const fn description(&self) -> &'static str {
        self.description
    }
}

/// Kỳ tính lượng dùng của hạn mức [`LimitKind::Metered`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Period {
    /// Tháng dương lịch theo UTC.
    Month,
}

/// Loại hạn mức.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LimitKind {
    /// Số bản ghi đang có (nút tổ chức, nhân sự, sản phẩm).
    Count,
    /// Lượng đang dùng, theo `unit` (vd `bytes`).
    Amount { unit: &'static str },
    /// Lượng dùng trong kỳ, về 0 đầu kỳ (request, token AI).
    Metered { period: Period },
}

impl LimitKind {
    /// Tên loại cho API: `count`, `amount`, `metered`.
    pub const fn as_str(&self) -> &'static str {
        match self {
            LimitKind::Count => "count",
            LimitKind::Amount { .. } => "amount",
            LimitKind::Metered { .. } => "metered",
        }
    }
}

/// Một hạn mức theo gói, khoá `<module>.<tên>` (vd `files.storage_bytes`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Limit {
    key: &'static str,
    description: &'static str,
    kind: LimitKind,
}

impl Limit {
    /// Hạn mức đếm số bản ghi đang có.
    pub const fn count(key: &'static str, description: &'static str) -> Self {
        Limit { key, description, kind: LimitKind::Count }
    }

    /// Hạn mức theo lượng đang dùng, đơn vị `unit` (vd `bytes`).
    pub const fn amount(key: &'static str, description: &'static str, unit: &'static str) -> Self {
        Limit { key, description, kind: LimitKind::Amount { unit } }
    }

    /// Hạn mức lượng dùng trong kỳ `period`.
    pub const fn metered(key: &'static str, description: &'static str, period: Period) -> Self {
        Limit { key, description, kind: LimitKind::Metered { period } }
    }

    pub const fn key(&self) -> &'static str {
        self.key
    }

    pub const fn description(&self) -> &'static str {
        self.description
    }

    pub const fn kind(&self) -> LimitKind {
        self.kind
    }
}

/// Bộ đếm thật trong database của module cho một hạn mức `Count` / `Amount`: job đối soát dùng nó
/// để sửa số đang dùng bị lệch (tiến trình chết giữa lúc giữ chỗ và lúc ghi). Module đăng ký trong
/// `start` bằng [`StartContext::usage_counter`](crate::module::StartContext::usage_counter).
#[async_trait::async_trait]
pub trait UsageCounter: Send + Sync {
    /// Lượng đang dùng thật của khách `tenant`.
    async fn count(&self, tenant: &TenantContext) -> Result<i64, Error>;
}

/// Khách được làm gì với dữ liệu của mình.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TenantAccess {
    #[default]
    Full,
    /// Thuê bao tạm khoá hay đã huỷ: chỉ xem, không ghi.
    ReadOnly,
}

/// Quyền dùng hiện tại của một khách (đã gộp gói, gói thêm, ghi đè).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TenantEntitlements {
    /// Không giới hạn gì: không thuê bao và không có gói mặc định, hay khách nền tảng.
    pub unrestricted: bool,
    pub access: TenantAccess,
    /// Module được bật (chưa gồm module `always_enabled` — chúng luôn bật).
    pub modules: BTreeSet<String>,
    pub features: BTreeSet<String>,
    /// Hạn mức theo khoá; `None` = không giới hạn. Khoá vắng mặt = 0 (không được dùng), trừ khi
    /// `unrestricted`.
    pub limits: BTreeMap<String, Option<i64>>,
}

impl TenantEntitlements {
    /// Không giới hạn, ghi đầy đủ.
    pub fn unrestricted() -> Self {
        TenantEntitlements { unrestricted: true, ..TenantEntitlements::default() }
    }

    /// Module `module` có được bật không (chưa xét `always_enabled`).
    pub fn has_module(&self, module: &str) -> bool {
        self.unrestricted || self.modules.contains(module)
    }

    pub fn has_feature(&self, feature: &str) -> bool {
        self.unrestricted || self.features.contains(feature)
    }

    /// Mức tối đa của hạn mức `limit`: `None` = không giới hạn.
    pub fn limit(&self, limit: &str) -> Option<i64> {
        if self.unrestricted { None } else { self.limits.get(limit).copied().unwrap_or(Some(0)) }
    }
}

/// Bên cung cấp quyền dùng (module `plans` công bố `dyn Entitlements`). Mọi hàm nhận ID khách:
/// sổ gói và thuê bao ở mặt điều khiển.
#[async_trait::async_trait]
pub trait Entitlements: Send + Sync {
    /// Quyền dùng hiện tại của khách `tenant` — có thể lấy từ bộ nhớ đệm ngắn hạn.
    async fn snapshot(&self, tenant: TenantId) -> Result<Arc<TenantEntitlements>, Error>;

    /// Giữ chỗ `amount` (> 0) của hạn mức `limit` theo `reference` (vd `catalog.product:<id>`),
    /// có hạn giữ. Gọi lại cùng `reference` + cùng `amount` → không giữ thêm; khác `amount` →
    /// `reference_reused`. Vượt hạn mức → `limit_exceeded`. Không gọi bên trong transaction ghi
    /// của module (SQLite một bên ghi: tự khoá mình).
    async fn reserve(
        &self,
        tenant: TenantId,
        limit: &Limit,
        amount: i64,
        reference: &str,
    ) -> Result<(), Error>;

    /// Chỗ đã giữ thành vĩnh viễn (bản ghi của module đã commit).
    async fn confirm(&self, tenant: TenantId, limit: &Limit, reference: &str) -> Result<(), Error>;

    /// Trả lại chỗ đã giữ hay đã xác nhận; gọi nhiều lần hay với `reference` lạ đều `Ok`.
    async fn release(&self, tenant: TenantId, limit: &Limit, reference: &str) -> Result<(), Error>;

    /// Ghi lượng dùng `quantity` của hạn mức `Metered`; trùng `reference` thì bỏ qua. Vượt mức
    /// của kỳ → `quota_exhausted`.
    async fn record_usage(
        &self,
        tenant: TenantId,
        limit: &Limit,
        quantity: i64,
        reference: &str,
    ) -> Result<(), Error>;
}

/// Tay cầm kernel đưa cho MỌI module lúc `start`. Trỏ tới `dyn Entitlements` khi có module công
/// bố; không có thì "không giới hạn". Clone rẻ (một `Arc`).
#[derive(Clone, Default)]
pub struct EntitlementsHandle {
    // `Arc<OnceLock<…>>`: mọi bản clone dùng chung một ô, kernel ghi đúng một lần sau khi mọi
    // module khởi động; đọc từ nhiều thread không cần lock.
    provider: Arc<OnceLock<Arc<dyn Entitlements>>>,
    catalog: EntitlementCatalog,
    meter: Arc<Meter>,
}

/// Hạn mức số request API theo tháng của mỗi khách (kernel khai, đếm ở lớp HTTP).
pub const API_REQUESTS: Limit = Limit::metered(
    "kernel.api_requests",
    "API requests (REST and MCP) a tenant can make each month.",
    Period::Month,
);

/// Đếm request API theo khách trong bộ nhớ của tiến trình; [`EntitlementsHandle::flush_usage`]
/// ghi gộp vào sổ lượng dùng (mỗi 60 giây). Tiến trình chết thì mất tối đa chừng ấy số đếm.
#[derive(Default)]
struct Meter {
    // `Mutex` chuẩn: chỉ giữ trong vài phép tính, không qua `.await`.
    counts: std::sync::Mutex<BTreeMap<TenantId, i64>>,
    /// Khách đã hết lượng request của kỳ: tới mốc này thì thôi chặn.
    exhausted: std::sync::Mutex<BTreeMap<TenantId, std::time::Instant>>,
}

impl EntitlementsHandle {
    pub(crate) fn new(catalog: EntitlementCatalog) -> Self {
        EntitlementsHandle { provider: Arc::default(), catalog, meter: Arc::default() }
    }

    /// Đếm một request API của khách `tenant` (lớp HTTP gọi). Không có bên cung cấp hay khách nền
    /// tảng: không đếm.
    pub fn count_request(&self, tenant: TenantId) {
        if self.provider_for(tenant).is_none() {
            return;
        }
        if let Ok(mut counts) = self.meter.counts.lock() {
            *counts.entry(tenant).or_default() += 1;
        }
    }

    /// Khách `tenant` đã hết lượng request của kỳ: số giây tới kỳ sau, để trả 429.
    pub fn request_quota_wait(&self, tenant: TenantId) -> Option<u64> {
        let mut exhausted = self.meter.exhausted.lock().ok()?;
        let until = *exhausted.get(&tenant)?;
        let now = std::time::Instant::now();
        if until <= now {
            exhausted.remove(&tenant);
            return None;
        }
        Some((until - now).as_secs().max(1))
    }

    /// Ghi số request đã đếm vào sổ lượng dùng, mỗi khách một dòng, `reference` = `node` + mốc
    /// `slot` (vd phút) — gọi lại cùng mốc không ghi trùng. Khách hết lượng: lần sau bị chặn tới
    /// đầu kỳ sau. Lỗi khác: ghi log, số đếm của lượt đó bỏ đi (đếm để tính tiền, không giữ tiền).
    pub async fn flush_usage(&self, node: &str, slot: i64) {
        let counts = match self.meter.counts.lock() {
            Ok(mut counts) => std::mem::take(&mut *counts),
            Err(_) => return,
        };
        for (tenant, quantity) in counts {
            let reference = format!("{node}:{slot}");
            match self.record_usage(tenant, &API_REQUESTS, quantity, &reference).await {
                Ok(()) => {}
                Err(error) if error.code() == "quota_exhausted" => {
                    let wait = error.retry_after().unwrap_or(60);
                    let until = std::time::Instant::now() + std::time::Duration::from_secs(wait);
                    if let Ok(mut exhausted) = self.meter.exhausted.lock() {
                        exhausted.insert(tenant, until);
                    }
                }
                Err(error) => {
                    tracing::warn!(tenant_id = %tenant, error = %error, "cannot record API request usage");
                }
            }
        }
    }

    /// Gắn bên cung cấp; lần gọi sau bị bỏ qua.
    pub(crate) fn fill(&self, provider: Option<Arc<dyn Entitlements>>) {
        if let Some(provider) = provider {
            let _ = self.provider.set(provider);
        }
    }

    /// Có bên cung cấp quyền dùng không (đã cài `plans`).
    pub fn is_enforced(&self) -> bool {
        self.provider.get().is_some()
    }

    /// Sổ tính năng và hạn mức của mọi module.
    pub fn catalog(&self) -> &EntitlementCatalog {
        &self.catalog
    }

    /// Bên cung cấp, trừ khi khách được miễn (khách nền tảng) hay chưa có bên cung cấp.
    fn provider_for(&self, tenant: TenantId) -> Option<&Arc<dyn Entitlements>> {
        if tenant == DEFAULT_TENANT { None } else { self.provider.get() }
    }

    /// Quyền dùng hiện tại của khách `tenant`.
    pub async fn snapshot(&self, tenant: TenantId) -> Result<Arc<TenantEntitlements>, Error> {
        match self.provider_for(tenant) {
            Some(provider) => provider.snapshot(tenant).await,
            None => Ok(Arc::new(TenantEntitlements::unrestricted())),
        }
    }

    /// Module `module` có được bật cho khách `tenant` không (module `always_enabled` luôn bật).
    pub async fn module_enabled(&self, tenant: TenantId, module: &str) -> Result<bool, Error> {
        if self.catalog.is_always_enabled(module) {
            return Ok(true);
        }
        Ok(self.snapshot(tenant).await?.has_module(module))
    }

    /// Như [`module_enabled`](Self::module_enabled), lỗi `module_not_enabled` khi chưa bật.
    pub async fn require_module(&self, tenant: TenantId, module: &str) -> Result<(), Error> {
        if self.module_enabled(tenant, module).await? {
            Ok(())
        } else {
            Err(module_not_enabled(module))
        }
    }

    pub async fn has_feature(&self, tenant: TenantId, feature: &Feature) -> Result<bool, Error> {
        Ok(self.snapshot(tenant).await?.has_feature(feature.key()))
    }

    /// Lỗi `feature_not_enabled` khi khách chưa có tính năng `feature`.
    pub async fn require_feature(&self, tenant: TenantId, feature: &Feature) -> Result<(), Error> {
        if self.has_feature(tenant, feature).await? {
            Ok(())
        } else {
            metrics::counter!("entitlement_denials_total", "kind" => "feature").increment(1);
            Err(feature_not_enabled(feature.key()))
        }
    }

    /// Xem [`Entitlements::reserve`]; không có bên cung cấp thì `Ok`.
    pub async fn reserve(
        &self,
        tenant: TenantId,
        limit: &Limit,
        amount: i64,
        reference: &str,
    ) -> Result<(), Error> {
        match self.provider_for(tenant) {
            Some(provider) => provider.reserve(tenant, limit, amount, reference).await,
            None => Ok(()),
        }
    }

    /// Xem [`Entitlements::confirm`]; không có bên cung cấp thì `Ok`.
    pub async fn confirm(
        &self,
        tenant: TenantId,
        limit: &Limit,
        reference: &str,
    ) -> Result<(), Error> {
        match self.provider_for(tenant) {
            Some(provider) => provider.confirm(tenant, limit, reference).await,
            None => Ok(()),
        }
    }

    /// Xem [`Entitlements::release`]; không có bên cung cấp thì `Ok`.
    pub async fn release(
        &self,
        tenant: TenantId,
        limit: &Limit,
        reference: &str,
    ) -> Result<(), Error> {
        match self.provider_for(tenant) {
            Some(provider) => provider.release(tenant, limit, reference).await,
            None => Ok(()),
        }
    }

    /// Xem [`Entitlements::record_usage`]; không có bên cung cấp thì `Ok`.
    pub async fn record_usage(
        &self,
        tenant: TenantId,
        limit: &Limit,
        quantity: i64,
        reference: &str,
    ) -> Result<(), Error> {
        match self.provider_for(tenant) {
            Some(provider) => provider.record_usage(tenant, limit, quantity, reference).await,
            None => Ok(()),
        }
    }
}

/// Một hạn mức trong sổ, kèm module khai nó.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogLimit {
    pub module: &'static str,
    pub limit: Limit,
}

/// Một tính năng trong sổ, kèm module khai nó.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatalogFeature {
    pub module: &'static str,
    pub feature: Feature,
}

#[derive(Default)]
struct CatalogData {
    modules: Vec<&'static str>,
    always_enabled: BTreeSet<&'static str>,
    features: BTreeMap<&'static str, CatalogFeature>,
    limits: BTreeMap<&'static str, CatalogLimit>,
}

/// Sổ tính năng, hạn mức, module bật được của mọi module — dựng lúc đăng ký (khai báo tĩnh), phần
/// `requires` và bộ đếm điền sau khi mọi module khởi động. Clone rẻ.
#[derive(Clone, Default)]
pub struct EntitlementCatalog {
    data: Arc<CatalogData>,
    // Phần điền sau `start`.
    late: Arc<OnceLock<Late>>,
}

#[derive(Default)]
struct Late {
    /// `(module, module nó cần)`: ghi lúc module gọi `StartContext::require`.
    requires: BTreeSet<(&'static str, &'static str)>,
    counters: BTreeMap<&'static str, Arc<dyn UsageCounter>>,
}

impl EntitlementCatalog {
    /// Dựng sổ từ khai báo của các module; trả kèm các lỗi khai báo (khoá không mang tiền tố của
    /// module, trùng khoá, khoá sai dạng).
    pub(crate) fn build(
        declared: Vec<(&'static str, bool, Vec<Feature>, Vec<Limit>)>,
    ) -> (Self, Vec<String>) {
        let mut problems = Vec::new();
        let mut data = CatalogData::default();
        for (module, always, features, limits) in declared {
            data.modules.push(module);
            if always {
                data.always_enabled.insert(module);
            }
            for feature in features {
                problems.extend(key_problem(module, feature.key(), "feature"));
                if data.features.insert(feature.key(), CatalogFeature { module, feature }).is_some()
                {
                    problems.push(format!("feature `{}` is declared twice", feature.key()));
                }
            }
            for limit in limits {
                problems.extend(key_problem(module, limit.key(), "limit"));
                if data.limits.insert(limit.key(), CatalogLimit { module, limit }).is_some() {
                    problems.push(format!("limit `{}` is declared twice", limit.key()));
                }
            }
        }
        // Hai module nội bộ của kernel luôn bật.
        data.always_enabled.extend([crate::builtin::NAME, crate::antispam::module::NAME]);
        (EntitlementCatalog { data: Arc::new(data), late: Arc::default() }, problems)
    }

    pub(crate) fn fill(
        &self,
        requires: BTreeSet<(&'static str, &'static str)>,
        counters: BTreeMap<&'static str, Arc<dyn UsageCounter>>,
    ) {
        let _ = self.late.set(Late { requires, counters });
    }

    /// Mọi module đã đăng ký, theo thứ tự.
    pub fn modules(&self) -> &[&'static str] {
        &self.data.modules
    }

    /// Module không bao giờ bị gói chặn.
    pub fn is_always_enabled(&self, module: &str) -> bool {
        self.data.always_enabled.contains(module)
    }

    pub fn features(&self) -> impl Iterator<Item = &CatalogFeature> {
        self.data.features.values()
    }

    pub fn feature(&self, key: &str) -> Option<&CatalogFeature> {
        self.data.features.get(key)
    }

    pub fn limits(&self) -> impl Iterator<Item = &CatalogLimit> {
        self.data.limits.values()
    }

    pub fn limit(&self, key: &str) -> Option<&CatalogLimit> {
        self.data.limits.get(key)
    }

    /// Module mà `module` cần (đã `require` trait của chúng lúc khởi động). Rỗng trước khi mọi
    /// module khởi động.
    pub fn requires(&self, module: &str) -> Vec<&'static str> {
        self.late
            .get()
            .map(|late| {
                late.requires
                    .iter()
                    .filter(|(who, _)| *who == module)
                    .map(|(_, needed)| *needed)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Bộ đếm thật của hạn mức `key`, nếu module đã đăng ký.
    pub fn counter(&self, key: &str) -> Option<Arc<dyn UsageCounter>> {
        self.late.get().and_then(|late| late.counters.get(key).cloned())
    }
}

/// Lỗi khai báo khi `key` không có dạng `<module>.<tên>`.
fn key_problem(module: &str, key: &str, what: &str) -> Option<String> {
    let owned = key.strip_prefix(module).and_then(|rest| rest.strip_prefix('.'));
    let well_formed = owned
        .is_some_and(|rest| !rest.is_empty() && rest.split('.').all(crate::module::is_identifier));
    (!well_formed).then(|| {
        format!(
            "module `{module}` declares {what} `{key}`; it must be named `{module}.<name>` \
             (lowercase letters, digits, `_`, parts separated by `.`)"
        )
    })
}

/// `module_not_enabled` (403): gói của khách chưa có module này.
pub fn module_not_enabled(module: &str) -> Error {
    Error::forbidden(
        "module_not_enabled",
        "Your current plan does not include the module `{module}`. Upgrade the plan to use it.",
    )
    .with_param("module", module.to_owned())
}

/// `feature_not_enabled` (403): gói của khách chưa có tính năng này.
pub fn feature_not_enabled(feature: &str) -> Error {
    Error::forbidden(
        "feature_not_enabled",
        "Your current plan does not include the feature `{feature}`.",
    )
    .with_param("feature", feature.to_owned())
}

/// `limit_exceeded` (403): đã dùng hết hạn mức.
pub fn limit_exceeded(limit: &str, used: i64, max: i64) -> Error {
    metrics::counter!("entitlement_denials_total", "kind" => "limit").increment(1);
    Error::forbidden(
        "limit_exceeded",
        "The limit `{limit}` is used up ({used}/{max}). Upgrade the plan or free some up.",
    )
    .with_param("limit", limit.to_owned())
    .with_param("used", used)
    .with_param("max", max)
}

/// `quota_exhausted` (429): đã dùng hết lượng của kỳ này; kỳ mới sau `retry_after` giây.
pub fn quota_exhausted(limit: &str, retry_after: u64) -> Error {
    metrics::counter!("entitlement_denials_total", "kind" => "quota").increment(1);
    Error::too_many_requests(
        "quota_exhausted",
        "The `{limit}` of this period is used up; a new period starts in {retry_after} seconds.",
    )
    .with_param("limit", limit.to_owned())
    .with_param("retry_after", retry_after)
    .with_retry_after(retry_after)
}

/// `reference_reused` (409): cùng mã tham chiếu đã giữ chỗ cho một lượng khác.
pub fn reference_reused(reference: &str, limit: &str) -> Error {
    Error::conflict(
        "reference_reused",
        "The reference `{reference}` was already used for another amount of `{limit}`.",
    )
    .with_param("reference", reference.to_owned())
    .with_param("limit", limit.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHOP_FEATURE: Feature = Feature::new("shop.coupons", "Coupons.");
    const SHOP_LIMIT: Limit = Limit::count("shop.products", "Products.");

    #[test]
    fn khoa_phai_mang_ten_module_va_khong_trung() {
        let (catalog, problems) = EntitlementCatalog::build(vec![
            ("shop", false, vec![SHOP_FEATURE], vec![SHOP_LIMIT]),
            ("blog", true, vec![Feature::new("shop.posts", "x")], vec![Limit::count("blog", "x")]),
            ("news", false, vec![], vec![SHOP_LIMIT]),
        ]);
        assert_eq!(problems.len(), 4, "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("`shop.posts`")));
        assert!(problems.iter().any(|p| p.contains("`blog`")));
        assert!(problems.iter().any(|p| p.contains("declared twice")));
        assert!(catalog.is_always_enabled("blog"));
        assert!(catalog.is_always_enabled("kernel"));
        assert!(!catalog.is_always_enabled("shop"));
        assert_eq!(catalog.feature("shop.coupons").unwrap().module, "shop");
    }

    #[test]
    fn quyen_dung_khong_gioi_han_va_co_gioi_han() {
        let open = TenantEntitlements::unrestricted();
        assert!(open.has_module("shop") && open.has_feature("shop.coupons"));
        assert_eq!(open.limit("shop.products"), None);
        let mut limited = TenantEntitlements::default();
        limited.modules.insert("shop".to_owned());
        limited.limits.insert("shop.products".to_owned(), Some(10));
        limited.limits.insert("shop.orders".to_owned(), None);
        assert!(limited.has_module("shop") && !limited.has_module("blog"));
        assert!(!limited.has_feature("shop.coupons"));
        assert_eq!(limited.limit("shop.products"), Some(10));
        assert_eq!(limited.limit("shop.orders"), None);
        assert_eq!(limited.limit("shop.unknown"), Some(0));
    }

    #[tokio::test]
    async fn khong_co_ben_cung_cap_thi_khong_gioi_han() {
        let (catalog, _) = EntitlementCatalog::build(vec![("shop", false, vec![], vec![])]);
        let handle = EntitlementsHandle::new(catalog);
        let tenant = TenantId::from_uuid(uuid::Uuid::now_v7());
        assert!(!handle.is_enforced());
        assert!(handle.module_enabled(tenant, "shop").await.unwrap());
        handle.require_feature(tenant, &SHOP_FEATURE).await.unwrap();
        handle.reserve(tenant, &SHOP_LIMIT, 5, "shop.product:1").await.unwrap();
    }
}
