//! Tiền hoàn của một lần trả hàng (D-222, đặc tả 43 mục "Trả hàng và huỷ"): hàm thuần trên dòng đơn
//! và ảnh chụp `Evaluation` đã lưu vào đơn — không gọi database.
//!
//! - Hàng: mỗi dòng hoàn theo tỉ lệ tiền đã trả (`promotions::api::returns::line_refund` — đơn vị cuối
//!   nhận phần dư, nên trả nhiều lần cộng lại đúng bằng tiền đã trả).
//! - Phí giao: chỉ khi lỗi do người bán ([`SELLER_FAULT`]); chia theo số đơn vị hàng cần giao (không
//!   tính quà), cùng
//!   cách chia — trả hết hàng thì hoàn đủ phí giao đã trả.
//! - Thu lại khoản giảm lớp `order` khi đơn còn lại rớt dưới ngưỡng (`order_clawback`, theo
//!   `clawback_on_return` đã chụp; mặc định không thu). Tính theo TỔNG sau lần trả, trừ phần đã thu ở
//!   các lần trước — trả tiếp món đã bị thu thì được trả lại phần đó.
//! - Quà: chương trình mất điều kiện (không còn món nào, hay đơn còn lại dưới `min_order`) mà người mua
//!   giữ quà → trừ giá trị quà (`compare_at` của dòng quà), trừ khi chương trình chọn `keep`; mỗi
//!   chương trình trừ một lần. Chương trình `return_gift`: lúc yêu cầu, dòng quà được thêm vào yêu
//!   cầu trả ([`gifts_to_return`]) — chỉ quà không gửi lại mới bị trừ.

use std::collections::HashMap;

use autoxide_kernel::error::Error;
use autoxide_kernel::money::Money;
use autoxide_promotions::api::returns::{line_refund, order_clawback};
use autoxide_promotions::api::{Evaluation, GiftReturnPolicy};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::entity::{line, order};

/// Lý do trả do lỗi của người bán: hoàn cả phí giao.
pub(crate) const SELLER_FAULT: [&str; 4] =
    ["damaged", "wrong_item", "not_as_described", "missing_parts"];

/// Cách tính tiền hoàn của một lần trả (lưu cùng yêu cầu để lần trả sau biết phần đã thu / đã trừ).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Breakdown {
    pub goods: i64,
    pub shipping: i64,
    /// Khoản giảm thu lại ở lần này (âm: trả lại phần đã thu ở lần trước).
    pub clawback: i64,
    pub gift_deduction: i64,
    /// Chương trình đã bị trừ giá trị quà ở lần này.
    #[serde(default)]
    pub gift_deducted: Vec<Uuid>,
    pub total: i64,
}

/// Các lần trả trước của đơn (đã nhận hàng).
#[derive(Debug, Clone, Default)]
pub(crate) struct Earlier {
    pub clawback: i64,
    pub gift_deducted: Vec<Uuid>,
}

impl Earlier {
    pub(crate) fn add(&mut self, breakdown: &Breakdown) {
        self.clawback += breakdown.clawback;
        self.gift_deducted.extend(breakdown.gift_deducted.iter().copied());
    }
}

/// Giá trị một đơn vị của dòng (tiền đã trả; quà: giá niêm yết) — khai giá trị hàng gửi trả.
pub(crate) fn unit_value(line: &line::Model) -> i64 {
    if line.gift_of.is_some() {
        line.compare_at_minor.unwrap_or(0)
    } else if line.quantity > 0 {
        line.total_minor / line.quantity
    } else {
        0
    }
}

/// Đơn vị còn giữ của từng dòng sau khi trả `returning` (đã trả trước đó: `returned_quantity`).
fn kept(line: &line::Model, returning: &HashMap<Uuid, i64>) -> i64 {
    line.quantity - line.returned_quantity - returning.get(&line.id).copied().unwrap_or(0)
}

/// Giá trị hàng còn giữ (trước giảm, không tính quà) — để so với ngưỡng của chương trình.
fn kept_subtotal(lines: &[line::Model], returning: &HashMap<Uuid, i64>) -> i64 {
    lines
        .iter()
        .filter(|line| line.gift_of.is_none() && line.quantity > 0)
        .map(|line| {
            let kept = i128::from(kept(line, returning));
            let value = i128::from(line.subtotal_minor) * kept / i128::from(line.quantity);
            i64::try_from(value).unwrap_or(i64::MAX)
        })
        .sum()
}

/// Chương trình có quà đã mất điều kiện sau khi trả `returning`.
fn lost_gift_promotions(
    lines: &[line::Model],
    returning: &HashMap<Uuid, i64>,
    evaluation: &Evaluation,
) -> Result<Vec<(Uuid, GiftReturnPolicy)>, Error> {
    let any_kept = lines.iter().any(|line| line.gift_of.is_none() && kept(line, returning) > 0);
    let subtotal = Money::new(kept_subtotal(lines, returning), evaluation.currency);
    let mut lost = Vec::new();
    for applied in &evaluation.applied {
        if !lines.iter().any(|line| line.gift_of == Some(applied.promotion_id)) {
            continue;
        }
        let below = match applied.min_order {
            Some(min) => subtotal.compare(min)? == std::cmp::Ordering::Less,
            None => false,
        };
        if !any_kept || below {
            lost.push((applied.promotion_id, applied.gift_return_policy));
        }
    }
    Ok(lost)
}

/// Dòng quà phải gửi kèm khi trả `returning` (chương trình `return_gift` mất điều kiện): số còn giữ
/// của mỗi dòng quà.
pub(crate) fn gifts_to_return(
    lines: &[line::Model],
    returning: &HashMap<Uuid, i64>,
    evaluation: Option<&Evaluation>,
) -> Result<Vec<(Uuid, i64)>, Error> {
    let Some(evaluation) = evaluation else { return Ok(Vec::new()) };
    let mut extra = Vec::new();
    for (promotion, policy) in lost_gift_promotions(lines, returning, evaluation)? {
        if policy != GiftReturnPolicy::ReturnGift {
            continue;
        }
        for gift in lines.iter().filter(|line| line.gift_of == Some(promotion)) {
            let left = kept(gift, returning);
            if left > 0 {
                extra.push((gift.id, left));
            }
        }
    }
    Ok(extra)
}

/// Tiền hoàn khi nhận về `returning` (dòng → số đơn vị) của đơn `order`.
pub(crate) fn refund_for(
    order: &order::Model,
    lines: &[line::Model],
    returning: &HashMap<Uuid, i64>,
    seller_fault: bool,
    evaluation: Option<&Evaluation>,
    earlier: &Earlier,
) -> Result<Breakdown, Error> {
    let money = |minor: i64| Money::from_columns(minor, &order.currency);
    let mut breakdown = Breakdown::default();
    for line in lines {
        let quantity = returning.get(&line.id).copied().unwrap_or(0);
        if quantity > 0 {
            let paid = money(line.total_minor)?;
            let refund = line_refund(paid, line.quantity, line.returned_quantity, quantity)?;
            breakdown.goods += refund.minor();
        }
    }
    if seller_fault {
        let goods_paid: i64 = lines.iter().map(|line| line.total_minor).sum();
        let shipping_paid = (order.total_minor - goods_paid).max(0);
        let shipped: Vec<&line::Model> =
            lines.iter().filter(|line| line.requires_shipping && line.gift_of.is_none()).collect();
        let units: i64 = shipped.iter().map(|line| line.quantity).sum();
        let before: i64 = shipped.iter().map(|line| line.returned_quantity).sum();
        let now: i64 =
            shipped.iter().map(|line| returning.get(&line.id).copied().unwrap_or(0)).sum();
        if shipping_paid > 0 && units > 0 && now > 0 {
            breakdown.shipping = line_refund(money(shipping_paid)?, units, before, now)?.minor();
        }
    }
    if let Some(evaluation) = evaluation {
        let kept_keys: Vec<&str> = lines
            .iter()
            .filter(|line| kept(line, returning) > 0)
            .map(|line| line.line_key.as_str())
            .collect();
        let subtotal = money(kept_subtotal(lines, returning))?;
        let after = order_clawback(evaluation, &kept_keys, subtotal)?.minor();
        breakdown.clawback = after - earlier.clawback;
        for (promotion, policy) in lost_gift_promotions(lines, returning, evaluation)? {
            if policy == GiftReturnPolicy::Keep || earlier.gift_deducted.contains(&promotion) {
                continue;
            }
            let value: i64 = lines
                .iter()
                .filter(|line| line.gift_of == Some(promotion))
                .map(|gift| gift.compare_at_minor.unwrap_or(0) * kept(gift, returning).max(0))
                .sum();
            if value > 0 {
                breakdown.gift_deduction += value;
                breakdown.gift_deducted.push(promotion);
            }
        }
    }
    let gross = breakdown.goods + breakdown.shipping;
    let total = gross - breakdown.clawback - breakdown.gift_deduction;
    if total < 0 {
        // Không đòi thêm tiền người mua: phần thu lại chỉ tới hết tiền hoàn của lần này.
        breakdown.clawback = (gross - breakdown.gift_deduction).max(0).min(breakdown.clawback);
        breakdown.total = (gross - breakdown.clawback - breakdown.gift_deduction).max(0);
    } else {
        breakdown.total = total;
    }
    Ok(breakdown)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    fn line(key: &str, quantity: i64, subtotal: i64, total: i64) -> line::Model {
        line::Model {
            id: Uuid::now_v7(),
            tenant_id: Uuid::nil(),
            order_id: Uuid::nil(),
            line_key: key.to_owned(),
            parent_line_id: None,
            variant_id: Uuid::now_v7(),
            product_id: Uuid::now_v7(),
            category_id: None,
            seller_org_id: Uuid::nil(),
            kind: "physical".to_owned(),
            sku: key.to_owned(),
            name: key.to_owned(),
            snapshot: "{}".to_owned(),
            sale_mode: "in_stock".to_owned(),
            preorder_available_on: None,
            requires_shipping: true,
            tax_inclusive: true,
            quantity,
            unit_price_minor: subtotal / quantity,
            subtotal_minor: subtotal,
            discount_minor: subtotal - total,
            net_minor: total,
            tax_minor: 0,
            total_minor: total,
            compare_at_minor: None,
            discounts: "[]".to_owned(),
            gift_of: None,
            warehouse_org_id: None,
            returned_quantity: 0,
            allocated_quantity: 0,
            cancelled_quantity: 0,
            preorder_delayed_to: None,
            created_at: Utc::now(),
        }
    }

    fn order(total: i64) -> order::Model {
        let now = Utc::now();
        order::Model {
            id: Uuid::now_v7(),
            tenant_id: Uuid::nil(),
            org_id: Uuid::nil(),
            checkout_id: None,
            number: "2026-000001".to_owned(),
            channel: "web".to_owned(),
            external_ref: None,
            buyer_user_id: None,
            buyer_key: None,
            contact: "{}".to_owned(),
            ship_to: None,
            shipping_option: None,
            pickup_org_id: None,
            status: "delivered".to_owned(),
            payment_status: "paid".to_owned(),
            fulfillment_status: "fulfilled".to_owned(),
            payment_method: "card".to_owned(),
            funds_flow: "self_collect".to_owned(),
            currency: "VND".to_owned(),
            subtotal_minor: 0,
            discount_minor: 0,
            shipping_fee_minor: 0,
            shipping_discount_minor: 0,
            tax_minor: 0,
            total_minor: total,
            paid_minor: total,
            refunded_minor: 0,
            payment_due_at: None,
            attribution: None,
            promotions: None,
            stock_shortage: false,
            invoice_request: None,
            terms_version: None,
            note: None,
            locale: "vi".to_owned(),
            placed_at: now,
            confirmed_at: Some(now),
            delivered_at: Some(now),
            completed_at: None,
            cancelled_at: None,
            cancel_reason: None,
            preorder_accepted_at: None,
            version: 1,
            created_at: now,
            updated_at: now,
        }
    }

    /// Đơn 500k giảm 50k (chương trình lớp `order`, ngưỡng 400k), quà tặng trị giá 30k.
    fn evaluation(promotion: Uuid, clawback: &str, gift_policy: &str) -> Evaluation {
        let vnd = |minor: i64| json!({"amount": minor.to_string(), "currency": "VND"});
        let reason = json!({"code": "x", "params": {}, "text": "x"});
        serde_json::from_value(json!({
            "currency": "VND",
            "lines": [
                {"line_key": "a", "promotion_id": promotion, "class": "order", "amount": vnd(30_000),
                 "reason": reason, "capped_by_law": false, "capped_by_floor": false, "funding": []},
                {"line_key": "b", "promotion_id": promotion, "class": "order", "amount": vnd(20_000),
                 "reason": reason, "capped_by_law": false, "capped_by_floor": false, "funding": []},
            ],
            "shipping_discount": vnd(0), "shipping": [], "gifts": [], "gift_choices": [],
            "total_discount": vnd(50_000),
            "applied": [{"promotion_id": promotion, "code": null, "voucher_id": null, "class": "order",
                         "title": "t", "terms": "t", "amount": vnd(50_000), "min_order": vnd(400_000),
                         "gift_return_policy": gift_policy, "return_usage_on_cancel": true}],
            "rejected_codes": [], "withheld": [], "clawback_on_return": clawback, "fingerprint": "f",
        }))
        .unwrap()
    }

    /// Dòng a: 300k (trả 270k sau giảm), dòng b: 200k (trả 180k), quà 30k; phí giao 30k.
    fn setup(promotion: Uuid) -> (order::Model, Vec<line::Model>) {
        let a = line("a", 1, 300_000, 270_000);
        let b = line("b", 2, 200_000, 180_000);
        let mut gift = line("gift", 1, 0, 0);
        gift.gift_of = Some(promotion);
        gift.compare_at_minor = Some(30_000);
        (order(480_000), vec![a, b, gift])
    }

    #[test]
    fn tra_mot_phan_hoan_theo_ti_le_tong_khop() {
        let promotion = Uuid::now_v7();
        let (order, mut lines) = setup(promotion);
        let none = evaluation(promotion, "none", "keep");
        let b = lines[1].id;
        // Trả 1 rồi 1 đơn vị của dòng b (180k cho 2): 90k + 90k = đúng số đã trả.
        let first = HashMap::from([(b, 1)]);
        let one =
            refund_for(&order, &lines, &first, false, Some(&none), &Earlier::default()).unwrap();
        lines[1].returned_quantity = 1;
        let two =
            refund_for(&order, &lines, &first, false, Some(&none), &Earlier::default()).unwrap();
        assert_eq!(one.goods + two.goods, 180_000);
        assert_eq!((one.clawback, one.shipping), (0, 0));
    }

    #[test]
    fn roi_duoi_nguong_thu_lai_khoan_giam_va_tra_lai_khi_tra_tiep() {
        let promotion = Uuid::now_v7();
        let (order, mut lines) = setup(promotion);
        let proportional = evaluation(promotion, "proportional", "keep");
        let (a, b) = (lines[0].id, lines[1].id);
        // Trả dòng b: còn 300k < ngưỡng 400k → thu lại 30k đã giảm cho dòng a.
        let returning = HashMap::from([(b, 2)]);
        let first =
            refund_for(&order, &lines, &returning, false, Some(&proportional), &Earlier::default())
                .unwrap();
        assert_eq!((first.goods, first.clawback, first.total), (180_000, 30_000, 150_000));
        // Trả tiếp dòng a: phần đã thu được trả lại; tổng hoàn = tổng đã trả cho hàng.
        lines[1].returned_quantity = 2;
        let mut earlier = Earlier::default();
        earlier.add(&first);
        let returning = HashMap::from([(a, 1)]);
        let second =
            refund_for(&order, &lines, &returning, false, Some(&proportional), &earlier).unwrap();
        assert_eq!(second.clawback, -30_000);
        assert_eq!(first.total + second.total, 450_000);
    }

    #[test]
    fn tra_mon_dieu_kien_tru_gia_tri_qua_va_hoan_phi_giao_khi_loi_nguoi_ban() {
        let promotion = Uuid::now_v7();
        let (order, lines) = setup(promotion);
        let (a, b) = (lines[0].id, lines[1].id);
        let all = HashMap::from([(a, 1), (b, 2)]);
        // Trả hết hàng, giữ quà: trừ 30k giá trị quà; lỗi người bán: hoàn cả phí giao 30k.
        let deduct = evaluation(promotion, "none", "deduct_gift_value");
        let refund =
            refund_for(&order, &lines, &all, true, Some(&deduct), &Earlier::default()).unwrap();
        assert_eq!(refund.shipping, 30_000);
        assert_eq!(refund.gift_deduction, 30_000);
        assert_eq!(refund.total, 450_000 + 30_000 - 30_000);
        // `return_gift`: quà phải gửi kèm; gửi kèm thì không trừ.
        let back = evaluation(promotion, "none", "return_gift");
        let gifts = gifts_to_return(&lines, &all, Some(&back)).unwrap();
        assert_eq!(gifts, vec![(lines[2].id, 1)]);
        let mut with_gift = all.clone();
        with_gift.insert(lines[2].id, 1);
        let refund =
            refund_for(&order, &lines, &with_gift, false, Some(&back), &Earlier::default())
                .unwrap();
        assert_eq!((refund.gift_deduction, refund.total), (0, 450_000));
        // `keep`: giữ quà, không trừ.
        let keep = evaluation(promotion, "none", "keep");
        assert!(gifts_to_return(&lines, &all, Some(&keep)).unwrap().is_empty());
    }
}
