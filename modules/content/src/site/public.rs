//! `GET /public/site` (bước 67, D-234, D-20261122-public-site): MỘT lời gọi cho phần khung của mọi trang
//! — token hiệu lực (theme gốc ghép bản chủ đề đang chạy), hiệu ứng, đầu trang, chân trang (bố cục kèm
//! dữ liệu khối), chân trang pháp lý, phần của module khác.
//!
//! Không thêm việc cho đường nóng: kết quả tính một lần, giữ trong bộ nhớ theo (khách, website, ngôn ngữ
//! hỏi) tới mốc đổi chủ đề kế tiếp hoặc tối đa [`CACHE_SECONDS`], cái nào tới trước. Cache nóng: không
//! một câu truy vấn nào. Sửa theme / bản chủ đề / khu cố định trên máy này xoá cache ngay; máy khác thấy
//! chậm tối đa 30 giây. `Cache-Control: max-age` = số giây tới mốc đổi, trần [`MAX_AGE_CAP`] — CDN và
//! trình duyệt tự hết hạn đúng lúc chủ đề đổi.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use autoxide_kernel::blocks::{BlockRef, BlockRequest};
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{ScopeExt, Site, TenantContext};
use autoxide_kernel::time::{SignedDuration, Timestamp};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::{BASE, REGIONS, SiteRegion};
use crate::entity::site_region;
use crate::entries::layout;
use crate::service::ContentService;

/// Giữ khung đã tính tối đa chừng này giây (máy khác thấy thay đổi chậm tối đa chừng này).
pub(crate) const CACHE_SECONDS: i64 = 30;
/// Trần của `Cache-Control: max-age`.
pub(crate) const MAX_AGE_CAP: i64 = 300;
/// Khối chân trang pháp lý của `compliance` (tự gắn dưới khu `footer`, ngoài bố cục).
const LEGAL_FOOTER: &str = "compliance.legal_footer";
const LEGAL_FOOTER_ID: &str = "$legal_footer";
/// Giới hạn số dòng trong cache của một tiến trình (khách × website × ngôn ngữ).
const CACHE_ENTRIES_MAX: usize = 10_000;

/// Khung website đã tính.
#[derive(Debug)]
pub(crate) struct PublicSite {
    /// `data` của câu trả lời.
    pub body: Value,
    pub etag: String,
    pub next_change_at: Option<Timestamp>,
}

impl PublicSite {
    /// Số giây cho `Cache-Control: max-age` lúc `now`: tới mốc đổi chủ đề, trần [`MAX_AGE_CAP`].
    pub(crate) fn max_age(&self, now: Timestamp) -> i64 {
        match self.next_change_at {
            Some(next) => (next.as_unix_seconds() - now.as_unix_seconds()).clamp(0, MAX_AGE_CAP),
            None => MAX_AGE_CAP,
        }
    }
}

type Key = (Uuid, Uuid, Option<String>);

struct Cached {
    generation: u64,
    expires_at: Timestamp,
    value: Arc<PublicSite>,
}

/// Cache trong bộ nhớ của khung website.
#[derive(Default)]
pub(crate) struct SiteCache {
    generation: AtomicU64,
    entries: Mutex<HashMap<Key, Cached>>,
}

impl SiteCache {
    /// Mọi khung đã tính hết hiệu lực (gọi khi sửa theme, bản chủ đề, khu cố định).
    pub(crate) fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    fn get(&self, key: &Key, now: Timestamp) -> Option<Arc<PublicSite>> {
        let generation = self.generation.load(Ordering::SeqCst);
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries
            .get(key)
            .filter(|cached| cached.generation == generation && cached.expires_at > now)
            .map(|cached| cached.value.clone())
    }

    fn put(&self, key: Key, generation: u64, expires_at: Timestamp, value: Arc<PublicSite>) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if entries.len() >= CACHE_ENTRIES_MAX {
            // Bản hết hạn hay đời cũ bị dọn trước; vẫn đầy thì bỏ hết (tính lại khi cần).
            let current = self.generation.load(Ordering::SeqCst);
            entries
                .retain(|_, cached| cached.generation == current && cached.expires_at > expires_at);
            if entries.len() >= CACHE_ENTRIES_MAX {
                entries.clear();
            }
        }
        entries.insert(key, Cached { generation, expires_at, value });
    }
}

impl ContentService {
    /// Khung website `site` ở ngôn ngữ `locale` (không gửi: mặc định của website) lúc `now`.
    pub(crate) async fn public_site(
        &self,
        tenant: &TenantContext,
        site: &Site,
        locale: Option<&str>,
        now: Timestamp,
    ) -> Result<Arc<PublicSite>, Error> {
        let key = (tenant.id().as_uuid(), site.org().as_uuid(), locale.map(str::to_owned));
        if let Some(found) = self.site_cache.get(&key, now) {
            return Ok(found);
        }
        // Đời của cache đọc TRƯỚC khi tính: sửa xen giữa lúc tính thì bản này không được dùng lại.
        let generation = self.site_cache.generation.load(Ordering::SeqCst);
        let computed = Arc::new(self.compute_site(tenant, site, locale, now).await?);
        let mut expires_at =
            now.checked_add(SignedDuration::from_secs(CACHE_SECONDS)).unwrap_or(now);
        if let Some(next) = computed.next_change_at {
            expires_at = expires_at.min(next);
        }
        self.site_cache.put(key, generation, expires_at, computed.clone());
        Ok(computed)
    }

    async fn compute_site(
        &self,
        tenant: &TenantContext,
        site: &Site,
        locale: Option<&str>,
        now: Timestamp,
    ) -> Result<PublicSite, Error> {
        let languages = self.site_languages(tenant, site).await?;
        let mut problems = Vec::new();
        let locale = self.check_locale(&languages, locale, "locale", &mut problems);
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let theme = self.site_theme(tenant, site, now, None).await?;
        let rows = site_region::Entity::find()
            .scoped(tenant)
            .filter(site_region::Column::OrgId.is_in(site.lineage_ids()))
            .filter(site_region::Column::Locale.is_in([locale.clone(), languages.default.clone()]))
            .filter(site_region::Column::Layout.is_not_null())
            .all(tenant.database().read())
            .await?;
        let variant = theme.variant.as_ref().map(|row| row.id);
        let usable = self.usable_blocks(tenant).await?;
        let mut layouts: Vec<(&str, Value)> = Vec::new();
        for region in REGIONS {
            if let Some(row) = pick(&rows, site, region, variant, &locale, &languages.default) {
                let parsed: Value = row
                    .layout
                    .as_deref()
                    .and_then(|text| serde_json::from_str(text).ok())
                    .unwrap_or_else(layout::empty);
                layouts.push((region, layout::public(&parsed, &usable)));
            }
        }
        // Mọi khối cần dữ liệu của hai khu trong MỘT lần gọi sổ khối: `id` mang tiền tố khu để không trùng.
        let mut refs: Vec<(&str, BlockRef)> = Vec::new();
        for (region, body) in &layouts {
            for (name, id, props) in layout::blocks_of(body) {
                if usable.get(name).is_some_and(|info| info.block.has_data) {
                    refs.push((name, BlockRef::new(format!("{region}/{id}"), props.clone())));
                }
            }
        }
        let footer = usable.contains_key(LEGAL_FOOTER);
        if footer {
            refs.push((LEGAL_FOOTER, BlockRef::new(LEGAL_FOOTER_ID, Value::Object(Map::new()))));
        }
        let request = BlockRequest::new(tenant, site, &locale);
        let mut data = self.blocks.resolve(&request, &refs).await;
        let legal_footer = data.remove(LEGAL_FOOTER_ID).filter(|value| footer && !value.is_null());
        let mut regions: BTreeMap<&str, Value> = BTreeMap::new();
        for (region, body) in layouts {
            let prefix = format!("{region}/");
            let blocks_data: Map<String, Value> = data
                .iter()
                .filter_map(|(id, value)| {
                    Some((id.strip_prefix(&prefix)?.to_owned(), value.clone()))
                })
                .collect();
            regions.insert(region, json!({"layout": body, "blocks_data": blocks_data}));
        }
        let mut extensions = Map::new();
        for (module, extension) in self.site_extensions.all() {
            match extension.site(&request).await {
                Ok(Some(value)) => {
                    extensions.insert(module.to_owned(), value);
                }
                Ok(None) => {}
                // Phần của một module lỗi không làm hỏng khung website.
                Err(error) => tracing::warn!(module, %error, "website part of a module failed"),
            }
        }
        let body = json!({
            "locale": locale,
            "tokens": theme.tokens,
            "effect": theme.effect,
            "effect_intensity": theme.effect_intensity,
            "variant": theme.variant.as_ref().map(|row| json!({"id": row.id, "name": row.name})),
            "next_change_at": theme.next_change_at,
            "header": regions.remove("header"),
            "footer": regions.remove("footer"),
            "legal_footer": legal_footer,
            "extensions": extensions,
        });
        Ok(PublicSite {
            etag: fingerprint(tenant, &body),
            body,
            next_change_at: theme.next_change_at,
        })
    }
}

/// Khu `region` cho website: ô của bản chủ đề đang chạy trước, rồi ô của theme gốc; trong mỗi ô, nút
/// gần website nhất trước (cửa hàng đè chuỗi); trong mỗi nút, ngôn ngữ hỏi rồi ngôn ngữ mặc định.
fn pick<'a>(
    rows: &'a [SiteRegion],
    site: &Site,
    region: &str,
    variant: Option<Uuid>,
    locale: &str,
    default: &str,
) -> Option<&'a SiteRegion> {
    let slots: Vec<String> =
        variant.map(|id| id.to_string()).into_iter().chain([BASE.to_owned()]).collect();
    for slot in &slots {
        for org in site.lineage() {
            for wanted in [locale, default] {
                let found = rows.iter().find(|row| {
                    row.region == region
                        && row.variant_slot == *slot
                        && row.org_id == org.as_uuid()
                        && row.locale == wanted
                });
                if found.is_some() {
                    return found;
                }
            }
        }
    }
    None
}

fn fingerprint(tenant: &TenantContext, value: &Value) -> String {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(tenant.id().as_uuid().as_bytes());
    hasher.update(value.to_string().as_bytes());
    let digest = hasher.finalize();
    let short = digest.get(..16).unwrap_or_default();
    format!("\"{}\"", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(short))
}
