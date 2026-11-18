//! Cài đặt vận chuyển của cửa hàng (bước 44 mục 2): vùng theo tỉnh, phương thức, bậc phí, điểm
//! nhận tại cửa hàng. Mỗi bản ghi thuộc một nút; đọc / ghi kiểm `shipping.settings.manage` ở nút
//! đó (độ sâu theo grant, bước 34). Phương thức của nút trên áp xuống nút dưới (`quote.rs`).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use autoxide_kernel::access::{OrgTarget, Permission, Principal, Scope};
use autoxide_kernel::error::{Error, ErrorKind, FieldError};
use autoxide_kernel::money::Money;
use autoxide_kernel::pagination::{Page, PageRequest, fetch_page};
use autoxide_kernel::tenancy::{OrgId, ScopeExt, TenantContext, root_of};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, EntityTrait, QueryFilter,
    QueryOrder, Set,
};
use uuid::Uuid;

use crate::api::MethodKind;
use crate::entity::{method, pickup_point, rate, zone, zone_region};
use crate::errors;
use crate::permissions::SETTINGS_MANAGE;
use crate::service::{ShippingService, check_length, now};

/// Một vùng: một quốc gia, hay một tỉnh của quốc gia đó.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Region {
    pub country: String,
    /// `None` = cả nước.
    pub province_code: Option<String>,
}

/// Vùng kèm các tỉnh.
#[derive(Debug, Clone)]
pub(crate) struct ZoneView {
    pub zone: zone::Model,
    pub regions: Vec<Region>,
}

/// Các trường của phương thức; tạo mới cần `code`, `kind`, `names`, `currency`, còn lại có mặc
/// định. Sửa: trường `None` giữ nguyên (`code`, `kind`, `org_id`, `currency` không đổi được).
#[derive(Debug, Clone, Default)]
pub(crate) struct MethodFields {
    pub org_id: Option<Uuid>,
    pub code: Option<String>,
    pub kind: Option<MethodKind>,
    pub names: Option<BTreeMap<String, String>>,
    pub active: Option<bool>,
    pub position: Option<i32>,
    pub carrier_account_id: Option<Option<Uuid>>,
    pub service_code: Option<Option<String>>,
    pub currency: Option<String>,
    pub free_over: Option<Option<Money>>,
    pub handling_fee: Option<Money>,
    pub tax_category: Option<String>,
    pub volumetric_divisor: Option<i32>,
    pub default_item_weight_grams: Option<Option<i64>>,
    pub countries: Option<Vec<String>>,
    pub estimated_days: Option<Option<(i16, i16)>>,
    pub calendar_id: Option<Option<Uuid>>,
    pub cutoff_minute: Option<Option<i32>>,
    pub same_day: Option<bool>,
    pub hold_days: Option<Option<i16>>,
    pub auto_book: Option<bool>,
}

/// Một bậc phí gửi lên.
#[derive(Debug, Clone)]
pub(crate) struct RateInput {
    pub zone_id: Uuid,
    pub min_weight_grams: i64,
    pub max_weight_grams: Option<i64>,
    pub max_side_mm: Option<i64>,
    pub amount: Money,
    pub step_grams: Option<i64>,
    pub step_amount: Option<Money>,
}

/// Một điểm nhận tại cửa hàng gửi lên.
#[derive(Debug, Clone)]
pub(crate) struct PickupInput {
    pub org_id: Uuid,
    pub instructions: String,
    pub prep_hours: i32,
}

/// Phương thức kèm bậc phí và điểm nhận.
#[derive(Debug, Clone)]
pub(crate) struct MethodView {
    pub method: method::Model,
    pub rates: Vec<rate::Model>,
    pub pickup_points: Vec<pickup_point::Model>,
}

/// Tên phương thức theo `locale`, không có thì tiếng Việt, tiếng Anh, rồi tên đầu tiên.
pub(crate) fn method_name(names: &str, locale: &str) -> String {
    let names: BTreeMap<String, String> = serde_json::from_str(names).unwrap_or_default();
    let language = locale.split('-').next().unwrap_or(locale);
    [locale, language, "vi", "en"]
        .iter()
        .find_map(|key| names.get(*key))
        .or_else(|| names.values().next())
        .cloned()
        .unwrap_or_default()
}

/// Nút của bản ghi mới khi người tạo không nêu: gốc nếu họ có `permission` cho cả khách; nút
/// duy nhất nếu mọi grant ở một nút; nhiều nút thì phải chọn.
pub(crate) fn default_org(
    tenant: &TenantContext,
    principal: &Principal,
    permission: &Permission,
) -> Result<OrgId, Error> {
    let grants = principal.grants(permission);
    if grants.is_empty() || grants.iter().any(|grant| grant.scope == Scope::Tenant) {
        return Ok(root_of(tenant.id()));
    }
    let first = grants[0].org;
    if grants.iter().all(|grant| grant.org == first) {
        return Ok(first);
    }
    Err(Error::new(
        ErrorKind::Validation,
        "shipping.org_required",
        "You work in several organization units: give `org_id`.",
    )
    .with_field(FieldError::new(
        "org_id",
        "required",
        "you work in several organization units: choose one",
    )))
}

/// Đọc bản ghi thuộc nút `org`: không phủ thì báo như không có.
pub(crate) async fn readable<C: ConnectionTrait>(
    principal: &Principal,
    connection: &C,
    permission: &Permission,
    org: Uuid,
    not_found: impl FnOnce() -> Error,
) -> Result<(), Error> {
    let target = OrgTarget::new(OrgId::from_uuid(org), None);
    match principal.require_at(connection, permission, target).await {
        Err(error) if error.code() == "org_out_of_scope" => Err(not_found()),
        other => other,
    }
}

/// Ghi bản ghi thuộc nút `org` trong transaction: không đọc được → 404; nút khoá → `org_frozen`.
pub(crate) async fn writable<C: ConnectionTrait>(
    principal: &Principal,
    transaction: &C,
    permission: &Permission,
    org: Uuid,
    not_found: impl FnOnce() -> Error,
) -> Result<(), Error> {
    readable(principal, transaction, permission, org, not_found).await?;
    principal
        .require_write_at(transaction, permission, OrgTarget::new(OrgId::from_uuid(org), None))
        .await
}

pub(crate) fn fail_if_any(problems: Vec<FieldError>) -> Result<(), Error> {
    if problems.is_empty() { Ok(()) } else { Err(Error::validation(problems)) }
}

/// Kiểm và chuẩn hoá danh sách vùng.
fn check_regions(problems: &mut Vec<FieldError>, regions: &[Region]) -> Vec<Region> {
    let mut out = BTreeSet::new();
    for (index, region) in regions.iter().enumerate() {
        let country = region.country.trim().to_ascii_uppercase();
        if country.len() != 2 || !country.bytes().all(|byte| byte.is_ascii_uppercase()) {
            problems.push(FieldError::new(
                format!("regions[{index}].country"),
                "country",
                "must be a two-letter ISO 3166-1 country code like `VN`",
            ));
            continue;
        }
        let province =
            region.province_code.as_deref().map(str::trim).filter(|code| !code.is_empty());
        if let Some(code) = province
            && code.chars().count() > 64
        {
            problems.push(FieldError::new(
                format!("regions[{index}].province_code"),
                "length",
                "must be at most 64 characters long",
            ));
            continue;
        }
        out.insert(Region { country, province_code: province.map(str::to_owned) });
    }
    out.into_iter().collect()
}

async fn replace_regions<C: ConnectionTrait>(
    transaction: &C,
    tenant: &TenantContext,
    zone_id: Uuid,
    regions: &[Region],
) -> Result<(), Error> {
    zone_region::Entity::delete_many()
        .filter(zone_region::Column::ZoneId.eq(zone_id))
        .scoped(tenant)
        .exec(transaction)
        .await?;
    for region in regions {
        zone_region::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant.id().as_uuid()),
            zone_id: Set(zone_id),
            country: Set(region.country.clone()),
            province_code: Set(region.province_code.clone()),
        }
        .insert(transaction)
        .await?;
    }
    Ok(())
}

async fn regions_of<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    zone_ids: &[Uuid],
) -> Result<HashMap<Uuid, Vec<Region>>, Error> {
    let rows = zone_region::Entity::find()
        .filter(zone_region::Column::ZoneId.is_in(zone_ids.iter().copied()))
        .scoped(tenant)
        .order_by_asc(zone_region::Column::Country)
        .order_by_asc(zone_region::Column::ProvinceCode)
        .all(connection)
        .await?;
    let mut out: HashMap<Uuid, Vec<Region>> = HashMap::new();
    for row in rows {
        out.entry(row.zone_id)
            .or_default()
            .push(Region { country: row.country, province_code: row.province_code });
    }
    Ok(out)
}

/// Bậc phí chồng nhau trong một vùng → `shipping.rate_overlap`.
pub(crate) fn check_overlaps(rates: &[RateInput]) -> Result<(), Error> {
    let mut by_zone: BTreeMap<Uuid, Vec<&RateInput>> = BTreeMap::new();
    for input in rates {
        by_zone.entry(input.zone_id).or_default().push(input);
    }
    for (zone, mut tiers) in by_zone {
        // Bậc cùng khoảng cân nhưng khác giới hạn cạnh là hợp lệ (cạnh lớn hơn → bậc khác):
        // chỉ so các bậc có cùng `max_side_mm`.
        tiers.sort_by_key(|tier| (tier.max_side_mm, tier.min_weight_grams));
        for pair in tiers.windows(2) {
            let (before, after) = (pair[0], pair[1]);
            if before.max_side_mm != after.max_side_mm {
                continue;
            }
            let overlaps = before.max_weight_grams.is_none_or(|max| max > after.min_weight_grams);
            if overlaps {
                return Err(errors::rate_overlap(zone));
            }
        }
    }
    Ok(())
}

impl ShippingService {
    // ---- Vùng ------------------------------------------------------------------------------

    pub(crate) async fn list_zones(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Option<Uuid>,
        page: &PageRequest,
    ) -> Result<Page<ZoneView>, Error> {
        let mut condition = Condition::all().add(principal.scope_condition(
            &SETTINGS_MANAGE,
            zone::Column::OrgId,
            None,
        )?);
        if let Some(org) = org {
            condition = condition.add(zone::Column::OrgId.eq(org));
        }
        let read = tenant.database().read();
        let select = zone::Entity::find().filter(condition).scoped(tenant);
        let page = fetch_page(select, zone::Column::Id, |row| row.id, page, read).await?;
        let ids: Vec<Uuid> = page.data.iter().map(|row| row.id).collect();
        let mut regions = regions_of(read, tenant, &ids).await?;
        Ok(page.map(|zone| {
            let regions = regions.remove(&zone.id).unwrap_or_default();
            ZoneView { zone, regions }
        }))
    }

    pub(crate) async fn get_zone(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<ZoneView, Error> {
        let read = tenant.database().read();
        let zone = find_zone(read, tenant, id).await?;
        readable(principal, read, &SETTINGS_MANAGE, zone.org_id, || errors::zone_not_found(id))
            .await?;
        let regions = regions_of(read, tenant, &[id]).await?.remove(&id).unwrap_or_default();
        Ok(ZoneView { zone, regions })
    }

    pub(crate) async fn create_zone(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        org: Option<Uuid>,
        name: String,
        regions: Vec<Region>,
    ) -> Result<ZoneView, Error> {
        let org = match org {
            Some(org) => org,
            None => default_org(tenant, principal, &SETTINGS_MANAGE)?.as_uuid(),
        };
        let mut problems = Vec::new();
        let name = name.trim().to_owned();
        check_length(&mut problems, "name", &name, 1, 200);
        let regions = check_regions(&mut problems, &regions);
        fail_if_any(problems)?;
        let transaction = tenant.database().begin_write().await?;
        principal
            .require_write_at(
                &transaction,
                &SETTINGS_MANAGE,
                OrgTarget::new(OrgId::from_uuid(org), None),
            )
            .await?;
        let at = now();
        let id = Uuid::now_v7();
        let zone = zone::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant.id().as_uuid()),
            org_id: Set(org),
            name: Set(name),
            created_at: Set(at),
            updated_at: Set(at),
        }
        .insert(&transaction)
        .await?;
        replace_regions(&transaction, tenant, id, &regions).await?;
        transaction.commit().await?;
        Ok(ZoneView { zone, regions })
    }

    /// Đổi tên và / hoặc thay toàn bộ tỉnh của vùng.
    pub(crate) async fn update_zone(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
        name: Option<String>,
        regions: Option<Vec<Region>>,
    ) -> Result<ZoneView, Error> {
        let mut problems = Vec::new();
        let name = name.map(|name| name.trim().to_owned());
        if let Some(name) = &name {
            check_length(&mut problems, "name", name, 1, 200);
        }
        let regions = regions.map(|regions| check_regions(&mut problems, &regions));
        fail_if_any(problems)?;
        let transaction = tenant.database().begin_write().await?;
        let current = find_zone(&transaction, tenant, id).await?;
        writable(principal, &transaction, &SETTINGS_MANAGE, current.org_id, || {
            errors::zone_not_found(id)
        })
        .await?;
        let mut active: zone::ActiveModel = current.into();
        if let Some(name) = name {
            active.name = Set(name);
        }
        active.updated_at = Set(now());
        let zone = active.update(&transaction).await?;
        if let Some(regions) = &regions {
            replace_regions(&transaction, tenant, id, regions).await?;
        }
        let regions =
            regions_of(&transaction, tenant, &[id]).await?.remove(&id).unwrap_or_default();
        transaction.commit().await?;
        Ok(ZoneView { zone, regions })
    }

    /// Xoá vùng cùng tỉnh và mọi bậc phí trỏ tới nó.
    pub(crate) async fn delete_zone(
        &self,
        tenant: &TenantContext,
        principal: &Principal,
        id: Uuid,
    ) -> Result<(), Error> {
        let transaction = tenant.database().begin_write().await?;
        let current = find_zone(&transaction, tenant, id).await?;
        writable(principal, &transaction, &SETTINGS_MANAGE, current.org_id, || {
            errors::zone_not_found(id)
        })
        .await?;
        rate::Entity::delete_many()
            .filter(rate::Column::ZoneId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        zone_region::Entity::delete_many()
            .filter(zone_region::Column::ZoneId.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        zone::Entity::delete_many()
            .filter(zone::Column::Id.eq(id))
            .scoped(tenant)
            .exec(&transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }
}

pub(crate) async fn find_zone<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    id: Uuid,
) -> Result<zone::Model, Error> {
    zone::Entity::find_by_id(id)
        .scoped(tenant)
        .one(connection)
        .await?
        .ok_or_else(|| errors::zone_not_found(id))
}

pub(crate) async fn find_method<C: ConnectionTrait>(
    connection: &C,
    tenant: &TenantContext,
    id: Uuid,
) -> Result<method::Model, Error> {
    method::Entity::find_by_id(id)
        .scoped(tenant)
        .one(connection)
        .await?
        .ok_or_else(|| errors::method_not_found(id))
}

#[cfg(test)]
mod tests {
    use autoxide_kernel::money::Money;
    use uuid::Uuid;

    use super::{RateInput, check_overlaps, method_name};

    fn tier(zone: Uuid, min: i64, max: Option<i64>) -> RateInput {
        RateInput {
            zone_id: zone,
            min_weight_grams: min,
            max_weight_grams: max,
            max_side_mm: None,
            amount: Money::new(10_000, autoxide_kernel::money::Currency::VND),
            step_grams: None,
            step_amount: None,
        }
    }

    #[test]
    fn bac_chong_nhau_bi_tu_choi() {
        let zone = Uuid::now_v7();
        assert!(check_overlaps(&[tier(zone, 0, Some(500)), tier(zone, 500, None)]).is_ok());
        let error =
            check_overlaps(&[tier(zone, 0, Some(1000)), tier(zone, 500, None)]).unwrap_err();
        assert_eq!(error.code(), "shipping.rate_overlap");
        assert!(check_overlaps(&[tier(zone, 0, None), tier(zone, 2000, None)]).is_err());
        // Vùng khác thì không chồng.
        assert!(check_overlaps(&[tier(zone, 0, None), tier(Uuid::now_v7(), 0, None)]).is_ok());
    }

    #[test]
    fn ten_theo_ngon_ngu() {
        let names = r#"{"en":"Standard","vi":"Tiêu chuẩn"}"#;
        assert_eq!(method_name(names, "vi-VN"), "Tiêu chuẩn");
        assert_eq!(method_name(names, "en"), "Standard");
        assert_eq!(method_name(names, "fr"), "Tiêu chuẩn");
    }
}
