//! `dyn Inventory` và `dyn Promotions` giả cho test của `orders`: đủ hành vi để kiểm giỏ và đặt hàng
//! (tồn theo nút, giữ / xác nhận / trả chỗ theo `order_id`, mã giảm phần trăm), có thể tiêm lỗi.
//! Kiểu của các module đó là `#[non_exhaustive]` nên dựng qua JSON.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use autoxide_inventory::api::{
    Availability, FulfilRequest, Inventory, ReleaseReason, Reservation, ReserveRequest,
    RestockRequest,
};
use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::money::Money;
use autoxide_kernel::reference::Submitted;
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use autoxide_promotions::api::{
    AutomaticPrice, Cart, Eligibility, Evaluation, GrantVoucher, Promotions, Redemption,
    ReserveRequest as PromotionHold, Voucher, VoucherStatus,
};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

// Mã lỗi của module mà bộ giả đóng vai.
const INSUFFICIENT_STOCK: &str = "inventory.insufficient_stock";
const NO_RESERVATION: &str = "inventory.reservation_not_found";
const PROMOTIONS_CHANGED: &str = "promotions.changed";
const EXHAUSTED: &str = "promotions.exhausted";
const NO_REDEMPTION: &str = "promotions.redemption_not_found";

/// Lỗi mang mã của module mà bộ giả đóng vai (`inventory.…`, `promotions.…`): câu và bản dịch là
/// của module đó, không phải của `orders`.
fn foreign(kind: ErrorKind, code: &'static str) -> Error {
    Error::new(kind, code, "Error of the faked module.")
}

/// Một chỗ giữ: nút bán, các dòng (khoá, biến thể, số lượng), trạng thái.
type Hold = (OrgId, Vec<(String, Uuid, i64)>, &'static str);

#[derive(Default)]
pub(crate) struct FakeInventory {
    /// Tồn bán được theo (nút, biến thể).
    stock: Mutex<HashMap<(OrgId, Uuid), i64>>,
    holds: Mutex<HashMap<Uuid, Hold>>,
}

impl FakeInventory {
    pub(crate) fn stock(&self, org: OrgId, variant: Uuid, quantity: i64) {
        *self.stock.lock().unwrap().entry((org, variant)).or_default() += quantity;
    }

    pub(crate) fn available(&self, org: OrgId, variant: Uuid) -> i64 {
        self.stock.lock().unwrap().get(&(org, variant)).copied().unwrap_or(0)
    }

    /// Trạng thái chỗ giữ của đơn (`held`, `confirmed`, `released`).
    pub(crate) fn hold_status(&self, order: Uuid) -> Option<&'static str> {
        self.holds.lock().unwrap().get(&order).map(|hold| hold.2)
    }

    fn reservation(order: Uuid, hold: &Hold) -> Reservation {
        let lines: Vec<Value> = hold
            .1
            .iter()
            .map(|(key, variant, quantity)| {
                json!({"line_key": key, "variant_id": variant, "bundle_variant_id": null,
                    "warehouse_org_id": hold.0, "quantity": quantity, "fulfilled_quantity": 0,
                    "released_quantity": 0, "backordered": false, "preordered": false,
                    "allocated_quantity": quantity})
            })
            .collect();
        serde_json::from_value(json!({
            "id": order, "order_id": order, "selling_org_id": hold.0, "status": hold.2,
            "expires_at": null, "lines": lines,
        }))
        .unwrap()
    }
}

#[async_trait::async_trait]
impl Inventory for FakeInventory {
    async fn availability(
        &self,
        _tenant: &TenantContext,
        selling_org_id: OrgId,
        variant_ids: &[Uuid],
    ) -> Result<Vec<Availability>, Error> {
        let stock = self.stock.lock().unwrap();
        Ok(variant_ids
            .iter()
            .filter_map(|variant| {
                let available = *stock.get(&(selling_org_id, *variant))?;
                serde_json::from_value(json!({"variant_id": variant,
                    "warehouse_org_id": selling_org_id, "on_hand": available,
                    "available": available, "incoming": 0}))
                .ok()
            })
            .collect())
    }

    async fn reserve(
        &self,
        _tenant: &TenantContext,
        request: &ReserveRequest,
    ) -> Result<Submitted<Reservation>, Error> {
        let mut holds = self.holds.lock().unwrap();
        if let Some(hold) = holds.get(&request.order_id) {
            return Ok(Submitted::Replayed(Self::reservation(request.order_id, hold)));
        }
        let mut stock = self.stock.lock().unwrap();
        for line in &request.lines {
            let left = stock.get(&(request.selling_org_id, line.variant_id)).copied().unwrap_or(0);
            if left < line.quantity {
                return Err(foreign(ErrorKind::Conflict, INSUFFICIENT_STOCK)
                    .with_param("variant_id", line.variant_id)
                    .with_param("available", left));
            }
        }
        for line in &request.lines {
            *stock.entry((request.selling_org_id, line.variant_id)).or_default() -= line.quantity;
        }
        let hold: Hold = (
            request.selling_org_id,
            request
                .lines
                .iter()
                .map(|line| (line.line_key.clone(), line.variant_id, line.quantity))
                .collect(),
            "held",
        );
        let reservation = Self::reservation(request.order_id, &hold);
        holds.insert(request.order_id, hold);
        Ok(Submitted::Created(reservation))
    }

    async fn confirm(&self, _tenant: &TenantContext, order_id: Uuid) -> Result<Reservation, Error> {
        let mut holds = self.holds.lock().unwrap();
        let hold =
            holds.get_mut(&order_id).ok_or_else(|| foreign(ErrorKind::NotFound, NO_RESERVATION))?;
        if hold.2 == "held" {
            hold.2 = "confirmed";
        }
        Ok(Self::reservation(order_id, hold))
    }

    async fn release(
        &self,
        _tenant: &TenantContext,
        order_id: Uuid,
        _reason: ReleaseReason,
    ) -> Result<Option<Reservation>, Error> {
        let mut holds = self.holds.lock().unwrap();
        let Some(hold) = holds.get_mut(&order_id) else { return Ok(None) };
        if hold.2 != "released" {
            let mut stock = self.stock.lock().unwrap();
            for (_, variant, quantity) in &hold.1 {
                *stock.entry((hold.0, *variant)).or_default() += quantity;
            }
            hold.2 = "released";
        }
        Ok(Some(Self::reservation(order_id, hold)))
    }

    async fn fulfil(
        &self,
        _tenant: &TenantContext,
        request: &FulfilRequest,
    ) -> Result<Reservation, Error> {
        let holds = self.holds.lock().unwrap();
        let hold = holds
            .get(&request.order_id)
            .ok_or_else(|| foreign(ErrorKind::NotFound, NO_RESERVATION))?;
        Ok(Self::reservation(request.order_id, hold))
    }

    async fn restock(
        &self,
        _tenant: &TenantContext,
        request: &RestockRequest,
    ) -> Result<(), Error> {
        for line in request.lines.iter().filter(|line| line.sellable) {
            self.stock(request.warehouse_org_id, line.variant_id, line.quantity);
        }
        Ok(())
    }

    async fn in_stock_at(
        &self,
        _tenant: &TenantContext,
        store_org_id: OrgId,
        variant_ids: &[Uuid],
    ) -> Result<Vec<Uuid>, Error> {
        let stock = self.stock.lock().unwrap();
        Ok(variant_ids
            .iter()
            .copied()
            .filter(|id| stock.get(&(store_org_id, *id)).is_some_and(|left| *left > 0))
            .collect())
    }

    async fn restock_alerts_of(
        &self,
        _tenant: &TenantContext,
        _user_id: Uuid,
        _variant_ids: &[Uuid],
    ) -> Result<Vec<Uuid>, Error> {
        Ok(Vec::new())
    }
}

/// Khuyến mại giả: mã giảm theo phần vạn của tạm tính (lớp `order`, chia về dòng theo tiền dòng).
#[derive(Default)]
pub(crate) struct FakePromotions {
    codes: Mutex<HashMap<String, (Uuid, i64)>>,
    /// Lượt giữ theo đơn: trạng thái.
    redemptions: Mutex<HashMap<Uuid, &'static str>>,
    /// Số lời gọi `reserve` kế tiếp lỗi như hết lượt.
    pub fail_reserve: AtomicUsize,
}

impl FakePromotions {
    pub(crate) fn add_code(&self, code: &str, basis_points: i64) -> Uuid {
        let id = Uuid::now_v7();
        self.codes.lock().unwrap().insert(code.to_uppercase(), (id, basis_points));
        id
    }

    pub(crate) fn redemption_status(&self, order: Uuid) -> Option<&'static str> {
        self.redemptions.lock().unwrap().get(&order).copied()
    }

    fn reason(code: &str, text: &str) -> Value {
        json!({"code": code, "params": {}, "text": text})
    }

    fn compute(&self, cart: &Cart) -> Evaluation {
        let currency = cart.currency;
        let codes = self.codes.lock().unwrap();
        let subtotals: Vec<i64> =
            cart.lines.iter().map(|line| line.unit_price.minor() * line.quantity).collect();
        let subtotal: i64 = subtotals.iter().sum();
        let mut lines = Vec::new();
        let mut applied = Vec::new();
        let mut rejected = Vec::new();
        let mut total = 0;
        for code in &cart.codes {
            let Some((id, basis_points)) = codes.get(&code.to_uppercase()) else {
                rejected.push(json!({"code": code, "voucher_id": null,
                    "reason": Self::reason("promotions.reason.code_invalid", "Invalid code")}));
                continue;
            };
            let discount = subtotal * basis_points / 10_000;
            if discount == 0 || subtotals.iter().all(|amount| *amount == 0) {
                continue;
            }
            let ratios: Vec<u64> = subtotals.iter().map(|amount| (*amount).max(0) as u64).collect();
            let shares = Money::new(discount, currency).allocate(&ratios).unwrap();
            for (line, share) in cart.lines.iter().zip(shares) {
                if share.is_zero() {
                    continue;
                }
                lines.push(json!({"line_key": line.line_key, "promotion_id": id, "class": "order",
                    "amount": share, "reason": Self::reason("promotions.reason.applied", "Discount"),
                    "capped_by_law": false, "capped_by_floor": false,
                    "funding": [{"payer_kind": "seller", "payer_org_id": cart.org_id, "amount": share}]}));
            }
            applied.push(
                json!({"promotion_id": id, "code": code.to_uppercase(), "voucher_id": null,
                "class": "order", "title": "Discount", "terms": "On the whole order",
                "amount": Money::new(discount, currency), "min_order": null,
                "gift_return_policy": "return_gift", "return_usage_on_cancel": true}),
            );
            total += discount;
        }
        let fingerprint = format!("{total}:{}", cart.codes.join(","));
        serde_json::from_value(json!({
            "currency": currency, "lines": lines, "shipping_discount": Money::zero(currency),
            "shipping": [], "gifts": [], "gift_choices": [],
            "total_discount": Money::new(total, currency), "applied": applied,
            "rejected_codes": rejected, "withheld": [], "clawback_on_return": "none",
            "fingerprint": fingerprint,
        }))
        .unwrap()
    }

    fn redemption(order: Uuid, status: &str, evaluation: Evaluation) -> Redemption {
        serde_json::from_value(json!({"id": order, "order_id": order, "status": status,
            "evaluation": evaluation, "expires_at": null}))
        .unwrap()
    }
}

#[async_trait::async_trait]
impl Promotions for FakePromotions {
    async fn evaluate(&self, _tenant: &TenantContext, cart: &Cart) -> Result<Evaluation, Error> {
        Ok(self.compute(cart))
    }

    async fn eligible(&self, _tenant: &TenantContext, cart: &Cart) -> Result<Eligibility, Error> {
        Ok(serde_json::from_value(json!({"usable": [], "ineligible": [],
            "best_selection": {"codes": [], "voucher_ids": []},
            "best_total_discount": Money::zero(cart.currency)}))
        .unwrap())
    }

    async fn reserve(
        &self,
        _tenant: &TenantContext,
        request: &PromotionHold,
    ) -> Result<Submitted<Redemption>, Error> {
        if self.fail_reserve.load(Ordering::SeqCst) > 0 {
            self.fail_reserve.fetch_sub(1, Ordering::SeqCst);
            return Err(foreign(ErrorKind::Conflict, EXHAUSTED));
        }
        let evaluation = self.compute(&request.cart);
        if let Some(expected) = &request.expected_fingerprint
            && *expected != evaluation.fingerprint
        {
            return Err(foreign(ErrorKind::Conflict, PROMOTIONS_CHANGED));
        }
        let mut redemptions = self.redemptions.lock().unwrap();
        let replay = redemptions.contains_key(&request.order_id);
        redemptions.entry(request.order_id).or_insert("held");
        let redemption = Self::redemption(request.order_id, "held", evaluation);
        Ok(if replay { Submitted::Replayed(redemption) } else { Submitted::Created(redemption) })
    }

    async fn confirm(&self, tenant: &TenantContext, order_id: Uuid) -> Result<Redemption, Error> {
        let _ = tenant;
        let mut redemptions = self.redemptions.lock().unwrap();
        let status = redemptions
            .get_mut(&order_id)
            .ok_or_else(|| foreign(ErrorKind::NotFound, NO_REDEMPTION))?;
        if *status == "held" {
            *status = "confirmed";
        }
        let empty: Evaluation = self.compute(&Cart::new(
            OrgId::from_uuid(Uuid::nil()),
            autoxide_kernel::money::Currency::parse("VND")?,
            Utc::now(),
        ));
        Ok(Self::redemption(order_id, status, empty))
    }

    async fn release(
        &self,
        _tenant: &TenantContext,
        order_id: Uuid,
    ) -> Result<Option<Redemption>, Error> {
        let mut redemptions = self.redemptions.lock().unwrap();
        let Some(status) = redemptions.get_mut(&order_id) else { return Ok(None) };
        *status = "released";
        let empty = self.compute(&Cart::new(
            OrgId::from_uuid(Uuid::nil()),
            autoxide_kernel::money::Currency::parse("VND")?,
            Utc::now(),
        ));
        Ok(Some(Self::redemption(order_id, "released", empty)))
    }

    async fn redemption(
        &self,
        _tenant: &TenantContext,
        _order_id: Uuid,
    ) -> Result<Option<Redemption>, Error> {
        Ok(None)
    }

    async fn grant_voucher(
        &self,
        _tenant: &TenantContext,
        _request: &GrantVoucher,
    ) -> Result<Submitted<Voucher>, Error> {
        Err(Error::internal("not used in these tests"))
    }

    async fn vouchers_of(
        &self,
        _tenant: &TenantContext,
        _user_id: Uuid,
        _status: Option<VoucherStatus>,
    ) -> Result<Vec<Voucher>, Error> {
        Ok(Vec::new())
    }

    async fn automatic_prices(
        &self,
        _tenant: &TenantContext,
        _org_id: OrgId,
        _at: DateTime<Utc>,
        _prices: &[(Uuid, Money)],
    ) -> Result<Vec<AutomaticPrice>, Error> {
        Ok(Vec::new())
    }
}

/// `media` giả: ai đã tải lên ảnh nào.
#[derive(Default)]
pub(crate) struct FakeMedia {
    pub owners: Mutex<HashMap<Uuid, Uuid>>,
}

impl FakeMedia {
    /// Ảnh mới do `user` tải lên.
    pub(crate) fn upload(&self, user: Uuid) -> Uuid {
        let id = Uuid::now_v7();
        self.owners.lock().unwrap().insert(id, user);
        id
    }
}

#[async_trait::async_trait]
impl autoxide_media::api::Media for FakeMedia {
    async fn find_many(
        &self,
        _: &TenantContext,
        _: &[Uuid],
    ) -> Result<Vec<autoxide_media::api::MediaRef>, Error> {
        Ok(Vec::new())
    }

    async fn uploaders(
        &self,
        _: &TenantContext,
        ids: &[Uuid],
    ) -> Result<Vec<(Uuid, Option<Uuid>)>, Error> {
        let owners = self.owners.lock().unwrap();
        Ok(ids.iter().filter_map(|id| owners.get(id).map(|owner| (*id, Some(*owner)))).collect())
    }
}
