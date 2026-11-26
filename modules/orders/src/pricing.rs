//! Báo giá ở máy chủ: một hàm duy nhất cho giỏ, đặt hàng và quầy. Giá, giảm, phí giao, thuế luôn
//! tính lại từ `catalog`, `promotions`, `shipping`, `tax` — không bao giờ tin số client gửi.
//!
//! Thứ tự: biến thể + giá (`catalog`) → tồn bán được (`inventory`, chỉ để báo) → phí giao (`shipping`,
//! trên tạm tính trước giảm) → khuyến mại (`promotions`, biết phí giao để giảm phí) → thuế (`tax`,
//! trên tiền sau giảm). Module nào chưa cài thì bước đó bỏ qua (không giảm, không phí, thuế 0).
//!
//! `quote_token` = `<hạn unix>.<dấu vân tay>`: dấu vân tay tính trên nội dung báo giá (dòng, tiền,
//! cách giao, kết quả khuyến mại). Đặt hàng tính lại và so: khác → `orders.quote_changed`. Sửa hạn
//! trong token không có lợi gì — giá vẫn tính lại ở máy chủ.

use autoxide_kernel::time::Timestamp;
use jiff::SignedDuration;
use std::collections::HashMap;
use std::sync::Arc;

use autoxide_catalog::api::{Catalog, PriceQuery, ProductStatus, SaleMode, VariantSnapshot};
use autoxide_inventory::api::Inventory;
use autoxide_kernel::error::Error;
use autoxide_kernel::money::{Currency, Money};
use autoxide_kernel::reference;
use autoxide_kernel::tenancy::{OrgId, TenantContext};
use autoxide_promotions::api::{
    Cart as PromotionCart, CartLine as PromotionLine, Evaluation, GiftSelection, Promotions,
};
use autoxide_shipping::api::{Address, QuoteItem, QuoteRequest, ShippingOption, ShippingRates};
use autoxide_tax::api::{PriceMode, TaxComponent, TaxEngine, TaxQuoteRequest, TaxableLine};
use serde_json::json;
use uuid::Uuid;

use crate::api::BuyerProfile;
use crate::errors;

/// Khoá dòng của mã thuế phí giao trong lời gọi `TaxEngine`.
const SHIPPING_LINE: &str = "shipping";

/// Các module bên ngoài báo giá cần. Chỉ `catalog` bắt buộc.
#[derive(Clone)]
pub(crate) struct Sources {
    pub catalog: Arc<dyn Catalog>,
    pub inventory: Option<Arc<dyn Inventory>>,
    pub promotions: Option<Arc<dyn Promotions>>,
    pub shipping: Option<Arc<dyn ShippingRates>>,
    pub tax: Option<Arc<dyn TaxEngine>>,
}

/// Đầu vào của một lần báo giá.
#[derive(Debug, Clone)]
pub(crate) struct PriceInput {
    pub org_id: OrgId,
    pub currency: Currency,
    pub locale: String,
    /// (biến thể, số lượng), theo thứ tự hiển thị.
    pub lines: Vec<(Uuid, i64)>,
    pub codes: Vec<String>,
    pub voucher_ids: Vec<Uuid>,
    pub gift_selections: Vec<GiftSelection>,
    pub buyer_id: Option<Uuid>,
    pub buyer_keys: Vec<String>,
    pub profile: BuyerProfile,
    pub destination: Option<Address>,
    /// Người nhận (tên, số điện thoại) khi đã biết: chuyển cho hãng vận chuyển lúc báo phí.
    pub recipient: Option<(String, String)>,
    pub shipping_option: Option<String>,
    pub payment_method: Option<String>,
    /// `web` / `app` / `pos`.
    pub client: Option<String>,
    pub first_order: bool,
    pub at: Timestamp,
}

/// Vì sao một dòng chưa đặt được.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LineProblem {
    /// Biến thể không có, ngừng bán, hay chưa có giá ở cửa hàng.
    Unavailable,
    /// Hàng chỉ bán khi còn mà không đủ: còn `available`.
    OutOfStock { available: i64 },
    /// Quá hạn nhận đặt trước.
    PreorderClosed,
}

/// Một dòng đã báo giá.
#[derive(Debug, Clone)]
pub(crate) struct QuotedLine {
    /// Khoá dòng (ID biến thể; quà: `gift:<promotion>:<biến thể>`).
    pub key: String,
    pub variant_id: Uuid,
    pub variant: Option<VariantSnapshot>,
    pub quantity: i64,
    /// Giá một đơn vị trước giảm (theo `tax_inclusive` của bảng giá); 0 với quà.
    pub unit_price: Money,
    pub compare_at: Option<Money>,
    pub tax_inclusive: bool,
    /// `unit_price × quantity`.
    pub subtotal: Money,
    pub discount: Money,
    /// Sau giảm, cùng chế độ giá với `unit_price`.
    pub net: Money,
    pub tax: Money,
    /// Thành phần thuế của dòng như `TaxEngine` trả.
    pub taxes: Vec<TaxComponent>,
    /// Người mua trả cho dòng: sau giảm, gồm thuế.
    pub gross: Money,
    /// Dòng quà do khuyến mại thêm.
    pub gift_of: Option<Uuid>,
    pub problem: Option<LineProblem>,
}

/// Báo giá của cả giỏ.
#[derive(Debug, Clone)]
pub(crate) struct Quote {
    pub lines: Vec<QuotedLine>,
    pub subtotal: Money,
    pub discount: Money,
    pub shipping_fee: Money,
    pub shipping_discount: Money,
    pub tax: Money,
    pub total: Money,
    pub requires_shipping: bool,
    pub shipping_options: Vec<ShippingOption>,
    pub shipping: Option<ShippingOption>,
    /// Người mua chọn cách giao không còn dùng được.
    pub shipping_option_unavailable: bool,
    /// Còn thiếu bao nhiêu để được miễn phí giao (0: đã được), khi cách giao có ngưỡng.
    pub free_shipping_remaining: Option<Money>,
    pub evaluation: Option<Evaluation>,
    pub token: String,
    pub expires_at: Timestamp,
}

impl Quote {
    /// Giỏ có dòng hàng đặt trước (D-204).
    pub(crate) fn has_preorder(&self) -> bool {
        self.lines.iter().any(|line| {
            line.gift_of.is_none()
                && line
                    .variant
                    .as_ref()
                    .is_some_and(|variant| variant.sale_mode == SaleMode::Preorder)
        })
    }

    /// Dấu vân tay trong `quote_token`.
    pub(crate) fn fingerprint(&self) -> &str {
        token_fingerprint(&self.token)
    }
}

/// Dấu vân tay của một `quote_token` (token hỏng: chuỗi rỗng).
pub(crate) fn token_fingerprint(token: &str) -> &str {
    token.split_once('.').map_or("", |(_, fingerprint)| fingerprint)
}

/// Hạn và dấu vân tay trong `quote_token`; token hỏng → `None`.
pub(crate) fn parse_token(token: &str) -> Option<(Timestamp, &str)> {
    let (expires, fingerprint) = token.split_once('.')?;
    let expires = Timestamp::from_unix_seconds(expires.parse().ok()?)?;
    Some((expires, fingerprint))
}

fn add(a: Money, b: Money) -> Result<Money, Error> {
    a.checked_add(b)
}

/// Báo giá `input`; `ttl_minutes` là tuổi của `quote_token`.
pub(crate) async fn quote(
    sources: &Sources,
    tenant: &TenantContext,
    input: &PriceInput,
    ttl_minutes: i64,
) -> Result<Quote, Error> {
    let currency = input.currency;
    let zero = Money::zero(currency);
    let ids: Vec<Uuid> = input.lines.iter().map(|(variant, _)| *variant).collect();
    let mut lines = priced_lines(sources, tenant, input, &ids).await?;
    mark_stock(sources, tenant, input.org_id, &mut lines).await?;

    let subtotal = Money::sum(currency, lines.iter().map(|line| line.subtotal))?;
    let requires_shipping = lines.iter().any(|line| {
        line.variant.as_ref().is_some_and(|variant| variant.requires_shipping)
            && line.problem.is_none()
    });

    // Phí giao: trên tạm tính trước giảm (ngưỡng miễn phí của cửa hàng so với giá niêm yết).
    let mut shipping_options = Vec::new();
    let mut chosen = None;
    let mut shipping_option_unavailable = false;
    if requires_shipping && let Some(rates) = &sources.shipping {
        let items = lines
            .iter()
            .filter(|line| line.problem.is_none())
            .filter_map(|line| line.variant.as_ref().map(|variant| (line, variant)))
            .map(|(line, variant)| {
                QuoteItem::new(line.variant_id, line.quantity)
                    .sku(variant.sku.clone())
                    .requires_shipping(variant.requires_shipping)
                    .weight(variant.weight_grams)
                    .dimensions(variant.length_mm, variant.width_mm, variant.height_mm)
            })
            .collect();
        let mut request =
            QuoteRequest::new(input.org_id, items, subtotal, input.at).locale(input.locale.clone());
        if let Some(destination) = &input.destination {
            request = request.destination(destination.clone());
        }
        if let Some((name, phone)) = &input.recipient {
            request = request.recipient(name.clone(), phone.clone());
        }
        let quoted = rates.quote(tenant, &request).await?;
        shipping_options = quoted.options;
        if let Some(key) = &input.shipping_option {
            chosen = shipping_options.iter().find(|option| &option.option_key == key).cloned();
            shipping_option_unavailable = chosen.is_none();
        }
    }
    let shipping_fee = chosen.as_ref().map_or(zero, |option| option.amount);

    // Khuyến mại.
    let evaluation = match &sources.promotions {
        Some(promotions) if lines.iter().any(|line| line.problem.is_none()) => {
            let cart = promotion_cart(input, &lines, chosen.as_ref(), shipping_fee);
            Some(promotions.evaluate(tenant, &cart).await?)
        }
        _ => None,
    };
    if let Some(evaluation) = &evaluation {
        for line in &mut lines {
            let discount = evaluation.line_discount(&line.key);
            // Khoản giảm không bao giờ vượt tiền của dòng.
            let discount =
                if discount.compare(line.subtotal)?.is_gt() { line.subtotal } else { discount };
            line.discount = discount;
            line.net = line.subtotal.checked_sub(discount)?;
        }
        add_gifts(sources, tenant, input, evaluation, &mut lines).await?;
    }
    let shipping_discount = evaluation.as_ref().map_or(Ok(zero), |evaluation| {
        let discount = evaluation.shipping_discount;
        Ok::<_, Error>(if discount.compare(shipping_fee)?.is_gt() {
            shipping_fee
        } else {
            discount
        })
    })?;
    let shipping_net = shipping_fee.checked_sub(shipping_discount)?;

    // Thuế, trên tiền sau giảm.
    let shipping_tax =
        apply_tax(sources, tenant, input, &mut lines, chosen.as_ref(), shipping_net).await?;

    let discount = Money::sum(currency, lines.iter().map(|line| line.discount))?;
    let lines_tax = Money::sum(currency, lines.iter().map(|line| line.tax))?;
    let lines_gross = Money::sum(
        currency,
        lines.iter().filter(|line| line.problem.is_none()).map(|line| line.gross),
    )?;
    let tax = add(lines_tax, shipping_tax.0)?;
    let total = add(lines_gross, shipping_tax.1)?;

    let free_shipping_remaining = free_shipping_remaining(
        chosen.as_ref(),
        &shipping_options,
        subtotal.checked_sub(discount)?,
    )?;
    let expires_at = input.at + SignedDuration::from_mins(ttl_minutes);
    let mut quote = Quote {
        lines,
        subtotal,
        discount,
        shipping_fee,
        shipping_discount,
        tax,
        total,
        requires_shipping,
        shipping_options,
        shipping: chosen,
        shipping_option_unavailable,
        free_shipping_remaining,
        evaluation,
        token: String::new(),
        expires_at,
    };
    let fingerprint = fingerprint(&quote)?;
    quote.token = format!("{}.{fingerprint}", expires_at.as_unix_seconds());
    Ok(quote)
}

/// Dòng có biến thể và giá từ `catalog`.
async fn priced_lines(
    sources: &Sources,
    tenant: &TenantContext,
    input: &PriceInput,
    ids: &[Uuid],
) -> Result<Vec<QuotedLine>, Error> {
    let zero = Money::zero(input.currency);
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let variants: HashMap<Uuid, VariantSnapshot> = sources
        .catalog
        .variants(tenant, ids, Some(&input.locale))
        .await?
        .into_iter()
        .map(|variant| (variant.variant_id, variant))
        .collect();
    let mut query = PriceQuery::new(input.org_id, input.currency, input.at, ids.to_vec());
    if let Some(group) = &input.profile.price_group {
        query = query.for_customer_group(group.clone());
    }
    if let Some(client) = input.client.as_deref().filter(|client| *client == "pos") {
        query = query.for_channel(client);
    }
    let prices: HashMap<Uuid, _> = sources
        .catalog
        .prices(tenant, &query)
        .await?
        .into_iter()
        .map(|price| (price.variant_id, price))
        .collect();
    let mut lines = Vec::with_capacity(input.lines.len());
    for (variant_id, quantity) in &input.lines {
        let variant = variants.get(variant_id).cloned();
        let price = prices.get(variant_id);
        let sellable = variant.as_ref().is_some_and(|variant| {
            variant.status == ProductStatus::Active && variant.variant_active
        });
        let preorder_closed = variant.as_ref().is_some_and(|variant| {
            variant.sale_mode == SaleMode::Preorder
                && variant.preorder_ends_at.is_some_and(|ends| ends <= input.at)
        });
        let problem = match (sellable, price) {
            (true, Some(_)) if preorder_closed => Some(LineProblem::PreorderClosed),
            (true, Some(_)) => None,
            _ => Some(LineProblem::Unavailable),
        };
        let unit_price = price.map_or(zero, |price| price.amount);
        if unit_price.currency() != input.currency {
            return Err(Error::internal("catalog returned a price in another currency"));
        }
        let subtotal = if problem.is_none() { unit_price.checked_mul(*quantity)? } else { zero };
        lines.push(QuotedLine {
            key: variant_id.to_string(),
            variant_id: *variant_id,
            variant,
            quantity: *quantity,
            unit_price,
            compare_at: price.and_then(|price| price.compare_at),
            tax_inclusive: price.is_none_or(|price| price.tax_inclusive),
            subtotal,
            discount: zero,
            net: subtotal,
            tax: zero,
            taxes: Vec::new(),
            gross: subtotal,
            gift_of: None,
            problem,
        });
    }
    Ok(lines)
}

/// Đánh dấu dòng hàng "chỉ bán khi còn" không đủ tồn (chỉ báo, chưa giữ chỗ).
async fn mark_stock(
    sources: &Sources,
    tenant: &TenantContext,
    org: OrgId,
    lines: &mut [QuotedLine],
) -> Result<(), Error> {
    let Some(inventory) = &sources.inventory else { return Ok(()) };
    let tracked: Vec<Uuid> = lines
        .iter()
        .filter(|line| line.problem.is_none())
        .filter(|line| {
            line.variant.as_ref().is_some_and(|variant| {
                variant.track_inventory && variant.sale_mode == SaleMode::InStock
            })
        })
        .map(|line| line.variant_id)
        .collect();
    if tracked.is_empty() {
        return Ok(());
    }
    let mut available: HashMap<Uuid, i64> = HashMap::new();
    for row in inventory.availability(tenant, org, &tracked).await? {
        *available.entry(row.variant_id).or_default() += row.available.max(0);
    }
    for line in lines.iter_mut().filter(|line| tracked.contains(&line.variant_id)) {
        let left = available.get(&line.variant_id).copied().unwrap_or(0);
        if left < line.quantity {
            line.problem = Some(LineProblem::OutOfStock { available: left });
            line.subtotal = Money::zero(line.subtotal.currency());
            line.net = line.subtotal;
            line.gross = line.subtotal;
        }
    }
    Ok(())
}

/// Giỏ cho `promotions` (dữ liệu điều kiện: người mua, phương thức, hãng, tỉnh — D-222).
fn promotion_cart(
    input: &PriceInput,
    lines: &[QuotedLine],
    chosen: Option<&ShippingOption>,
    shipping_fee: Money,
) -> PromotionCart {
    let mut cart = PromotionCart::new(input.org_id, input.currency, input.at);
    cart.locale = input.locale.clone();
    cart.buyer_id = input.buyer_id;
    cart.buyer_keys = input.buyer_keys.clone();
    cart.customer_groups = input.profile.groups.clone();
    cart.membership_tier = input.profile.membership_tier.clone();
    cart.client = input.client.clone();
    cart.payment_method = input.payment_method.clone();
    cart.carrier = chosen.and_then(|option| option.carrier_code.clone());
    cart.ship_to_province_code =
        input.destination.as_ref().map(|address| address.province_code.clone());
    cart.first_order = input.first_order;
    cart.codes = input.codes.clone();
    // Voucher chỉ của người đăng nhập (D-222).
    if input.buyer_id.is_some() {
        cart.voucher_ids = input.voucher_ids.clone();
    }
    cart.gift_selections = input.gift_selections.clone();
    cart.shipping_fee = chosen.map(|_| shipping_fee);
    for line in lines.iter().filter(|line| line.problem.is_none()) {
        let mut promotion_line =
            PromotionLine::new(line.key.clone(), line.variant_id, line.quantity, line.unit_price);
        promotion_line.has_compare_at = line.compare_at.is_some();
        cart.lines.push(promotion_line);
    }
    cart
}

/// Thêm dòng quà (giá 0) do khuyến mại trả về.
async fn add_gifts(
    sources: &Sources,
    tenant: &TenantContext,
    input: &PriceInput,
    evaluation: &Evaluation,
    lines: &mut Vec<QuotedLine>,
) -> Result<(), Error> {
    if evaluation.gifts.is_empty() {
        return Ok(());
    }
    let ids: Vec<Uuid> = evaluation.gifts.iter().map(|gift| gift.variant_id).collect();
    let variants: HashMap<Uuid, VariantSnapshot> = sources
        .catalog
        .variants(tenant, &ids, Some(&input.locale))
        .await?
        .into_iter()
        .map(|variant| (variant.variant_id, variant))
        .collect();
    // Giá niêm yết của quà: ghi vào `compare_at` của dòng (giá trị quà, trừ vào tiền hoàn khi trả
    // món điều kiện mà chương trình chọn `deduct_gift_value` — D-222).
    let query = PriceQuery::new(input.org_id, input.currency, input.at, ids.clone());
    let values: HashMap<Uuid, Money> = sources
        .catalog
        .prices(tenant, &query)
        .await?
        .into_iter()
        .filter(|price| price.amount.currency() == input.currency)
        .map(|price| (price.variant_id, price.amount))
        .collect();
    let zero = Money::zero(input.currency);
    for gift in &evaluation.gifts {
        let variant = variants.get(&gift.variant_id).cloned();
        let problem = variant.is_none().then_some(LineProblem::Unavailable);
        lines.push(QuotedLine {
            key: format!("gift:{}:{}", gift.promotion_id, gift.variant_id),
            variant_id: gift.variant_id,
            variant,
            quantity: gift.quantity,
            unit_price: zero,
            compare_at: values.get(&gift.variant_id).copied(),
            tax_inclusive: true,
            subtotal: zero,
            discount: zero,
            net: zero,
            tax: zero,
            taxes: Vec::new(),
            gross: zero,
            gift_of: Some(gift.promotion_id),
            problem,
        });
    }
    Ok(())
}

/// Ngày thuế của đơn đặt lúc `at` (LG-2): ngày theo múi giờ của vùng thuế `jurisdiction` của người bán —
/// đã có hồ sơ thuế nên hỏi thẳng vùng, không tra hồ sơ lần nữa trên đường nóng; vùng không khai múi giờ thì
/// theo `TaxEngine::tax_date` (múi giờ của nút, rồi UTC).
pub(crate) async fn tax_point(
    engine: &dyn TaxEngine,
    tenant: &TenantContext,
    jurisdiction: &str,
    org: OrgId,
    at: Timestamp,
) -> Result<autoxide_kernel::time::Date, Error> {
    match engine.tax_date_in(jurisdiction, at).await? {
        Some(day) => Ok(day),
        None => engine.tax_date(tenant, org, at).await,
    }
}

/// Tính thuế cho dòng (ghi vào `line.tax`, `line.gross`) và cho phí giao; trả (thuế, tiền người mua
/// trả) của phí giao.
async fn apply_tax(
    sources: &Sources,
    tenant: &TenantContext,
    input: &PriceInput,
    lines: &mut [QuotedLine],
    chosen: Option<&ShippingOption>,
    shipping_net: Money,
) -> Result<(Money, Money), Error> {
    let zero = Money::zero(input.currency);
    // Mặc định: không thuế, người mua trả đúng tiền sau giảm; engine thuế ghi đè bên dưới.
    for line in lines.iter_mut() {
        line.gross = if line.problem.is_none() { line.net } else { zero };
    }
    let Some(engine) = &sources.tax else {
        return Ok((zero, shipping_net));
    };
    // Nút chưa có hồ sơ thuế (chưa cấu hình thuế): chưa tính thuế, như `catalog` (bước 41).
    let Some(profile) = engine.profile(tenant, input.org_id).await? else {
        return Ok((zero, shipping_net));
    };
    let jurisdiction = profile.jurisdiction;
    // Ngày thuế theo múi giờ của vùng thuế người bán (giờ Việt Nam), không theo UTC (LG-2): đơn đặt lúc
    // 00:30 ngày 01/01/2027 giờ Việt Nam đã hết GTGT 8%.
    let tax_point =
        tax_point(engine.as_ref(), tenant, &jurisdiction, input.org_id, input.at).await?;
    let mut shipping = (zero, shipping_net);
    // Một lần gọi cho mỗi chế độ giá (bảng giá gồm thuế hay chưa); phí giao coi như gồm thuế.
    for inclusive in [true, false] {
        let mut taxable: Vec<TaxableLine> = lines
            .iter()
            .filter(|line| line.problem.is_none() && line.tax_inclusive == inclusive)
            .filter_map(|line| {
                let category = line.variant.as_ref()?.tax_category.clone();
                Some(TaxableLine::new(line.key.clone(), category, line.net))
            })
            .collect();
        let shipping_line = inclusive && chosen.is_some() && shipping_net.is_positive();
        if let Some(option) = chosen.filter(|_| shipping_line) {
            taxable.push(TaxableLine::new(
                SHIPPING_LINE,
                option.tax_category.clone(),
                shipping_net,
            ));
        }
        if taxable.is_empty() {
            continue;
        }
        let mode = if inclusive { PriceMode::Inclusive } else { PriceMode::Exclusive };
        let request = TaxQuoteRequest::new(
            jurisdiction.clone(),
            tax_point,
            input.org_id,
            mode,
            input.currency,
            taxable,
        );
        let taxed = engine.quote(tenant, request).await?;
        for taxed_line in taxed.lines {
            let line_tax = Money::sum(input.currency, taxed_line.taxes.iter().map(|t| t.amount))?;
            if taxed_line.line_id == SHIPPING_LINE {
                shipping = (line_tax, taxed_line.gross);
            } else if let Some(line) = lines.iter_mut().find(|line| line.key == taxed_line.line_id)
            {
                line.tax = line_tax;
                line.taxes = taxed_line.taxes.clone();
                line.gross = taxed_line.gross;
            }
        }
    }
    Ok(shipping)
}

/// Còn thiếu bao nhiêu (sau giảm) để được miễn phí giao: theo cách giao đã chọn, chưa chọn thì
/// theo ngưỡng thấp nhất trong các cách giao có ngưỡng (D-201).
fn free_shipping_remaining(
    chosen: Option<&ShippingOption>,
    options: &[ShippingOption],
    after_discount: Money,
) -> Result<Option<Money>, Error> {
    let threshold = match chosen {
        Some(option) => option.free_over,
        None => options
            .iter()
            .filter_map(|option| option.free_over)
            .min_by_key(|threshold| threshold.minor()),
    };
    let Some(threshold) = threshold else { return Ok(None) };
    let remaining = threshold.checked_sub(after_discount)?;
    Ok(Some(if remaining.is_positive() {
        remaining
    } else {
        Money::zero(after_discount.currency())
    }))
}

/// Dấu vân tay của nội dung báo giá (không gồm token, hạn).
fn fingerprint(quote: &Quote) -> Result<String, Error> {
    let lines: Vec<_> = quote
        .lines
        .iter()
        .map(|line| {
            json!([
                line.key,
                line.quantity,
                line.unit_price,
                line.discount,
                line.tax,
                line.gross,
                line.problem.is_some()
            ])
        })
        .collect();
    reference::fingerprint(&json!({
        "lines": lines,
        "subtotal": quote.subtotal,
        "discount": quote.discount,
        "shipping": quote.shipping.as_ref().map(|option| (&option.option_key, option.amount)),
        "shipping_discount": quote.shipping_discount,
        "tax": quote.tax,
        "total": quote.total,
        // Chỉ thứ người mua thấy (chương trình, số tiền) — không dấu vân tay nội bộ của
        // `promotions`, vốn đổi theo cả dữ liệu không làm đổi tiền (vd phương thức thanh toán).
        "promotions": quote.evaluation.as_ref().map(|evaluation| {
            evaluation
                .applied
                .iter()
                .map(|applied| (applied.promotion_id, applied.amount))
                .collect::<Vec<_>>()
        }),
    }))
}

/// Lỗi của dòng cho người mua (đặt hàng).
pub(crate) fn line_error(line: &QuotedLine) -> Option<Error> {
    let sku = line.variant.as_ref().map_or_else(|| line.variant_id.to_string(), |v| v.sku.clone());
    match line.problem.as_ref()? {
        LineProblem::Unavailable => Some(errors::variant_unavailable(&sku)),
        LineProblem::OutOfStock { available } => Some(errors::out_of_stock(&sku, *available)),
        LineProblem::PreorderClosed => Some(errors::preorder_closed()),
    }
}

/// Giỏ cho `Promotions::eligible`, dựng từ đầu vào và báo giá đã tính (bộ chọn mã của website).
pub(crate) fn promotion_cart_of(input: &PriceInput, quote: &Quote) -> PromotionCart {
    let lines: Vec<QuotedLine> =
        quote.lines.iter().filter(|line| line.gift_of.is_none()).cloned().collect();
    promotion_cart(input, &lines, quote.shipping.as_ref(), quote.shipping_fee)
}
