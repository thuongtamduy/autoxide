//! Bộ vị trí chuẩn (D-236): gieo cho khách mới (`Module::seed_tenant`) chỉ khi khách chưa có vị trí
//! nào, trong một transaction — chạy lại không trùng, không gieo lại vị trí khách đã xoá. Khách có
//! sẵn thêm phần còn thiếu bằng `POST /placements/presets`.

use autoxide_kernel::access::Principal;
use autoxide_kernel::error::Error;
use autoxide_kernel::tenancy::{ScopeExt, TenantContext, TenantSeed};
use sea_orm::{EntityTrait, IntoActiveModel, PaginatorTrait};
use serde_json::{Map, Value, json};
use std::sync::Arc;
use uuid::Uuid;

use super::placements::{Placement, PlacementInput, checked};
use crate::entity::placement;
use crate::permissions::CONTENT_MANAGE;
use crate::service::{ContentService, now};

/// (khoá, tên, kiểu, cài đặt, nhận, tỉ lệ, tỉ lệ điện thoại, cỡ tối thiểu, loại trang)
type Preset = (
    &'static str,
    &'static str,
    &'static str,
    Value,
    &'static str,
    Option<(i32, i32)>,
    Option<(i32, i32)>,
    Option<(i32, i32)>,
    &'static [&'static str],
);

fn presets() -> Vec<Preset> {
    const SIDEBAR: &[&str] = &["category", "collection", "search", "content_entry"];
    vec![
        ("top_strip", "Top strip", "strip", json!({}), "text", None, None, None, &["any"]),
        (
            "home_hero",
            "Home hero",
            "carousel",
            json!({"autoplay_seconds": 6}),
            "image",
            Some((3, 1)),
            Some((4, 5)),
            Some((1800, 600)),
            &["home"],
        ),
        (
            "home_middle",
            "Home middle",
            "grid",
            json!({"columns": 3}),
            "image",
            Some((1, 1)),
            None,
            Some((600, 600)),
            &["home"],
        ),
        (
            "category_top",
            "Top of category",
            "single",
            json!({}),
            "image",
            Some((4, 1)),
            None,
            Some((1600, 400)),
            &["category", "collection"],
        ),
        (
            "sidebar_left",
            "Left column",
            "stack",
            json!({}),
            "image",
            Some((1, 2)),
            None,
            Some((300, 600)),
            SIDEBAR,
        ),
        (
            "sidebar_right",
            "Right column",
            "stack",
            json!({}),
            "image",
            Some((1, 2)),
            None,
            Some((300, 600)),
            SIDEBAR,
        ),
        (
            "inline",
            "Within content",
            "single",
            json!({}),
            "any",
            Some((4, 1)),
            None,
            Some((1200, 300)),
            &["content_entry", "product", "landing"],
        ),
        (
            "above_footer",
            "Above footer",
            "single",
            json!({}),
            "image",
            Some((4, 1)),
            None,
            Some((1600, 400)),
            &["any"],
        ),
        (
            "popup",
            "Popup",
            "popup",
            json!({"frequency": "once_per_day", "delay_seconds": 5}),
            "image",
            Some((4, 5)),
            None,
            Some((600, 750)),
            &["home", "category", "content_entry"],
        ),
    ]
}

/// Dòng của bộ vị trí chuẩn cho khách `tenant`.
fn rows(tenant: &TenantContext) -> Result<Vec<Placement>, Error> {
    let at = now();
    presets()
        .into_iter()
        .map(|(key, name, display, settings, accepts, ratio, mobile_ratio, min, kinds)| {
            let input = PlacementInput {
                key: key.to_owned(),
                name: name.to_owned(),
                description: String::new(),
                max_items: None,
                display: display.to_owned(),
                display_settings: settings.as_object().cloned().unwrap_or_else(Map::new),
                accepts: Some(accepts.to_owned()),
                ratio,
                mobile_ratio,
                min_width: min.map(|min| min.0),
                min_height: min.map(|min| min.1),
                devices: None,
                page_kinds: kinds.iter().map(|kind| (*kind).to_owned()).collect(),
                active: None,
            };
            let row = checked(tenant, input, Uuid::now_v7(), at, None)?;
            Ok(Placement { preset_key: Some(key.to_owned()), ..row })
        })
        .collect()
}

impl ContentService {
    /// Thêm các vị trí chuẩn còn thiếu (theo khoá); trả những vị trí vừa thêm. Cần `content.manage`.
    pub(crate) async fn install_presets(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
    ) -> Result<Vec<Placement>, Error> {
        principal.require(&CONTENT_MANAGE)?;
        let transaction = tenant.database().begin_write().await?;
        let keys: Vec<String> = placement::Entity::find()
            .scoped(tenant)
            .all(&transaction)
            .await?
            .into_iter()
            .map(|row| row.key)
            .collect();
        let missing: Vec<Placement> =
            rows(tenant)?.into_iter().filter(|row| !keys.contains(&row.key)).collect();
        if !missing.is_empty() {
            placement::Entity::insert_many(
                missing.iter().cloned().map(IntoActiveModel::into_active_model),
            )
            .exec(&transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(missing)
    }
}

/// Bộ gieo cho khách mới: chỉ khi khách chưa có vị trí nào.
pub(crate) struct Seed;

#[async_trait::async_trait]
impl TenantSeed for Seed {
    async fn seed(&self, tenant: &TenantContext) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        let existing = placement::Entity::find().scoped(tenant).count(&transaction).await?;
        if existing == 0 {
            placement::Entity::insert_many(
                rows(tenant)?.into_iter().map(IntoActiveModel::into_active_model),
            )
            .exec(&transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }
}

/// Bộ gieo dưới dạng `Arc` cho `Module::seed_tenant`.
pub(crate) fn seed() -> Arc<dyn TenantSeed> {
    Arc::new(Seed)
}
