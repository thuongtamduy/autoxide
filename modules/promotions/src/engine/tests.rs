//! Test thuần của bộ tính: không database (bộ tính không chạm database), nên chạy giống nhau ở
//! cả hai gate. Test qua service / REST ở `crate::tests`.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::collections::{BTreeSet, HashMap};

use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::tenancy::OrgId;
use uuid::Uuid;

use super::caps::{self, LineCap};
use super::{Draft, Input, Unlock, VariantFacts, compute};
use crate::api::{Cart, CartLine, GiftSelection, PromotionClass};
use crate::model::*;

fn vnd() -> Currency {
    Currency::parse("VND").unwrap()
}

fn at() -> Timestamp {
    Timestamp::from_utc(2026, 11, 18, 3, 0, 0).unwrap()
}

/// Chương trình mặc định: giảm tự động 10% mọi dòng của cửa hàng `org`.
fn program(org: Uuid) -> Program {
    Program {
        id: Uuid::now_v7(),
        org_id: org,
        kind: Kind::PercentOff,
        scope: Scope::Items,
        trigger: Trigger::Automatic,
        status: Status::Active,
        currency: "VND".to_owned(),
        value_bp: Some(1000),
        value_minor: None,
        min_order_minor: None,
        min_quantity: None,
        max_discount_minor: None,
        buy_quantity: None,
        get_quantity: None,
        reward_selection: None,
        reward_value_bp: None,
        max_applications: None,
        bundle_quantity: None,
        bundle_price_minor: None,
        max_add_on_quantity: None,
        gift_quantity: None,
        first_order_only: false,
        starts_at: at() - SignedDuration::from_hours(24),
        ends_at: at() + SignedDuration::from_hours(24),
        priority: 0,
        exclusivity: Exclusivity::None,
        combines_with: [PromotionClass::Product, PromotionClass::Order, PromotionClass::Shipping]
            .into_iter()
            .collect(),
        stop_after: false,
        usage_limit: None,
        usage_count: 0,
        per_customer_limit: None,
        per_customer_period: Period::Lifetime,
        daily_usage_limit: None,
        budget_minor: None,
        budget_used_minor: 0,
        return_usage_on_cancel: true,
        gift_return_policy: GiftPolicy::ReturnGift,
        program_kind: ProgramKind::Regular,
        public: true,
        texts: vec![("en".into(), "Sale".into(), "Terms".into())],
        conditions_targets: TargetSet::default(),
        reward_targets: TargetSet::default(),
        customer_groups: BTreeSet::new(),
        channels: BTreeSet::new(),
        conditions: Vec::new(),
        windows: Vec::new(),
        tiers: Vec::new(),
        flash_items: Vec::new(),
        gift_options: Vec::new(),
        funding: Vec::new(),
        sponsors: Default::default(),
    }
}

/// Giỏ thử: cửa hàng, các dòng (khoá, giá, số lượng), biến thể của cửa hàng.
struct Fixture {
    store: Uuid,
    cart: Cart,
    variants: HashMap<Uuid, VariantFacts>,
    unlocked: HashMap<Uuid, Unlock>,
    best: bool,
}

impl Fixture {
    fn new(lines: &[(&str, i64, i64)]) -> Self {
        let store = Uuid::now_v7();
        let mut cart = Cart::new(OrgId::from_uuid(store), vnd(), at());
        let mut variants = HashMap::new();
        for (key, price, quantity) in lines {
            let variant = Uuid::now_v7();
            cart.lines.push(CartLine::new(*key, variant, *quantity, Money::new(*price, vnd())));
            variants.insert(
                variant,
                VariantFacts {
                    product_id: Uuid::now_v7(),
                    org_id: store,
                    category_ids: Vec::new(),
                },
            );
        }
        Fixture { store, cart, variants, unlocked: HashMap::new(), best: false }
    }

    fn variant(&self, key: &str) -> Uuid {
        self.cart.lines.iter().find(|line| line.line_key == key).unwrap().variant_id
    }

    fn run(&self, programs: &[&Program]) -> Draft {
        let ancestors = [self.store];
        let input = Input {
            cart: &self.cart,
            variants: &self.variants,
            ancestors: &ancestors,
            local: jiff::tz::Offset::constant(7).to_datetime(self.cart.at.to_jiff()),
            best_for_buyer: self.best,
            max_vouchers_per_class: 1,
            unlocked: &self.unlocked,
            include_inactive: None,
        };
        compute(programs, &input)
    }
}

/// Tổng giảm theo `line_key`.
fn by_key(fixture: &Fixture, draft: &Draft) -> Vec<(String, i64)> {
    let mut out: Vec<(String, i64)> = Vec::new();
    for applied in &draft.applied {
        for part in &applied.lines {
            let key = fixture.cart.lines[part.line].line_key.clone();
            match out.iter_mut().find(|(found, _)| *found == key) {
                Some(found) => found.1 += part.amount,
                None => out.push((key, part.amount)),
            }
        }
    }
    out.sort();
    out
}

fn total(draft: &Draft) -> i64 {
    draft.applied.iter().map(super::Applied::total).sum()
}

#[test]
fn giam_phan_tram_tren_tong_dong() {
    let fixture = Fixture::new(&[("a", 33_333, 3)]);
    let promo = program(fixture.store);
    let draft = fixture.run(&[&promo]);
    // 10% của 99 999 = 9 999.9 → 10 000 (nửa lên, trên tổng dòng).
    assert_eq!(by_key(&fixture, &draft), vec![("a".to_owned(), 10_000)]);
}

#[test]
fn cung_gio_cung_ket_qua_khi_xao_tron_dong() {
    let mut fixture = Fixture::new(&[("a", 100_000, 1), ("b", 50_000, 3), ("c", 70_000, 2)]);
    let mut order = program(fixture.store);
    order.scope = Scope::Order;
    order.kind = Kind::AmountOff;
    order.value_minor = Some(33_333);
    let mut bxgy = program(fixture.store);
    bxgy.kind = Kind::BuyXGetY;
    bxgy.buy_quantity = Some(1);
    bxgy.get_quantity = Some(1);
    let first = by_key(&fixture, &fixture.run(&[&order, &bxgy]));
    for round in 0..100 {
        fixture.cart.lines.rotate_left(round % 3);
        if round % 2 == 0 {
            fixture.cart.lines.reverse();
        }
        assert_eq!(by_key(&fixture, &fixture.run(&[&bxgy, &order])), first);
    }
}

#[test]
fn exclusive_chan_cong_don() {
    let fixture = Fixture::new(&[("a", 100_000, 1)]);
    let mut high = program(fixture.store);
    high.priority = 10;
    high.exclusivity = Exclusivity::Class;
    let low = program(fixture.store);
    let mut order = program(fixture.store);
    order.scope = Scope::Order;
    order.kind = Kind::AmountOff;
    order.value_minor = Some(5_000);
    // `class`: chặn cùng lớp, lớp khác vẫn cộng.
    let draft = fixture.run(&[&high, &low, &order]);
    let ids: Vec<Uuid> = draft.applied.iter().map(|applied| applied.program).collect();
    assert_eq!(ids, vec![high.id, order.id]);
    // `global`: không cộng với gì.
    high.exclusivity = Exclusivity::Global;
    let draft = fixture.run(&[&high, &low, &order]);
    assert_eq!(draft.applied.len(), 1);
    assert_eq!(draft.applied[0].program, high.id);
}

#[test]
fn ma_tran_combines_with_hai_phia_cung_cho_phep() {
    let fixture = Fixture::new(&[("a", 100_000, 1)]);
    for (product_allows, order_allows, expected) in
        [(true, true, 2), (true, false, 1), (false, true, 1), (false, false, 1)]
    {
        let mut product = program(fixture.store);
        let mut order = program(fixture.store);
        order.scope = Scope::Order;
        order.kind = Kind::AmountOff;
        order.value_minor = Some(1_000);
        if !product_allows {
            product.combines_with.remove(&PromotionClass::Order);
        }
        if !order_allows {
            order.combines_with.remove(&PromotionClass::Product);
        }
        assert_eq!(fixture.run(&[&product, &order]).applied.len(), expected);
    }
}

#[test]
fn stop_after_dung_chuong_trinh_uu_tien_thap_hon() {
    let fixture = Fixture::new(&[("a", 100_000, 1)]);
    let mut first = program(fixture.store);
    first.priority = 5;
    first.stop_after = true;
    let second = program(fixture.store);
    let draft = fixture.run(&[&first, &second]);
    assert_eq!(draft.applied.len(), 1);
    // Cộng dồn tính trên phần còn lại: 10% rồi 10% của 90 000.
    let draft = fixture.run(&[&program(fixture.store), &second]);
    assert_eq!(total(&draft), 10_000 + 9_000);
}

#[test]
fn best_for_buyer_tat_dinh_va_lay_loi_nhat() {
    let mut fixture = Fixture::new(&[("a", 100_000, 2), ("b", 40_000, 1)]);
    fixture.best = true;
    let stack_a = program(fixture.store);
    let stack_b = program(fixture.store);
    let mut exclusive = program(fixture.store);
    exclusive.exclusivity = Exclusivity::Class;
    exclusive.value_bp = Some(2500);
    exclusive.priority = -1;
    let first = fixture.run(&[&stack_a, &stack_b, &exclusive]);
    assert_eq!(first.applied.len(), 1);
    assert_eq!(first.applied[0].program, exclusive.id);
    let expected = by_key(&fixture, &first);
    for round in 0..100 {
        fixture.cart.lines.swap(0, 1);
        let again = fixture.run(&[&exclusive, &stack_b, &stack_a]);
        assert_eq!(by_key(&fixture, &again), expected, "lần {round}");
    }
}

#[test]
fn giam_theo_don_phan_ve_dong_tong_khop() {
    let fixture = Fixture::new(&[("a", 33_333, 1), ("b", 33_333, 1), ("c", 33_334, 1)]);
    let mut order = program(fixture.store);
    order.scope = Scope::Order;
    order.kind = Kind::AmountOff;
    order.value_minor = Some(10_001);
    order.min_order_minor = Some(50_000);
    let draft = fixture.run(&[&order]);
    assert_eq!(total(&draft), 10_001);
    assert_eq!(draft.applied[0].lines.len(), 3);
}

#[test]
fn don_chua_du_nguong_bao_thieu_bao_nhieu() {
    let mut fixture = Fixture::new(&[("a", 380_000, 1)]);
    let mut order = program(fixture.store);
    order.scope = Scope::Order;
    order.kind = Kind::AmountOff;
    order.trigger = Trigger::Code;
    order.value_minor = Some(50_000);
    order.min_order_minor = Some(500_000);
    fixture
        .unlocked
        .insert(order.id, Unlock { code: Some("SALE".into()), code_id: None, voucher_id: None });
    let draft = fixture.run(&[&order]);
    assert!(draft.applied.is_empty());
    let reason = &draft.rejected[0].reason;
    assert_eq!(reason.code, "promotions.reason.add_more");
    assert_eq!(reason.params["amount"], "120,000 VND");
}

#[test]
fn khong_giam_xuong_duoi_0() {
    let fixture = Fixture::new(&[("a", 10_000, 1)]);
    let mut amount = program(fixture.store);
    amount.kind = Kind::AmountOff;
    amount.value_minor = Some(50_000);
    let mut more = program(fixture.store);
    more.kind = Kind::AmountOff;
    more.value_minor = Some(50_000);
    assert_eq!(total(&fixture.run(&[&amount, &more])), 10_000);
}

#[test]
fn mua_2_tinh_1_mon_re_nhat() {
    let fixture = Fixture::new(&[("a", 100_000, 1), ("b", 60_000, 1), ("c", 80_000, 1)]);
    let mut bxgy = program(fixture.store);
    bxgy.kind = Kind::BuyXGetY;
    bxgy.buy_quantity = Some(1);
    bxgy.get_quantity = Some(1);
    bxgy.reward_selection = Some(RewardSelection::Cheapest);
    // 3 món: một lần áp, món rẻ nhất (b) miễn phí.
    assert_eq!(by_key(&fixture, &fixture.run(&[&bxgy])), vec![("b".to_owned(), 60_000)]);
}

#[test]
fn mua_x_tang_y_mon_chi_dinh_thanh_hang_tang() {
    let fixture = Fixture::new(&[("a", 100_000, 2)]);
    let gift = Uuid::now_v7();
    let mut bxgy = program(fixture.store);
    bxgy.kind = Kind::BuyXGetY;
    bxgy.buy_quantity = Some(2);
    bxgy.get_quantity = Some(1);
    bxgy.reward_selection = Some(RewardSelection::Specified);
    bxgy.conditions_targets.insert(TargetType::Variant, fixture.variant("a"));
    bxgy.reward_targets.insert(TargetType::Variant, gift);
    let draft = fixture.run(&[&bxgy]);
    assert_eq!(draft.applied[0].gifts, vec![(gift, 1)]);
}

#[test]
fn mua_2_giam_50_mon_thu_hai() {
    let fixture = Fixture::new(&[("a", 100_000, 4)]);
    let mut bxgy = program(fixture.store);
    bxgy.kind = Kind::BuyXGetY;
    bxgy.buy_quantity = Some(1);
    bxgy.get_quantity = Some(1);
    bxgy.reward_selection = Some(RewardSelection::SameItem);
    bxgy.reward_value_bp = Some(5000);
    assert_eq!(total(&fixture.run(&[&bxgy])), 100_000);
}

#[test]
fn mien_phi_van_chuyen() {
    let mut fixture = Fixture::new(&[("a", 300_000, 1)]);
    fixture.cart.shipping_fee = Some(Money::new(30_000, vnd()));
    let mut ship = program(fixture.store);
    ship.kind = Kind::FreeShipping;
    ship.scope = Scope::Shipping;
    ship.value_minor = Some(25_000);
    let draft = fixture.run(&[&ship]);
    assert_eq!(draft.applied[0].shipping, 25_000);
    assert!(draft.applied[0].lines.is_empty());
}

#[test]
fn nhom_khach_va_kenh() {
    let mut fixture = Fixture::new(&[("a", 100_000, 1)]);
    let group = Uuid::now_v7();
    let mut promo = program(fixture.store);
    promo.customer_groups.insert(group);
    promo.channels.insert("shopee".to_owned());
    assert!(fixture.run(&[&promo]).applied.is_empty());
    fixture.cart.customer_groups.push(group);
    assert!(fixture.run(&[&promo]).applied.is_empty());
    fixture.cart.channel = Some("shopee".to_owned());
    assert_eq!(fixture.run(&[&promo]).applied.len(), 1);
}

#[test]
fn bac_thang_lay_bac_cao_nhat() {
    let fixture = Fixture::new(&[("a", 100_000, 3)]);
    let mut tiered = program(fixture.store);
    tiered.kind = Kind::Tiered;
    tiered.tiers = vec![
        Tier {
            threshold_kind: ThresholdKind::Quantity,
            threshold: 2,
            value_bp: Some(1000),
            value_minor: None,
        },
        Tier {
            threshold_kind: ThresholdKind::Quantity,
            threshold: 3,
            value_bp: Some(1500),
            value_minor: None,
        },
        Tier {
            threshold_kind: ThresholdKind::Quantity,
            threshold: 5,
            value_bp: Some(2000),
            value_minor: None,
        },
    ];
    assert_eq!(total(&fixture.run(&[&tiered])), 45_000);
}

#[test]
fn combo_dong_gia_lap_lai() {
    let fixture = Fixture::new(&[("a", 100_000, 4), ("b", 80_000, 3)]);
    let mut combo = program(fixture.store);
    combo.kind = Kind::BundlePrice;
    combo.bundle_quantity = Some(3);
    combo.bundle_price_minor = Some(199_000);
    combo.max_applications = Some(2);
    // 7 món → 2 combo: (100k×3) và (100k + 80k×2); giảm 101k + 61k.
    assert_eq!(total(&fixture.run(&[&combo])), 101_000 + 61_000);
}

#[test]
fn mua_kem_deal_soc() {
    let fixture = Fixture::new(&[("main", 500_000, 1), ("deal", 100_000, 3)]);
    let mut add_on = program(fixture.store);
    add_on.kind = Kind::AddOn;
    add_on.conditions_targets.insert(TargetType::Variant, fixture.variant("main"));
    add_on.max_add_on_quantity = Some(2);
    add_on.flash_items = vec![FlashItem {
        variant_id: fixture.variant("deal"),
        sale_price_minor: Some(10_000),
        value_bp: None,
        quantity_limit: None,
        quantity_used: 0,
        per_customer_quantity_limit: None,
    }];
    assert_eq!(by_key(&fixture, &fixture.run(&[&add_on])), vec![("deal".to_owned(), 180_000)]);
}

#[test]
fn chon_qua() {
    let mut fixture = Fixture::new(&[("a", 600_000, 1)]);
    let (gift_a, gift_b) = (Uuid::now_v7(), Uuid::now_v7());
    let mut choice = program(fixture.store);
    choice.kind = Kind::GiftChoice;
    choice.min_order_minor = Some(500_000);
    choice.gift_quantity = Some(1);
    choice.gift_options = vec![
        GiftOption { variant_id: gift_a, stock_limit: Some(1), quantity_used: 1 },
        GiftOption { variant_id: gift_b, stock_limit: None, quantity_used: 0 },
    ];
    // Chưa chọn: gợi ý (quà hết suất không có trong danh sách), không chặn.
    let draft = fixture.run(&[&choice]);
    assert!(draft.applied.is_empty());
    assert_eq!(draft.choices[0].options, vec![gift_b]);
    fixture.cart.gift_selections.push(GiftSelection {
        promotion_id: choice.id,
        variant_id: gift_b,
        quantity: 3,
    });
    let draft = fixture.run(&[&choice]);
    assert_eq!(draft.applied[0].gifts, vec![(gift_b, 1)]);
}

#[test]
fn flash_sale_theo_suat_con() {
    let fixture = Fixture::new(&[("a", 200_000, 5)]);
    let mut flash = program(fixture.store);
    flash.kind = Kind::FlashSale;
    flash.flash_items = vec![FlashItem {
        variant_id: fixture.variant("a"),
        sale_price_minor: Some(99_000),
        value_bp: None,
        quantity_limit: Some(10),
        quantity_used: 8,
        per_customer_quantity_limit: Some(3),
    }];
    // Còn 2 suất: 2 đơn vị giá sốc.
    assert_eq!(total(&fixture.run(&[&flash])), 2 * 101_000);
    flash.flash_items[0].quantity_used = 10;
    assert!(fixture.run(&[&flash]).applied.is_empty());
}

#[test]
fn theo_phuong_thuc_thanh_toan() {
    let mut fixture = Fixture::new(&[("a", 100_000, 1)]);
    let mut promo = program(fixture.store);
    promo.conditions.push(Condition {
        kind: ConditionKind::PaymentMethod,
        operator: Operator::In,
        values: vec!["vnpay".into()],
    });
    assert!(fixture.run(&[&promo]).applied.is_empty());
    fixture.cart.payment_method = Some("VNPAY".into());
    assert_eq!(fixture.run(&[&promo]).applied.len(), 1);
}

#[test]
fn khung_gio_theo_mui_gio_cua_hang() {
    let fixture = Fixture::new(&[("a", 100_000, 1)]);
    // `at` = 03:00 UTC thứ Tư = 10:00 giờ Việt Nam; giờ vàng 9:00–11:00 thứ Tư.
    let mut promo = program(fixture.store);
    promo.windows = vec![Window { weekdays: 1 << 2, start_minute: 9 * 60, end_minute: 11 * 60 }];
    assert_eq!(fixture.run(&[&promo]).applied.len(), 1);
    promo.windows = vec![Window { weekdays: 1 << 2, start_minute: 3 * 60, end_minute: 4 * 60 }];
    assert!(fixture.run(&[&promo]).applied.is_empty());
}

#[test]
fn khach_moi_va_don_dau() {
    let mut fixture = Fixture::new(&[("a", 100_000, 1)]);
    let mut promo = program(fixture.store);
    promo.first_order_only = true;
    let mut returning = program(fixture.store);
    returning.conditions.push(Condition {
        kind: ConditionKind::ReturningCustomer,
        operator: Operator::In,
        values: Vec::new(),
    });
    assert_eq!(fixture.run(&[&promo, &returning]).applied[0].program, returning.id);
    fixture.cart.first_order = true;
    let draft = fixture.run(&[&promo, &returning]);
    assert_eq!(draft.applied.len(), 1);
    assert_eq!(draft.applied[0].program, promo.id);
}

#[test]
fn loai_hang_dang_giam() {
    let mut fixture = Fixture::new(&[("a", 100_000, 1), ("b", 100_000, 1)]);
    fixture.cart.lines[0].has_compare_at = true;
    let mut promo = program(fixture.store);
    promo.conditions.push(Condition {
        kind: ConditionKind::ExcludeDiscounted,
        operator: Operator::In,
        values: Vec::new(),
    });
    assert_eq!(by_key(&fixture, &fixture.run(&[&promo])), vec![("b".to_owned(), 10_000)]);
}

#[test]
fn voucher_shop_va_voucher_san_cung_gio() {
    let mut fixture = Fixture::new(&[("a", 200_000, 1)]);
    let platform = fixture.store;
    let seller = Uuid::now_v7();
    let variant = fixture.variant("a");
    fixture.variants.get_mut(&variant).unwrap().org_id = seller;
    let mut shop = program(seller);
    shop.trigger = Trigger::Voucher;
    shop.kind = Kind::AmountOff;
    shop.value_minor = Some(20_000);
    let mut shop_second = shop.clone();
    shop_second.id = Uuid::now_v7();
    let mut platform_voucher = program(platform);
    platform_voucher.trigger = Trigger::Voucher;
    platform_voucher.kind = Kind::AmountOff;
    platform_voucher.value_minor = Some(30_000);
    for promo in [&shop, &shop_second, &platform_voucher] {
        fixture.unlocked.insert(
            promo.id,
            Unlock { code: None, code_id: None, voucher_id: Some(Uuid::now_v7()) },
        );
    }
    let draft = fixture.run(&[&shop, &shop_second, &platform_voucher]);
    // Một voucher của người bán + một voucher sàn; voucher thứ hai của cùng người bán bị từ chối.
    assert_eq!(total(&draft), 50_000);
    assert_eq!(draft.rejected.len(), 1);
    assert_eq!(draft.rejected[0].reason.code, "promotions.reason.too_many_vouchers");
}

#[test]
fn tran_cong_ca_voucher_san_va_shop_va_cat_khoan_ap_sau() {
    let fixture = Fixture::new(&[("a", 100_000, 2)]);
    let mut big = program(fixture.store);
    big.value_bp = Some(4000);
    let mut order = program(fixture.store);
    order.scope = Scope::Order;
    order.kind = Kind::AmountOff;
    order.value_minor = Some(40_000);
    let mut draft = fixture.run(&[&big, &order]);
    assert_eq!(total(&draft), 80_000 + 40_000);
    // Trần 50% × giá tham chiếu 100 000 × 2 = 100 000: khoản lớp `order` (áp sau) bị cắt 20 000.
    let withheld =
        caps::apply(&mut draft, &[LineCap { law: Some(100_000), floor: None }], &[false]);
    assert!(withheld.is_empty());
    assert_eq!(total(&draft), 100_000);
    let order_part = &draft.applied[1].lines[0];
    assert_eq!((order_part.amount, order_part.capped_by_law), (20_000, true));
}

#[test]
fn san_gia_cat_khoan_giam_va_thieu_gia_tham_chieu_thi_giu_lai() {
    let fixture = Fixture::new(&[("a", 100_000, 1)]);
    let mut promo = program(fixture.store);
    promo.value_bp = Some(3000);
    let mut draft = fixture.run(&[&promo]);
    caps::apply(&mut draft, &[LineCap { law: None, floor: Some(25_000) }], &[false]);
    assert!(draft.applied[0].lines[0].capped_by_floor);
    assert_eq!(total(&draft), 25_000);
    let mut draft = fixture.run(&[&promo]);
    let withheld = caps::apply(&mut draft, &[LineCap { law: Some(0), floor: None }], &[true]);
    assert!(draft.applied.is_empty());
    assert_eq!(withheld[0].reason.code, "promotions.reason.no_reference_price");
}

#[test]
fn ma_sai_khong_chan_ca_gio_va_ngan_sach_khong_ap_mot_phan() {
    let mut fixture = Fixture::new(&[("a", 100_000, 1)]);
    let auto = program(fixture.store);
    let mut budget = program(fixture.store);
    budget.trigger = Trigger::Code;
    budget.budget_minor = Some(15_000);
    budget.budget_used_minor = 10_000;
    fixture
        .unlocked
        .insert(budget.id, Unlock { code: Some("B".into()), code_id: None, voucher_id: None });
    let draft = fixture.run(&[&auto, &budget]);
    assert_eq!(draft.applied.len(), 1);
    assert_eq!(draft.rejected[0].reason.code, "promotions.reason.budget_exhausted");
}
