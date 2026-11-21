//! Đọc công khai các vị trí của một trang (D-236): một câu cho vị trí, một câu cho banner, ảnh theo
//! lô (`Media::find_many_in`), link theo lô mỗi loại một lần — không tăng theo số vị trí.
//!
//! - Banner `active`, đúng ngôn ngữ, của website và mọi nút tổ tiên, trong khung giờ lúc gọi, theo
//!   `position` (rồi ID) — thứ tự cố định.
//! - Lọc theo trang chạy trong bộ nhớ: banner chọn loại trang / trang cụ thể chỉ hiện ở đó; banner
//!   không giới hạn trang luôn có.
//! - Banner có ảnh không còn công khai, hay link tới đối tượng không còn công khai (sản phẩm ngừng
//!   bán, chương trình hết hạn), bị bỏ — không báo lỗi cho người xem.
//! - Cache: `max-age` = min([`MAX_AGE_SECONDS`], số giây tới mốc bắt đầu / kết thúc gần nhất trong
//!   tương lai của mọi vị trí) — CDN không giữ banner đã hết giờ.

use autoxide_kernel::time::Timestamp;
use std::collections::{HashMap, HashSet};

use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::resource::Resource;
use autoxide_kernel::tenancy::{ScopeExt, Site, TenantContext};
use autoxide_media::api::{MediaRef, MediaVisibility};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder};
use uuid::Uuid;

use super::items::{Banner, Link, link_of, page_targets};
use super::placements::Placement;
use super::{MAX_KEYS, PAGE_KINDS, placement_not_found, strings};
use crate::entity::{banner, placement};
use crate::service::ContentService;

/// `max-age` tối đa của đọc công khai: banner đổi thì website thấy sau tối đa chừng này giây.
pub(crate) const MAX_AGE_SECONDS: i64 = 60;

/// Trang đang xem.
#[derive(Debug, Clone, Default)]
pub(crate) struct PageContext {
    /// Loại trang (vd `category`).
    pub kind: Option<String>,
    /// Trang cụ thể: (loại trong sổ đối tượng, ID).
    pub reference: Option<(String, String)>,
    /// Ngôn ngữ của trang; không có: ngôn ngữ mặc định của website.
    pub locale: Option<String>,
}

/// Một banner đã giải.
#[derive(Debug, Clone)]
pub(crate) struct ReadyBanner {
    pub row: Banner,
    pub media: Option<MediaRef>,
    pub mobile_media: Option<MediaRef>,
    /// Đường dẫn của link (địa chỉ, hay đường dẫn công khai của đối tượng).
    pub href: Option<String>,
}

/// Một vị trí đã giải.
#[derive(Debug, Clone)]
pub(crate) struct ReadyPlacement {
    pub placement: Placement,
    pub banners: Vec<ReadyBanner>,
}

/// Kết quả đọc công khai.
#[derive(Debug, Clone)]
pub(crate) struct PublicPlacements {
    pub locale: String,
    pub placements: Vec<ReadyPlacement>,
    /// Số giây được cache.
    pub max_age: i64,
}

impl ContentService {
    /// Các vị trí `keys` (tối đa [`MAX_KEYS`]) của website `site` ở ngôn ngữ `locale` lúc `now`.
    /// Khoá không có (hay vị trí đang tắt) thì vắng; `strict`: khi đó báo 404 (đọc một vị trí).
    pub(crate) async fn public_placements(
        &self,
        tenant: &TenantContext,
        site: &Site,
        keys: &[String],
        page: &PageContext,
        now: Timestamp,
        strict: bool,
    ) -> Result<PublicPlacements, Error> {
        let mut problems = Vec::new();
        let mut wanted: Vec<String> = Vec::new();
        for key in keys.iter().map(|key| key.trim()).filter(|key| !key.is_empty()) {
            if !wanted.iter().any(|known| known == key) {
                wanted.push(key.to_owned());
            }
        }
        if wanted.is_empty() {
            problems.push(FieldError::new("keys", "required", "name at least one placement"));
        }
        if wanted.len() > MAX_KEYS {
            return Err(Error::new(
                ErrorKind::Validation,
                "content.placement_keys_too_many",
                "Read at most {max} placements at once.",
            )
            .with_param("max", MAX_KEYS));
        }
        if let Some(kind) = &page.kind
            && !PAGE_KINDS.contains(&kind.as_str())
        {
            problems.push(
                FieldError::new("page_kind", "invalid", "must be one of {allowed}")
                    .with_param("allowed", PAGE_KINDS.join(", ")),
            );
        }
        let languages = self.site_languages(tenant, site).await?;
        let locale = self.check_locale(&languages, page.locale.as_deref(), "locale", &mut problems);
        if !problems.is_empty() {
            return Err(Error::validation(problems));
        }
        let read = tenant.database().read();
        let mut found = placement::Entity::find()
            .scoped(tenant)
            .filter(placement::Column::Key.is_in(wanted.clone()))
            .filter(placement::Column::Active.eq(true))
            .filter(placement::Column::Audience.eq("site"))
            .all(read)
            .await?;
        if strict && found.is_empty() {
            return Err(placement_not_found(wanted.first().map_or("", String::as_str)));
        }
        // Theo thứ tự người gọi nêu.
        found.sort_by_key(|row| wanted.iter().position(|key| *key == row.key));
        let ids: Vec<Uuid> = found.iter().map(|row| row.id).collect();
        let lineage = site.lineage_ids();
        let rows = if ids.is_empty() {
            Vec::new()
        } else {
            banner::Entity::find()
                .scoped(tenant)
                .filter(banner::Column::PlacementId.is_in(ids))
                .filter(banner::Column::Status.eq("active"))
                .filter(banner::Column::Locale.eq(locale.clone()))
                .filter(banner::Column::OrgId.is_in(lineage))
                .filter(
                    Condition::any()
                        .add(banner::Column::EndsAt.is_null())
                        .add(banner::Column::EndsAt.gt(now)),
                )
                .order_by_asc(banner::Column::Position)
                .order_by_asc(banner::Column::Id)
                .all(read)
                .await?
        };
        // Mốc gần nhất trong tương lai: banner sắp bắt đầu hay sắp kết thúc.
        let next = rows
            .iter()
            .flat_map(|row| [row.starts_at, row.ends_at])
            .flatten()
            .filter(|at| *at > now)
            .min();
        let max_age =
            next.map_or(MAX_AGE_SECONDS, |at| (at - now).as_secs().clamp(0, MAX_AGE_SECONDS));
        let showing: Vec<Banner> = rows
            .into_iter()
            .filter(|row| row.starts_at.is_none_or(|at| at <= now))
            .filter(|row| on_page(row, page))
            .collect();
        let resolved = self.resolve_banners(tenant, &locale, showing).await?;
        let mut by_placement: HashMap<Uuid, Vec<ReadyBanner>> = HashMap::new();
        for ready in resolved {
            by_placement.entry(ready.row.placement_id).or_default().push(ready);
        }
        let placements = found
            .into_iter()
            .map(|placement| {
                let mut banners = by_placement.remove(&placement.id).unwrap_or_default();
                // Vị trí không dùng ở loại trang này: không banner nào.
                if let Some(kind) = &page.kind
                    && !strings(&placement.page_kinds)
                        .iter()
                        .any(|known| known == "any" || known == kind)
                {
                    banners.clear();
                }
                banners.truncate(usize::try_from(placement.max_items).unwrap_or(0));
                ReadyPlacement { placement, banners }
            })
            .collect();
        Ok(PublicPlacements { locale, placements, max_age })
    }

    /// Ảnh và link của banner, theo lô; bỏ banner mà ảnh hay đối tượng đích không còn công khai.
    async fn resolve_banners(
        &self,
        tenant: &TenantContext,
        locale: &str,
        rows: Vec<Banner>,
    ) -> Result<Vec<ReadyBanner>, Error> {
        let image_ids: Vec<Uuid> = rows
            .iter()
            .flat_map(|row| [row.media_id, row.mobile_media_id])
            .flatten()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let images: HashMap<Uuid, MediaRef> = if image_ids.is_empty() {
            HashMap::new()
        } else {
            self.media
                .find_many_in(tenant, &image_ids, locale)
                .await?
                .into_iter()
                .filter(|item| item.visibility == MediaVisibility::Public)
                .map(|item| (item.id, item))
                .collect()
        };
        let mut wanted: HashMap<String, Vec<String>> = HashMap::new();
        for row in &rows {
            if let Link::Ref { kind, id } = link_of(row) {
                wanted.entry(kind).or_default().push(id);
            }
        }
        let mut objects: HashMap<(String, String), Resource> = HashMap::new();
        for (kind, mut ids) in wanted {
            // Loại không còn module nào cung cấp: banner trỏ tới nó bị bỏ.
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
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let media = match row.media_id {
                    Some(id) => Some(images.get(&id)?.clone()),
                    None => None,
                };
                let mobile_media = row.mobile_media_id.and_then(|id| images.get(&id).cloned());
                let href = match link_of(&row) {
                    Link::None => None,
                    Link::Url(url) => Some(url),
                    Link::Ref { kind, id } => objects.get(&(kind, id))?.url.clone(),
                };
                Some(ReadyBanner { row, media, mobile_media, href })
            })
            .collect())
    }
}

/// Banner có hiện ở trang `page` không.
fn on_page(row: &Banner, page: &PageContext) -> bool {
    let kinds = strings(&row.page_kinds);
    if !kinds.is_empty() && !page.kind.as_ref().is_some_and(|kind| kinds.contains(kind)) {
        return false;
    }
    let targets = page_targets(&row.page_targets);
    if targets.is_empty() {
        return true;
    }
    page.reference.as_ref().is_some_and(|(kind, id)| {
        targets.iter().any(|target| target.kind == *kind && target.id == *id)
    })
}
