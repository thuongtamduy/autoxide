//! Menu công khai: cây mục đã giải cho website (D-238).
//!
//! - Mục trỏ tới đối tượng: tiêu đề, đường dẫn lấy từ sổ đối tượng theo ngôn ngữ của menu; đối
//!   tượng không còn công khai (hay loại không còn module nào cung cấp) thì bỏ, kèm mục con.
//! - Nhóm tiêu đề không còn mục con nào thì bỏ.
//! - `auto_children`: mở bằng `children` của sổ đối tượng, có giới hạn.
//! - Nhãn nổi bật ngoài hạn thì ẩn.
//! - Thiếu menu ở ngôn ngữ hỏi thì dùng ngôn ngữ mặc định.
//!
//! **Cache:** bản đã giải giữ trong bộ nhớ tới `AUTOXIDE_MENUS_PUBLIC_CACHE_SECONDS`, rút ngắn tới
//! mốc bật / tắt gần nhất của nhãn nổi bật; lệnh ghi của module xoá cache **của khách đó**. Đối
//! tượng đổi tên hay đổi đường dẫn thì hiện sau tối đa một thời hạn cache — không nghe event của
//! module khác.
//!
//! **Theo khách (bước 33, D-238):** khoá cache gồm khách, nên hai khách cùng (`key`, `locale`) không
//! bao giờ nhận menu của nhau; mỗi khách một bộ đếm thế hệ, nên lệnh ghi của khách này không làm
//! mất cache của khách khác. `ETag` tính cả ID khách.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use autoxide_kernel::error::Error;
use autoxide_kernel::resource::{Resource, ResourceRef};
use autoxide_kernel::response::Data;
use autoxide_kernel::tenancy::{Site, TenantContext, TenantId};
use autoxide_media::api::{MediaRef, MediaVisibility};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::entity::item;
use crate::items::{Node, tree};
use crate::service::MenuService;

/// A website menu ready to show: links worked out, hidden targets left out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct PublicSiteMenu {
    #[schema(example = "main")]
    pub key: String,
    /// The language actually returned (the default language when the menu is not translated).
    #[schema(example = "vi")]
    pub locale: String,
    pub items: Vec<PublicSiteMenuItem>,
}

/// An item of a public menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct PublicSiteMenuItem {
    /// The label set on the item, or else the title of the object it points to.
    pub label: String,
    /// Where the item links to: a path on the website or an address. `null` for a heading that
    /// only groups sub-items (e.g. a column of a mega menu).
    pub href: Option<String>,
    /// The object the item points to, for websites that fetch more about it (e.g. best sellers
    /// of a product category).
    #[serde(rename = "ref")]
    pub target: Option<PublicMenuTarget>,
    /// Name of an icon from a fixed set; see the item schema of `PUT /api/v1/menus/{id}`.
    pub icon: Option<String>,
    /// How to set the item apart: `accent`, `brand`, `sale` or `muted`.
    pub highlight: Option<String>,
    /// A small label shown next to the item, only while it is in its date range.
    pub badge: Option<PublicMenuBadge>,
    /// An object to feature in the item's drop-down panel.
    pub feature: Option<PublicMenuFeature>,
    #[schema(no_recursion)]
    pub children: Vec<PublicSiteMenuItem>,
}

/// An object of a module, as a resource type and an id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct PublicMenuTarget {
    #[schema(example = "content.term")]
    pub kind: String,
    pub id: String,
}

/// A small label on a menu item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct PublicMenuBadge {
    #[schema(example = "-50%")]
    pub text: String,
    /// `sale`, `brand`, `new` or `neutral`.
    pub tone: String,
}

/// An object featured in a drop-down panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct PublicMenuFeature {
    #[serde(rename = "ref")]
    pub target: PublicMenuTarget,
    pub title: String,
    pub summary: String,
    pub href: Option<String>,
    /// Its picture, when it has a public one.
    pub image: Option<PublicMenuImage>,
}

/// A public picture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub(crate) struct PublicMenuImage {
    /// Address of the picture; it changes when the picture changes, so it may be cached long.
    pub url: String,
    pub alt: String,
    pub width: Option<i32>,
    pub height: Option<i32>,
}

/// Menu đã giải, sẵn để trả: kèm `ETag` (dấu vân tay của cả câu trả lời) và lúc hết hạn.
#[derive(Debug, Clone)]
pub(crate) struct Ready {
    pub menu: PublicSiteMenu,
    pub etag: String,
    pub expires_at: Timestamp,
}

/// Khoá cache: (khách, website, khoá menu, ngôn ngữ hỏi).
type CacheKey = (TenantId, Uuid, String, String);

/// Cache trong bộ nhớ của tiến trình: (khách, khoá, ngôn ngữ hỏi) → menu đã giải. Khoá chỉ gồm menu
/// có thật và ngôn ngữ được phép; bản hết hạn bị dọn mỗi lần cất bản mới.
#[derive(Default)]
pub(crate) struct PublicCache {
    inner: Mutex<CacheState>,
}

#[derive(Default)]
struct CacheState {
    /// Thế hệ của từng khách, tăng mỗi lần xoá cache của khách đó: bản giải xong sau một lệnh ghi
    /// không được cất lại. Khách chưa xoá lần nào: 0.
    generations: HashMap<TenantId, u64>,
    entries: HashMap<CacheKey, Arc<Ready>>,
}

impl PublicCache {
    /// Xoá cache của khách `tenant` (sau mỗi lệnh ghi của khách đó); khách khác giữ nguyên.
    pub(crate) fn clear(&self, tenant: TenantId) {
        if let Ok(mut state) = self.inner.lock() {
            let generation = state.generations.entry(tenant).or_default();
            *generation = generation.wrapping_add(1);
            state.entries.retain(|(owner, _, _, _), _| *owner != tenant);
        }
    }

    fn get(&self, key: &CacheKey, now: Timestamp) -> (u64, Option<Arc<Ready>>) {
        let Ok(state) = self.inner.lock() else {
            return (0, None);
        };
        let generation = state.generations.get(&key.0).copied().unwrap_or_default();
        let hit = state.entries.get(key).filter(|ready| ready.expires_at > now).cloned();
        (generation, hit)
    }

    fn put(&self, generation: u64, key: CacheKey, ready: Arc<Ready>, now: Timestamp) {
        if let Ok(mut state) = self.inner.lock()
            && state.generations.get(&key.0).copied().unwrap_or_default() == generation
        {
            // Bản hết hạn không bao giờ được dùng lại: dọn để cache không lớn dần theo số khách.
            state.entries.retain(|_, cached| cached.expires_at > now);
            state.entries.insert(key, ready);
        }
    }
}

impl MenuService {
    /// Menu công khai `key` của khách `tenant` ở ngôn ngữ `locale` (không gửi: mặc định), lúc
    /// `now`.
    pub(crate) async fn public_menu(
        &self,
        tenant: &TenantContext,
        site: &Site,
        key: &str,
        locale: Option<&str>,
        now: Timestamp,
    ) -> Result<Arc<Ready>, Error> {
        let mut problems = Vec::new();
        let (locales, default) = self.site_locales(tenant, site).await?;
        let locale = crate::service::check_locale(&locales, &default, locale, &mut problems);
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let cache_key = (tenant.id(), site.org().as_uuid(), key.to_owned(), locale.clone());
        let ttl = i64::from(self.config.public_cache_seconds);
        let (generation, hit) = self.cache.get(&cache_key, now);
        if let Some(hit) = hit {
            return Ok(hit);
        }
        let found = match self.find_by_key(tenant, site, key, &locale).await? {
            Some(found) => found,
            None => self
                .find_by_key(tenant, site, key, &default)
                .await?
                .ok_or_else(|| public_not_found(key, &locale))?,
        };
        let items = self.items_of(tenant.database().read(), tenant, found.id).await?;
        let next_change = next_badge_change(&items, now);
        let resolved = self.resolve(tenant, &tree(items), &found.locale, now).await?;
        let menu = PublicSiteMenu { key: found.key, locale: found.locale, items: resolved };
        let etag = fingerprint(tenant.id(), &Data { data: &menu })?;
        let mut expires_at = now + SignedDuration::from_secs(ttl);
        if let Some(next_change) = next_change {
            expires_at = expires_at.min(next_change);
        }
        let ready = Arc::new(Ready { menu, etag, expires_at });
        if ttl > 0 {
            self.cache.put(generation, cache_key, ready.clone(), now);
        }
        Ok(ready)
    }

    /// Giải cây: hỏi sổ đối tượng theo lô (mỗi loại một lần), mở nhóm con, lấy ảnh của ô gợi ý.
    async fn resolve(
        &self,
        tenant: &TenantContext,
        nodes: &[Node],
        locale: &str,
        now: Timestamp,
    ) -> Result<Vec<PublicSiteMenuItem>, Error> {
        let mut rows = Vec::new();
        collect(nodes, &mut rows);
        let mut wanted: HashMap<String, Vec<String>> = HashMap::new();
        let mut auto: HashMap<Uuid, Vec<ResourceRef>> = HashMap::new();
        for row in &rows {
            if let (Some(kind), Some(id)) = (&row.ref_kind, &row.ref_id) {
                wanted.entry(kind.clone()).or_default().push(id.clone());
                if row.auto_children && self.resources.has_children(kind) {
                    let limit = row
                        .auto_children_limit
                        .and_then(|limit| usize::try_from(limit).ok())
                        .unwrap_or(0);
                    let children =
                        self.resources.children(tenant, kind, id, Some(locale), limit).await?;
                    for child in &children {
                        wanted.entry(child.kind.clone()).or_default().push(child.id.clone());
                    }
                    auto.insert(row.id, children);
                }
            }
            if let (Some(kind), Some(id)) = (&row.feature_kind, &row.feature_id) {
                wanted.entry(kind.clone()).or_default().push(id.clone());
            }
        }
        let mut objects: HashMap<(String, String), Resource> = HashMap::new();
        for (kind, mut ids) in wanted {
            // Loại không còn module nào cung cấp (module bị gỡ): mục trỏ tới nó bị bỏ.
            if !self.resources.knows(&kind) {
                continue;
            }
            ids.sort_unstable();
            ids.dedup();
            let found = self.resources.resolve_many(tenant, &kind, &ids, Some(locale)).await?;
            objects.extend(
                found
                    .into_iter()
                    .filter(|(_, resource)| resource.public)
                    .map(|(id, resource)| ((kind.clone(), id), resource)),
            );
        }
        let image_ids: Vec<Uuid> = rows
            .iter()
            .filter_map(|row| Some((row.feature_kind.clone()?, row.feature_id.clone()?)))
            .filter_map(|key| objects.get(&key)?.image)
            .collect();
        let images: HashMap<Uuid, MediaRef> = if image_ids.is_empty() {
            HashMap::new()
        } else {
            self.media
                .find_many_in(tenant, &image_ids, locale)
                .await?
                .into_iter()
                .filter(|media| media.visibility == MediaVisibility::Public)
                .map(|media| (media.id, media))
                .collect()
        };
        let context = Context { objects: &objects, auto: &auto, images: &images, now };
        Ok(build(nodes, &context))
    }
}

// Tình trạng hiện tại của một đích (màn quản trị, `include=targets`).
/// How the object of a menu item stands now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TargetState {
    Public,
    NotPublic,
    Missing,
    UnknownKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetInfo {
    pub state: TargetState,
    /// Chỉ khi công khai: bộ tra không đưa tiêu đề của đối tượng chưa công khai ra ngoài module.
    pub title: Option<String>,
    pub path: Option<String>,
}

impl MenuService {
    /// Tình trạng đích (`ref`, `feature`) của mọi mục: hỏi sổ đối tượng theo lô, mỗi loại một lần,
    /// theo ngôn ngữ của menu. Người gọi đã qua kiểm `menus.read` (đọc menu trước).
    pub(crate) async fn target_states(
        &self,
        tenant: &TenantContext,
        nodes: &[Node],
        locale: &str,
    ) -> Result<HashMap<(String, String), TargetInfo>, Error> {
        let mut rows = Vec::new();
        collect(nodes, &mut rows);
        let mut wanted: HashMap<String, Vec<String>> = HashMap::new();
        for row in &rows {
            for (kind, id) in [(&row.ref_kind, &row.ref_id), (&row.feature_kind, &row.feature_id)] {
                if let (Some(kind), Some(id)) = (kind, id) {
                    wanted.entry(kind.clone()).or_default().push(id.clone());
                }
            }
        }
        let mut states = HashMap::new();
        for (kind, mut ids) in wanted {
            ids.sort_unstable();
            ids.dedup();
            if !self.resources.knows(&kind) {
                let unknown =
                    TargetInfo { state: TargetState::UnknownKind, title: None, path: None };
                states.extend(ids.into_iter().map(|id| ((kind.clone(), id), unknown.clone())));
                continue;
            }
            let found = self.resources.resolve_many(tenant, &kind, &ids, Some(locale)).await?;
            for id in ids {
                let info = match found.get(&id) {
                    None => TargetInfo { state: TargetState::Missing, title: None, path: None },
                    Some(resource) if resource.public => TargetInfo {
                        state: TargetState::Public,
                        title: Some(resource.title.clone()),
                        path: resource.url.clone(),
                    },
                    Some(_) => {
                        TargetInfo { state: TargetState::NotPublic, title: None, path: None }
                    }
                };
                states.insert((kind.clone(), id), info);
            }
        }
        Ok(states)
    }
}

struct Context<'a> {
    objects: &'a HashMap<(String, String), Resource>,
    auto: &'a HashMap<Uuid, Vec<ResourceRef>>,
    images: &'a HashMap<Uuid, MediaRef>,
    now: Timestamp,
}

fn collect<'a>(nodes: &'a [Node], rows: &mut Vec<&'a item::Model>) {
    for node in nodes {
        rows.push(&node.item);
        collect(&node.children, rows);
    }
}

fn build(nodes: &[Node], context: &Context<'_>) -> Vec<PublicSiteMenuItem> {
    nodes.iter().filter_map(|node| build_one(node, context)).collect()
}

fn build_one(node: &Node, context: &Context<'_>) -> Option<PublicSiteMenuItem> {
    let row = &node.item;
    let mut children = build(&node.children, context);
    let (label, href, target) = match (&row.ref_kind, &row.ref_id, &row.url) {
        (Some(kind), Some(id), _) => {
            let object = context.objects.get(&(kind.clone(), id.clone()))?;
            if let Some(auto) = context.auto.get(&row.id) {
                children.extend(auto.iter().filter_map(|child| {
                    let object = context.objects.get(&(child.kind.clone(), child.id.clone()))?;
                    Some(PublicSiteMenuItem {
                        label: object.title.clone(),
                        href: object.url.clone(),
                        target: Some(PublicMenuTarget {
                            kind: child.kind.clone(),
                            id: child.id.clone(),
                        }),
                        icon: None,
                        highlight: None,
                        badge: None,
                        feature: None,
                        children: Vec::new(),
                    })
                }));
            }
            let label = if row.label.is_empty() { object.title.clone() } else { row.label.clone() };
            (
                label,
                object.url.clone(),
                Some(PublicMenuTarget { kind: kind.clone(), id: id.clone() }),
            )
        }
        (_, _, Some(url)) => (row.label.clone(), Some(url.clone()), None),
        _ => {
            // Nhóm tiêu đề: không còn mục con nào hiện thì bỏ.
            if children.is_empty() {
                return None;
            }
            (row.label.clone(), None, None)
        }
    };
    let badge = match (&row.badge_text, &row.badge_tone) {
        (Some(text), Some(tone))
            if row.badge_from.is_none_or(|from| from <= context.now)
                && row.badge_until.is_none_or(|until| context.now < until) =>
        {
            Some(PublicMenuBadge { text: text.clone(), tone: tone.clone() })
        }
        _ => None,
    };
    let feature = match (&row.feature_kind, &row.feature_id) {
        (Some(kind), Some(id)) => {
            context.objects.get(&(kind.clone(), id.clone())).map(|object| PublicMenuFeature {
                target: PublicMenuTarget { kind: kind.clone(), id: id.clone() },
                title: object.title.clone(),
                summary: object.summary.clone(),
                href: object.url.clone(),
                image: object.image.and_then(|image| context.images.get(&image)).map(|media| {
                    PublicMenuImage {
                        url: media.url.clone(),
                        alt: media.alt.clone(),
                        width: media.width,
                        height: media.height,
                    }
                }),
            })
        }
        _ => None,
    };
    Some(PublicSiteMenuItem {
        label,
        href,
        target,
        icon: row.icon.clone(),
        highlight: row.highlight.clone(),
        badge,
        feature,
        children,
    })
}

/// Mốc gần nhất sau `now` mà một nhãn nổi bật bật hay tắt: cache không được giữ quá mốc đó.
fn next_badge_change(items: &[item::Model], now: Timestamp) -> Option<Timestamp> {
    items
        .iter()
        .filter(|row| row.badge_text.is_some())
        .flat_map(|row| [row.badge_from, row.badge_until])
        .flatten()
        .filter(|at| *at > now)
        .min()
}

/// `ETag` của cả câu trả lời: 16 byte đầu của SHA-256 của ID khách và thân JSON — hai khách có
/// menu giống hệt nhau vẫn nhận hai `ETag` khác nhau.
fn fingerprint<T: Serialize>(tenant: TenantId, value: &T) -> Result<String, Error> {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(value)
        .map_err(|error| Error::internal(format!("cannot serialize the menu: {error}")))?;
    let mut hasher = Sha256::new();
    hasher.update(tenant.as_uuid().as_bytes());
    hasher.update(&bytes);
    let digest = hasher.finalize();
    let short = digest.get(..16).unwrap_or_default();
    Ok(format!("\"{}\"", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(short)))
}

fn public_not_found(key: &str, locale: &str) -> Error {
    Error::not_found("menus.public_menu_not_found", "There is no menu `{key}` in `{locale}`.")
        .with_param("key", key)
        .with_param("locale", locale)
}
