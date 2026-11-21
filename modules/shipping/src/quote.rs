//! Báo giá phí giao (bước 44 mục 2): `ShippingRates::quote` cho `orders`, `POST /quote` cho nhân
//! viên / agent, và ước tính công khai dùng chung đúng hàm này.
//!
//! - Phương thức kế thừa theo cây: phương thức của nút bán và tổ tiên, nút gần hơn ghi đè cùng
//!   `code`.
//! - Hàng không cần giao không tính cân; giỏ toàn hàng không cần giao → báo giá rỗng.
//! - Cân tính phí = max(tổng cân thật, tổng cân quy đổi); cân quy đổi một món = D × R × C (cm³) ×
//!   1000 / `volumetric_divisor` gram, số nguyên, làm tròn lên.
//! - `TableRate`: vùng chứa tỉnh đích (vùng theo tỉnh thắng vùng cả nước), bậc `min ≤ cân < max`
//!   (max NULL = vô hạn), cạnh dài nhất ≤ giới hạn bậc; phí bậc + bước vượt.
//! - `Pickup`: mỗi điểm nhận một lựa chọn; không cần đích.
//! - `Carrier`: gọi hãng ngoài transaction, có giới hạn thời gian; lỗi → `unavailable`.
//! - `free_over` so với `order_value` → 0đ.

use autoxide_kernel::time::Timestamp;
use std::collections::{BTreeMap, HashMap};

use autoxide_address::api::AddressInput;
use autoxide_kernel::error::Error;
use autoxide_kernel::money::Money;
use autoxide_kernel::tenancy::{OrgId, OrgTree, ScopeExt, TenantContext};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::api::{
    MethodKind, Quote, QuoteItem, QuoteRequest, ShippingOption, ShippingRates, UnavailableMethod,
};
use crate::entity::{method, pickup_point, rate, zone_region};
use crate::errors;
use crate::service::ShippingService;
use crate::settings::method_name;

/// Mã lý do khi một phương thức không dùng được.
pub(crate) const WEIGHT_MISSING: &str = "shipping.weight_missing";
pub(crate) const NO_RATE: &str = "shipping.no_rate";
pub(crate) const COUNTRY_NOT_SERVED: &str = "shipping.country_not_served";
pub(crate) const NO_PICKUP_POINT: &str = "shipping.no_pickup_point";

/// Số liệu của giỏ theo một phương thức.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Load {
    pub chargeable_grams: i64,
    pub longest_side_mm: i64,
}

/// Cân quy đổi (gram) của một món: D × R × C mm³ → cm³ (÷ 1000) → kg (÷ divisor) → gram (× 1000),
/// gộp lại là mm³ / divisor, làm tròn lên. Thiếu một kích thước → 0.
pub(crate) fn volumetric_grams(item: &QuoteItem, divisor: i64) -> i64 {
    match (item.length_mm, item.width_mm, item.height_mm) {
        (Some(l), Some(w), Some(h)) if divisor > 0 && l > 0 && w > 0 && h > 0 => {
            let cubic = i128::from(l) * i128::from(w) * i128::from(h);
            let divisor = i128::from(divisor);
            let grams = (cubic + divisor - 1) / divisor;
            i64::try_from(grams).unwrap_or(i64::MAX)
        }
        _ => 0,
    }
}

/// Cân tính phí và cạnh dài nhất của giỏ. Món cần giao thiếu cân nặng dùng `default_weight`;
/// không có thì trả SKU của món thiếu.
pub(crate) fn load_of(
    items: &[QuoteItem],
    divisor: i64,
    default_weight: Option<i64>,
) -> Result<Load, String> {
    let (mut actual, mut volumetric, mut longest) = (0i64, 0i64, 0i64);
    for item in items.iter().filter(|item| item.requires_shipping && item.quantity > 0) {
        let Some(weight) = item.weight_grams.or(default_weight) else {
            return Err(item.sku.clone());
        };
        actual = actual.saturating_add(weight.max(0).saturating_mul(item.quantity));
        volumetric = volumetric
            .saturating_add(volumetric_grams(item, divisor).saturating_mul(item.quantity));
        for side in [item.length_mm, item.width_mm, item.height_mm].into_iter().flatten() {
            longest = longest.max(side);
        }
    }
    Ok(Load { chargeable_grams: actual.max(volumetric), longest_side_mm: longest })
}

/// Phí của một bậc cho cân `grams`: phí bậc + mỗi `step_grams` (hay phần lẻ) vượt `min_weight`
/// thêm `step_amount`.
pub(crate) fn tier_amount(tier: &rate::Model, grams: i64) -> i64 {
    let mut amount = tier.amount_minor;
    if let (Some(step), Some(extra)) = (tier.step_grams, tier.step_amount_minor)
        && step > 0
    {
        let over = (grams - tier.min_weight_grams).max(0);
        let steps = (over + step - 1) / step;
        amount = amount.saturating_add(steps.saturating_mul(extra));
    }
    amount
}

/// Bậc khớp cân và cạnh dài nhất, trong các bậc của một vùng.
pub(crate) fn pick_tier(tiers: &[rate::Model], load: Load) -> Option<&rate::Model> {
    tiers.iter().find(|tier| {
        tier.min_weight_grams <= load.chargeable_grams
            && tier.max_weight_grams.is_none_or(|max| load.chargeable_grams < max)
            && tier.max_side_mm.is_none_or(|max| load.longest_side_mm <= max)
    })
}

/// Dấu vân tay của một lựa chọn: phương thức, số tiền, điểm nhận, đích, cân. Đổi một trong
/// chúng → `option_key` khác → `orders` biết phí đã đổi.
fn option_key(
    method_id: Uuid,
    amount: Money,
    pickup: Option<OrgId>,
    request: &QuoteRequest,
    grams: i64,
    service: Option<&str>,
) -> String {
    let destination = request
        .destination
        .as_ref()
        .map(|address| {
            format!(
                "{}|{}|{}|{}",
                address.country, address.province_code, address.commune_code, address.line
            )
        })
        .unwrap_or_default();
    let text = format!(
        "{method_id}|{}|{}|{}|{destination}|{grams}|{}",
        amount.minor(),
        amount.currency().code(),
        pickup.map(|org| org.to_string()).unwrap_or_default(),
        service.unwrap_or_default(),
    );
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().take(16).map(|byte| format!("{byte:02x}")).collect()
}

impl ShippingService {
    /// Phương thức đang bật áp cho nút `org`: của nút và tổ tiên, nút gần hơn ghi đè cùng `code`.
    pub(crate) async fn effective_methods(
        &self,
        tenant: &TenantContext,
        org: OrgId,
    ) -> Result<Vec<method::Model>, Error> {
        let read = tenant.database().read();
        let mut chain = OrgTree::ancestors(read, tenant.id(), org).await?;
        if chain.is_empty() {
            chain.push(org);
        }
        // Gần nhất trước: độ sâu 0 là chính nút.
        let rank: HashMap<Uuid, usize> =
            chain.iter().enumerate().map(|(depth, org)| (org.as_uuid(), depth)).collect();
        let rows = method::Entity::find()
            .filter(method::Column::OrgId.is_in(rank.keys().copied()))
            .filter(method::Column::Active.eq(true))
            .scoped(tenant)
            .all(read)
            .await?;
        let mut by_code: BTreeMap<String, method::Model> = BTreeMap::new();
        for row in rows {
            let depth = rank.get(&row.org_id).copied().unwrap_or(usize::MAX);
            match by_code.get(&row.code) {
                Some(kept) if rank.get(&kept.org_id).copied().unwrap_or(usize::MAX) <= depth => {}
                _ => {
                    by_code.insert(row.code.clone(), row);
                }
            }
        }
        let mut methods: Vec<method::Model> = by_code.into_values().collect();
        methods.sort_by_key(|row| (row.position, row.code.clone()));
        Ok(methods)
    }

    /// Kiểm đích (Việt Nam: xã thuộc tỉnh, còn hiệu lực).
    async fn check_destination(&self, request: &QuoteRequest) -> Result<(), Error> {
        let Some(address) = &request.destination else {
            return Ok(());
        };
        if address.country != "VN" {
            return Ok(());
        }
        let Some(units) = &self.units else {
            return Ok(());
        };
        let input = AddressInput::new(
            "VN",
            address.province_code.clone(),
            address.commune_code.clone(),
            address.line.clone(),
        );
        units.validate(input, "destination.").await.map(|_| ()).map_err(errors::invalid_address)
    }

    /// Báo giá cho nhân viên / agent: cần `shipping.read` phủ cửa hàng bán.
    pub(crate) async fn quote_for(
        &self,
        tenant: &TenantContext,
        principal: &autoxide_kernel::access::Principal,
        request: &QuoteRequest,
    ) -> Result<Quote, Error> {
        let target = autoxide_kernel::access::OrgTarget::new(request.org_id, None);
        principal.require_at(tenant.database().read(), &crate::permissions::READ, target).await?;
        self.quote_options(tenant, request).await
    }

    /// Báo giá đầy đủ.
    pub(crate) async fn quote_options(
        &self,
        tenant: &TenantContext,
        request: &QuoteRequest,
    ) -> Result<Quote, Error> {
        self.check_destination(request).await?;
        self.quote_unchecked(tenant, request).await
    }

    /// Báo giá không kiểm đích (ước tính công khai: đích chỉ có tỉnh).
    pub(crate) async fn quote_unchecked(
        &self,
        tenant: &TenantContext,
        request: &QuoteRequest,
    ) -> Result<Quote, Error> {
        if !request.items.iter().any(|item| item.requires_shipping && item.quantity > 0) {
            return Ok(Quote::default());
        }
        let currency = request.order_value.currency();
        let methods = self.effective_methods(tenant, request.org_id).await?;
        let mut quote = Quote::default();
        for method in methods {
            let Some(kind) = MethodKind::parse(&method.kind) else { continue };
            if method.currency != currency.code() {
                continue;
            }
            let load = match load_of(
                &request.items,
                i64::from(method.volumetric_divisor),
                method.default_item_weight_grams,
            ) {
                Ok(load) => load,
                Err(_) if kind == MethodKind::Pickup => {
                    Load { chargeable_grams: 0, longest_side_mm: 0 }
                }
                Err(_) => {
                    quote.unavailable.push(UnavailableMethod::new(&method.code, WEIGHT_MISSING));
                    continue;
                }
            };
            let options = match kind {
                MethodKind::Pickup => self.pickup_options(tenant, &method, load).await?,
                MethodKind::TableRate => {
                    let Some(destination) = &request.destination else { continue };
                    if !serves(&method, &destination.country) {
                        quote
                            .unavailable
                            .push(UnavailableMethod::new(&method.code, COUNTRY_NOT_SERVED));
                        continue;
                    }
                    match self.table_rate(tenant, &method, destination, load).await? {
                        Some(amount) => vec![(amount, None, None, None)],
                        None => {
                            quote.unavailable.push(UnavailableMethod::new(&method.code, NO_RATE));
                            continue;
                        }
                    }
                }
                MethodKind::Carrier => {
                    let Some(destination) = &request.destination else { continue };
                    if !serves(&method, &destination.country) {
                        quote
                            .unavailable
                            .push(UnavailableMethod::new(&method.code, COUNTRY_NOT_SERVED));
                        continue;
                    }
                    match self.carrier_rates(tenant, &method, request, load).await {
                        Ok(rates) if !rates.is_empty() => rates,
                        Ok(_) => {
                            quote.unavailable.push(UnavailableMethod::new(&method.code, NO_RATE));
                            continue;
                        }
                        Err(error) => {
                            quote
                                .unavailable
                                .push(UnavailableMethod::new(&method.code, error.code()));
                            continue;
                        }
                    }
                }
            };
            if options.is_empty() {
                quote.unavailable.push(UnavailableMethod::new(&method.code, NO_PICKUP_POINT));
                continue;
            }
            let carrier_code = match (kind, method.carrier_account_id) {
                (MethodKind::Carrier, Some(account)) => Some(
                    crate::carriers::find_account(tenant.database().read(), tenant, account)
                        .await?
                        .carrier_code,
                ),
                _ => None,
            };
            let calendar =
                self.calendar_for(tenant, method.calendar_id, request.at.date_utc()).await?;
            for (amount, pickup, service, days) in options {
                let mut option =
                    self.option(&method, kind, amount, pickup, service, days, load, request)?;
                option.carrier_code.clone_from(&carrier_code);
                let days = option.estimated_days;
                option.eta = crate::eta::estimate(
                    &calendar,
                    request.at,
                    method.cutoff_minute,
                    method.same_day,
                    days,
                );
                if let Some(store) = pickup {
                    option.ready_at =
                        Some(self.ready_at(tenant, &method, store, &calendar, request).await?);
                }
                quote.options.push(option);
            }
        }
        quote.options.sort_by_key(|option| option.amount.minor());
        Ok(quote)
    }

    /// Dựng một lựa chọn (áp phí xử lý, miễn phí, dấu vân tay).
    #[expect(clippy::too_many_arguments, reason = "các phần của một lựa chọn, chỉ gọi ở một chỗ")]
    fn option(
        &self,
        method: &method::Model,
        kind: MethodKind,
        base_minor: i64,
        pickup: Option<OrgId>,
        service: Option<String>,
        days: Option<(u16, u16)>,
        load: Load,
        request: &QuoteRequest,
    ) -> Result<ShippingOption, Error> {
        let currency = request.order_value.currency();
        let free_over = method.free_over_minor.map(|minor| Money::new(minor, currency));
        let free = free_over.is_some_and(|limit| request.order_value.minor() >= limit.minor());
        let amount = if free {
            Money::zero(currency)
        } else {
            Money::new(base_minor.saturating_add(method.handling_fee_minor), currency)
        };
        let estimated_days =
            days.or_else(|| match (method.estimated_min_days, method.estimated_max_days) {
                (Some(min), Some(max)) => {
                    Some((u16::try_from(min).unwrap_or(0), u16::try_from(max).unwrap_or(0)))
                }
                _ => None,
            });
        let key = option_key(
            method.id,
            amount,
            pickup,
            request,
            load.chargeable_grams,
            service.as_deref(),
        );
        Ok(ShippingOption {
            method_id: method.id,
            method_code: method.code.clone(),
            name: method_name(&method.names, &request.locale),
            kind,
            amount,
            free_shipping_applied: free,
            free_over,
            tax_category: method.tax_category.clone(),
            carrier_code: None,
            service_code: service.or_else(|| method.service_code.clone()),
            pickup_org_id: pickup,
            chargeable_weight_grams: load.chargeable_grams,
            estimated_days,
            eta: None,
            ready_at: None,
            hold_days: method.hold_days.and_then(|days| u16::try_from(days).ok()),
            option_key: key,
        })
    }

    /// Phí bảng của phương thức tới `destination`, hay `None` khi không vùng / bậc nào khớp.
    async fn table_rate(
        &self,
        tenant: &TenantContext,
        method: &method::Model,
        destination: &crate::api::Address,
        load: Load,
    ) -> Result<Option<i64>, Error> {
        let read = tenant.database().read();
        // Vùng có tỉnh đích, và vùng cả nước (`province_code IS NULL` — không `= NULL`).
        let regions = zone_region::Entity::find()
            .filter(zone_region::Column::Country.eq(&destination.country))
            .filter(
                Condition::any()
                    .add(zone_region::Column::ProvinceCode.eq(&destination.province_code))
                    .add(zone_region::Column::ProvinceCode.is_null()),
            )
            .scoped(tenant)
            .all(read)
            .await?;
        if regions.is_empty() {
            return Ok(None);
        }
        let tiers = rate::Entity::find()
            .filter(rate::Column::MethodId.eq(method.id))
            .filter(rate::Column::ZoneId.is_in(regions.iter().map(|region| region.zone_id)))
            .scoped(tenant)
            .order_by_asc(rate::Column::MinWeightGrams)
            .all(read)
            .await?;
        // Vùng theo tỉnh thắng vùng cả nước.
        for specific in [true, false] {
            let zones: Vec<Uuid> = regions
                .iter()
                .filter(|region| region.province_code.is_some() == specific)
                .map(|region| region.zone_id)
                .collect();
            let candidates: Vec<rate::Model> =
                tiers.iter().filter(|tier| zones.contains(&tier.zone_id)).cloned().collect();
            if candidates.is_empty() {
                continue;
            }
            return Ok(
                pick_tier(&candidates, load).map(|tier| tier_amount(tier, load.chargeable_grams))
            );
        }
        Ok(None)
    }

    /// Mỗi điểm nhận một lựa chọn, phí 0 (phí xử lý của phương thức cộng sau).
    async fn pickup_options(
        &self,
        tenant: &TenantContext,
        method: &method::Model,
        _load: Load,
    ) -> Result<Vec<(i64, Option<OrgId>, Option<String>, Option<(u16, u16)>)>, Error> {
        let points = pickup_point::Entity::find()
            .filter(pickup_point::Column::MethodId.eq(method.id))
            .scoped(tenant)
            .order_by_asc(pickup_point::Column::Id)
            .all(tenant.database().read())
            .await?;
        Ok(points
            .into_iter()
            .map(|point| (0, Some(OrgId::from_uuid(point.org_id)), None, None))
            .collect())
    }
}

impl ShippingService {
    /// Phí của hãng cho phương thức `Carrier`: gọi `ShippingCarrier::quote` ngoài transaction, có
    /// giới hạn thời gian. Lỗi → phương thức vào `unavailable` (không làm hỏng cả báo giá).
    async fn carrier_rates(
        &self,
        tenant: &TenantContext,
        method: &method::Model,
        request: &QuoteRequest,
        load: Load,
    ) -> Result<Vec<(i64, Option<OrgId>, Option<String>, Option<(u16, u16)>)>, Error> {
        let account_id = method.carrier_account_id.ok_or_else(errors::no_carrier)?;
        let (row, account, carrier) = self.account(tenant, account_id).await?;
        if !row.active {
            return Err(errors::no_carrier());
        }
        let Some(destination) = request.destination.clone() else { return Ok(Vec::new()) };
        let parcel = crate::api::CarrierParcel::new(load.chargeable_grams.max(1));
        let ask =
            crate::api::CarrierQuoteRequest::new(destination, vec![parcel], request.order_value)
                .service(method.service_code.clone());
        let rates = match crate::carriers::within(
            self.config.carrier_timeout_seconds,
            carrier.quote(&account, &ask),
        )
        .await
        {
            Ok(rates) => rates,
            Err(error) => {
                self.note_carrier_error(tenant, account_id, &error).await;
                return Err(error);
            }
        };
        let currency = request.order_value.currency();
        Ok(rates
            .into_iter()
            .filter(|rate| rate.amount.currency() == currency)
            .map(|rate| (rate.amount.minor(), None, Some(rate.service_code), rate.estimated_days))
            .collect())
    }
}

/// Phương thức giao tới `country` (cột `countries`, mảng JSON; rỗng = chỉ `VN`).
fn serves(method: &method::Model, country: &str) -> bool {
    let countries: Vec<String> = serde_json::from_str(&method.countries).unwrap_or_default();
    if countries.is_empty() {
        country == "VN"
    } else {
        countries.iter().any(|code| code == country)
    }
}

#[async_trait::async_trait]
impl ShippingRates for ShippingService {
    async fn quote(&self, tenant: &TenantContext, request: &QuoteRequest) -> Result<Quote, Error> {
        self.quote_options(tenant, request).await
    }

    async fn confirm_option(
        &self,
        tenant: &TenantContext,
        request: &QuoteRequest,
        option_key: &str,
    ) -> Result<ShippingOption, Error> {
        let quote = self.quote_options(tenant, request).await?;
        quote
            .options
            .into_iter()
            .find(|option| option.option_key == option_key)
            .ok_or_else(errors::option_changed)
    }
}

impl ShippingService {
    /// Giờ hàng sẵn sàng ở cửa hàng `store`: chuẩn bị của điểm nhận + giờ mở cửa (hồ sơ công khai
    /// của nút qua `OrgDirectory`; chưa khai thì 08:00–21:00 theo lịch của phương thức).
    async fn ready_at(
        &self,
        tenant: &TenantContext,
        method: &method::Model,
        store: OrgId,
        calendar: &crate::eta::Calendar,
        request: &QuoteRequest,
    ) -> Result<Timestamp, Error> {
        let prep = pickup_point::Entity::find()
            .filter(pickup_point::Column::MethodId.eq(method.id))
            .filter(pickup_point::Column::OrgId.eq(store.as_uuid()))
            .scoped(tenant)
            .one(tenant.database().read())
            .await?
            .map_or(0, |point| point.prep_hours);
        let ranges = match &self.orgs {
            Some(orgs) => orgs.opening_hours(tenant, store).await?,
            None => None,
        };
        let hours = crate::eta::Hours::from_ranges(ranges);
        Ok(crate::eta::ready_at(calendar, &hours, request.at, prep))
    }
}
