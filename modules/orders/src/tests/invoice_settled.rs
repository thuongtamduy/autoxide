//! So hoá đơn với số đơn đã chốt khi đơn bị huỷ một phần (kiểm độc lập #575, M1): phần còn lại dựng như dòng hoá đơn,
//! so bằng số chính xác — không báo nhầm vì làm tròn. Test thuần, không chạm database.

use autoxide_invoices::api::{Buyer, DocumentLine, NewDocument};
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::tenancy::{DEFAULT_TENANT, root_of};
use autoxide_kernel::time::Timestamp;
use uuid::Uuid;

use super::invoices_pos::document;
use crate::entity::{line, order};
use crate::invoicing::{differs_from_order, settled_lines};

fn vnd(minor: i64) -> Money {
    Money::new(minor, Currency::VND)
}

/// Dòng đơn `quantity` × `unit`, giảm `discount`, thuế `tax`, tổng `total`, huỷ `cancelled`.
fn order_line(
    inclusive: bool,
    quantity: i64,
    unit: i64,
    discount: i64,
    tax: i64,
    total: i64,
    cancelled: i64,
) -> line::Model {
    let net = unit.wrapping_mul(quantity).wrapping_sub(discount);
    line::Model {
        id: Uuid::now_v7(),
        tenant_id: Uuid::nil(),
        order_id: Uuid::nil(),
        line_key: "a".to_owned(),
        parent_line_id: None,
        variant_id: Uuid::now_v7(),
        product_id: Uuid::now_v7(),
        category_id: None,
        seller_org_id: Uuid::nil(),
        kind: "physical".to_owned(),
        sku: "SKU-1".to_owned(),
        name: "Item".to_owned(),
        snapshot: "{}".to_owned(),
        sale_mode: "in_stock".to_owned(),
        preorder_available_on: None,
        requires_shipping: true,
        tax_inclusive: inclusive,
        quantity,
        unit_price_minor: unit,
        subtotal_minor: unit.wrapping_mul(quantity),
        discount_minor: discount,
        net_minor: net,
        tax_minor: tax,
        total_minor: total,
        compare_at_minor: None,
        discounts: "[]".to_owned(),
        gift_of: None,
        warehouse_org_id: None,
        returned_quantity: 0,
        allocated_quantity: 0,
        cancelled_quantity: cancelled,
        preorder_delayed_to: None,
        advance_minor: None,
        preorder_commit_at: None,
        created_at: Timestamp::now(),
    }
}

fn order_of(lines: &[line::Model], shipping: i64, shipping_tax: i64) -> order::Model {
    let now = Timestamp::now();
    let tax: i64 = lines.iter().map(|line| line.tax_minor).sum::<i64>() + shipping_tax;
    let total: i64 = lines.iter().map(|line| line.total_minor).sum::<i64>() + shipping;
    order::Model {
        id: Uuid::now_v7(),
        tenant_id: Uuid::nil(),
        org_id: Uuid::nil(),
        checkout_id: None,
        number: "2026-000001".to_owned(),
        channel: "web".to_owned(),
        custom_fields: None,
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
        shipping_fee_minor: shipping,
        shipping_discount_minor: 0,
        tax_minor: tax,
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

/// Hoá đơn có các dòng (`item_ref`, thuế, tổng) và tổng `total`.
fn invoice(lines: &[(Option<&str>, i64, i64)], total: i64) -> autoxide_invoices::api::Document {
    let draft = NewDocument::new(
        root_of(DEFAULT_TENANT),
        "vat_invoice",
        Currency::VND,
        Buyer::consumer(Some("An".to_owned())),
        Vec::new(),
        "orders",
        "orders:x:invoice",
    );
    let mut made = document(&draft, None);
    made.lines = lines
        .iter()
        .enumerate()
        .map(|(index, (item_ref, tax, line_total))| DocumentLine {
            line_no: i32::try_from(index + 1).unwrap(),
            kind: "goods".to_owned(),
            item_ref: item_ref.map(str::to_owned),
            description: "x".to_owned(),
            unit: None,
            quantity: "1".to_owned(),
            unit_price: vnd(line_total - tax),
            discount: vnd(0),
            net: vnd(line_total - tax),
            tax_category: Some("vat.standard".to_owned()),
            rate_percent: Some("10".to_owned()),
            tax: vnd(*tax),
            total: vnd(*line_total),
        })
        .collect();
    made.tax = vnd(lines.iter().map(|(_, tax, _)| tax).sum());
    made.total = vnd(total);
    made
}

/// Giá gồm thuế, GTGT 10%: 95.729 × 7, huỷ 2, phí giao 14.766. Đơn chốt thuế dòng 60.918; phần còn lại chính xác
/// 43.512,86 — hoá đơn làm tròn riêng phần đó (43.513 hay 43.514) không bị báo khác; thuế 10% → 8% thì báo.
#[test]
fn huy_mot_phan_gia_gom_thue_khong_bao_nham() {
    let lines = [order_line(true, 7, 95_729, 0, 60_918, 670_103, 2)];
    let row = order_of(&lines, 14_766, 1_342);
    let settled = settled_lines(&row, &lines);
    for tax in [43_513, 43_514] {
        let found = invoice(&[(Some("SKU-1"), tax, 478_645), (None, 1_342, 14_766)], 493_411);
        assert!(!differs_from_order(&settled, "sales", Some(&found)), "{tax}");
    }
    // Thuế suất 8% cho phần còn lại: 478.645 × 8/108 ≈ 35.455.
    let found = invoice(&[(Some("SKU-1"), 35_455, 478_645), (None, 1_342, 14_766)], 493_411);
    assert!(differs_from_order(&settled, "sales", Some(&found)));
}

/// Giá chưa thuế có giảm, GTGT 10%: 281.893 × 7, giảm 230.049, huỷ 2. Hoá đơn dựng phần còn lại 1.245.145 (giảm chia
/// theo số lượng, làm tròn xuống), thuế 124.515, tổng 1.369.660 — khớp đơn trong ngưỡng.
#[test]
fn huy_mot_phan_gia_chua_thue_co_giam_khong_bao_nham() {
    let lines = [order_line(false, 7, 281_893, 230_049, 174_320, 1_917_522, 2)];
    let row = order_of(&lines, 0, 0);
    let settled = settled_lines(&row, &lines);
    let found = invoice(&[(Some("SKU-1"), 124_515, 1_369_660)], 1_369_660);
    assert!(!differs_from_order(&settled, "sales", Some(&found)));
    // Tổng lệch 15.000 (vd lập sai dòng): báo.
    let found = invoice(&[(Some("SKU-1"), 124_515, 1_384_660)], 1_384_660);
    assert!(differs_from_order(&settled, "sales", Some(&found)));
}

/// Số rất lớn (tràn khi nhân): không panic — không so được thì báo khác để người vận hành xem.
#[test]
fn so_rat_lon_khong_panic() {
    let lines = [order_line(true, i64::MAX / 2, i64::MAX / 2, 0, i64::MAX / 4, i64::MAX / 2, 1)];
    let row = order_of(&lines, 0, 0);
    let settled = settled_lines(&row, &lines);
    let found = invoice(&[(Some("SKU-1"), 1, 1)], 1);
    assert!(differs_from_order(&settled, "sales", Some(&found)));
}
