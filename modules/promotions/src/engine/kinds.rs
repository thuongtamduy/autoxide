//! Cách tính của từng loại khuyến mại. Mọi danh sách dòng xếp theo `line_key` (không theo vị trí
//! trong giỏ) để giỏ xáo trộn thứ tự dòng vẫn ra cùng kết quả.

use autoxide_kernel::decimal::{Decimal, Rounding};
use autoxide_kernel::money::Money;

use super::{
    Applied, Input, LinePart, ReasonDraft, State, Unlock, VariantFacts, cart_conditions,
    in_windows, matches_text,
};
use crate::api::GiftChoice;
use crate::model::{
    ConditionKind, Kind, Program, RewardSelection, Scope, Status, TargetSet, ThresholdKind,
};

/// Kết quả tính một chương trình.
pub(super) enum Outcome {
    Applied(Applied),
    /// `gift_choice` đã đạt mà chưa chọn quà: gợi ý.
    Choice(GiftChoice),
}

/// Chương trình có là ứng viên cho giỏ không (mức giỏ, chưa xét dòng).
pub(super) fn admissible(program: &Program, input: &Input<'_>) -> Result<(), ReasonDraft> {
    let cart = input.cart;
    let simulated = input.include_inactive == Some(program.id);
    if !simulated && program.status != Status::Active {
        return Err(ReasonDraft::new("promotions.reason.not_running"));
    }
    if cart.at < program.starts_at || cart.at >= program.ends_at {
        return Err(ReasonDraft::new("promotions.reason.not_running"));
    }
    if !in_windows(program, input.local) {
        return Err(ReasonDraft::new("promotions.reason.outside_schedule"));
    }
    if program.currency != cart.currency.code() {
        return Err(ReasonDraft::new("promotions.reason.not_applicable"));
    }
    let own_store = input.ancestors.contains(&program.org_id);
    let sells_here = cart.lines.iter().any(|line| {
        input.variants.get(&line.variant_id).is_some_and(|facts| facts.org_id == program.org_id)
    });
    if !own_store && !sells_here {
        return Err(ReasonDraft::new("promotions.reason.not_applicable"));
    }
    if !program.customer_groups.is_empty()
        && !cart.customer_groups.iter().any(|group| program.customer_groups.contains(group))
    {
        return Err(ReasonDraft::new("promotions.reason.customer_group"));
    }
    if !program.channels.is_empty()
        && !cart.channel.as_ref().is_some_and(|channel| program.channels.contains(channel))
    {
        return Err(ReasonDraft::new("promotions.reason.channel"));
    }
    if program.first_order_only && !cart.first_order {
        return Err(ReasonDraft::new("promotions.reason.first_order_only"));
    }
    cart_conditions(program, cart)?;
    if program.remaining_uses() == Some(0) {
        return Err(ReasonDraft::new("promotions.reason.usage_exhausted"));
    }
    if program.remaining_budget() == Some(0) {
        return Err(ReasonDraft::new("promotions.reason.budget_exhausted"));
    }
    Ok(())
}

/// Tính chương trình trên phần còn lại của giỏ.
pub(super) fn compute(
    program: &Program,
    input: &Input<'_>,
    state: &State,
) -> Result<Outcome, ReasonDraft> {
    match (program.kind, program.scope) {
        (Kind::FreeShipping, _) | (_, Scope::Shipping) => shipping(program, input, state),
        (Kind::PercentOff | Kind::AmountOff, Scope::Order) => order_off(program, input, state),
        (Kind::Tiered, _) => tiered(program, input, state),
        (Kind::PercentOff | Kind::AmountOff | Kind::FixedPrice, _) => {
            items_off(program, input, state)
        }
        (Kind::FlashSale, _) => flash_sale(program, input, state),
        (Kind::BuyXGetY, _) => buy_x_get_y(program, input, state),
        (Kind::BundlePrice, _) => bundle(program, input, state),
        (Kind::AddOn, _) => add_on(program, input, state),
        (Kind::GiftChoice, _) => gift_choice(program, input, state),
    }
}

/// `amount × bp / 10 000`, làm tròn nửa lên về đơn vị nhỏ nhất (`rust_decimal`, D-188).
pub(crate) fn percent_of(amount: i64, bp: i32, input: &Input<'_>) -> i64 {
    Money::new(amount, input.cart.currency)
        .apply_rate(Decimal::new(i64::from(bp), 4), Rounding::HalfUp)
        .map_or(0, Money::minor)
}

/// Số tiền dạng người đọc: `120,000 VND`, `19.99 USD`.
pub(crate) fn show_money(amount: i64, input: &Input<'_>) -> String {
    let text = Money::new(amount, input.cart.currency).to_decimal().to_string();
    let (whole, fraction) =
        text.split_once('.').map_or((text.as_str(), None), |(a, b)| (a, Some(b)));
    let (sign, digits) = whole.strip_prefix('-').map_or(("", whole), |rest| ("-", rest));
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    match fraction {
        Some(fraction) => format!("{sign}{grouped}.{fraction} {}", input.cart.currency.code()),
        None => format!("{sign}{grouped} {}", input.cart.currency.code()),
    }
}

/// Phần trăm dạng người đọc: `1050` → `10.5`.
pub(crate) fn show_percent(bp: i32) -> String {
    Decimal::new(i64::from(bp), 2).normalize().to_string()
}

fn facts<'a>(input: &'a Input<'_>, line: usize) -> Option<&'a VariantFacts> {
    input.variants.get(&input.cart.lines[line].variant_id)
}

/// Dòng chương trình được áp, xếp theo `line_key`: đúng người bán, khớp mục tiêu, qua điều kiện
/// theo dòng, còn tiền.
pub(super) fn lines_for(
    program: &Program,
    input: &Input<'_>,
    state: &State,
    targets: &TargetSet,
) -> Vec<usize> {
    let own_store = input.ancestors.contains(&program.org_id);
    let mut lines: Vec<usize> = (0..input.cart.lines.len())
        .filter(|&index| {
            let line = &input.cart.lines[index];
            if state.remaining[index] <= 0 || line.quantity <= 0 {
                return false;
            }
            let facts = facts(input, index);
            if !own_store && facts.is_none_or(|facts| facts.org_id != program.org_id) {
                return false;
            }
            if !targets.is_empty()
                && !facts.is_some_and(|facts| {
                    targets.matches(line.variant_id, facts.product_id, &facts.category_ids)
                })
            {
                return false;
            }
            line_conditions(program, input, state, index)
        })
        .collect();
    lines.sort_by(|a, b| input.cart.lines[*a].line_key.cmp(&input.cart.lines[*b].line_key));
    lines
}

fn line_conditions(program: &Program, input: &Input<'_>, state: &State, index: usize) -> bool {
    let line = &input.cart.lines[index];
    let facts = facts(input, index);
    program.conditions.iter().all(|condition| match condition.kind {
        ConditionKind::SellerOrg => {
            let seller = facts.map(|facts| facts.org_id.to_string());
            matches_text(condition.operator, &condition.values, seller.as_deref())
        }
        // `catalog` chưa đưa thuộc tính / thương hiệu qua `api`: không có giá trị → `in` sai,
        // `not_in` đúng.
        ConditionKind::Brand | ConditionKind::Attribute => {
            matches_text(condition.operator, &condition.values, None)
        }
        ConditionKind::ExcludeDiscounted => {
            !line.has_compare_at && !state.product_discounted[index]
        }
        ConditionKind::ExcludeTargets => !condition.values.iter().any(|value| {
            let Some((kind, id)) = value.split_once(':') else { return false };
            let id = id.parse::<uuid::Uuid>().ok();
            match (kind, id, facts) {
                ("variant", Some(id), _) => line.variant_id == id,
                ("product", Some(id), Some(facts)) => facts.product_id == id,
                ("category", Some(id), Some(facts)) => facts.category_ids.contains(&id),
                _ => false,
            }
        }),
        _ => true,
    })
}

fn applied(program: &Program, reason: ReasonDraft) -> Applied {
    Applied {
        program: program.id,
        class: program.class(),
        unlock: None::<Unlock>,
        lines: Vec::new(),
        shipping: 0,
        gifts: Vec::new(),
        reason,
    }
}

fn part(line: usize, amount: i64) -> LinePart {
    LinePart { line, amount, units: 0, capped_by_law: false, capped_by_floor: false }
}

/// Ngưỡng chung (`min_quantity`, `min_order`) trên các dòng `lines`; `Err` nói còn thiếu bao nhiêu.
fn thresholds(
    program: &Program,
    input: &Input<'_>,
    state: &State,
    lines: &[usize],
) -> Result<(), ReasonDraft> {
    if lines.is_empty() {
        return Err(ReasonDraft::new("promotions.reason.not_applicable"));
    }
    let quantity: i64 =
        super::saturating_sum(lines.iter().map(|&line| input.cart.lines[line].quantity));
    if let Some(min) = program.min_quantity
        && quantity < i64::from(min)
    {
        return Err(ReasonDraft::new("promotions.reason.min_quantity")
            .with("count", i64::from(min) - quantity));
    }
    let subtotal: i64 = super::saturating_sum(lines.iter().map(|&line| state.remaining[line]));
    if let Some(min) = program.min_order_minor
        && subtotal < min
    {
        return Err(ReasonDraft::new("promotions.reason.add_more")
            .with("amount", show_money(min - subtotal, input))
            .with("min_order", show_money(min, input)));
    }
    Ok(())
}

/// Cắt tổng các phần về `max_discount_minor`, chia lại theo tỉ lệ (tổng khớp từng đồng).
fn cap_total(program: &Program, input: &Input<'_>, parts: &mut Vec<LinePart>) {
    let total: i64 = super::saturating_sum(parts.iter().map(|part| part.amount));
    if let Some(max) = program.max_discount_minor
        && total > max
    {
        spread(max, parts, input);
    }
    parts.retain(|part| part.amount > 0);
}

/// Chia `amount` cho các phần theo tỉ lệ số tiền hiện có của chúng (`Money::allocate`).
fn spread(amount: i64, parts: &mut [LinePart], input: &Input<'_>) {
    let ratios: Vec<u64> =
        parts.iter().map(|part| u64::try_from(part.amount).unwrap_or(0)).collect();
    if let Ok(shares) = Money::new(amount, input.cart.currency).allocate(&ratios) {
        for (part, share) in parts.iter_mut().zip(shares) {
            part.amount = share.minor();
        }
    }
}

/// Chia `amount` cho các dòng theo tiền còn lại của dòng.
fn allocate(amount: i64, lines: &[usize], state: &State, input: &Input<'_>) -> Vec<LinePart> {
    let mut parts: Vec<LinePart> =
        lines.iter().map(|&line| part(line, state.remaining[line])).collect();
    spread(amount, &mut parts, input);
    parts.retain(|part| part.amount > 0);
    parts
}

fn items_off(program: &Program, input: &Input<'_>, state: &State) -> Result<Outcome, ReasonDraft> {
    let lines = lines_for(program, input, state, &program.conditions_targets);
    thresholds(program, input, state, &lines)?;
    let mut parts = Vec::new();
    for &line in &lines {
        let quantity = input.cart.lines[line].quantity;
        let left = state.remaining[line];
        let amount = match program.kind {
            Kind::PercentOff => percent_of(left, program.value_bp.unwrap_or(0), input),
            Kind::AmountOff => program.value_minor.unwrap_or(0).saturating_mul(quantity),
            // Giá cố định mỗi đơn vị: giảm phần trên giá đó.
            _ => left - program.value_minor.unwrap_or(0).saturating_mul(quantity),
        };
        parts.push(part(line, amount.clamp(0, left)));
    }
    cap_total(program, input, &mut parts);
    let reason = match program.kind {
        Kind::PercentOff => ReasonDraft::new("promotions.reason.percent_off")
            .with("percent", show_percent(program.value_bp.unwrap_or(0))),
        Kind::AmountOff => ReasonDraft::new("promotions.reason.amount_off")
            .with("amount", show_money(program.value_minor.unwrap_or(0), input)),
        _ => ReasonDraft::new("promotions.reason.fixed_price")
            .with("price", show_money(program.value_minor.unwrap_or(0), input)),
    };
    let mut result = applied(program, reason);
    result.lines = parts;
    Ok(Outcome::Applied(result))
}

fn order_off(program: &Program, input: &Input<'_>, state: &State) -> Result<Outcome, ReasonDraft> {
    let lines = lines_for(program, input, state, &program.conditions_targets);
    thresholds(program, input, state, &lines)?;
    let subtotal: i64 = super::saturating_sum(lines.iter().map(|&line| state.remaining[line]));
    let mut amount = match program.kind {
        Kind::PercentOff => percent_of(subtotal, program.value_bp.unwrap_or(0), input),
        _ => program.value_minor.unwrap_or(0),
    }
    .clamp(0, subtotal);
    if let Some(max) = program.max_discount_minor {
        amount = amount.min(max);
    }
    let mut reason = match program.kind {
        Kind::PercentOff => ReasonDraft::new("promotions.reason.order_percent")
            .with("percent", show_percent(program.value_bp.unwrap_or(0))),
        _ => ReasonDraft::new("promotions.reason.order_amount")
            .with("amount", show_money(amount, input)),
    };
    reason = reason.with("min_order", show_money(program.min_order_minor.unwrap_or(0), input));
    let mut result = applied(program, reason);
    result.lines = allocate(amount, &lines, state, input);
    Ok(Outcome::Applied(result))
}

fn shipping(program: &Program, input: &Input<'_>, state: &State) -> Result<Outcome, ReasonDraft> {
    let lines = lines_for(program, input, state, &program.conditions_targets);
    thresholds(program, input, state, &lines)?;
    let fee = state.shipping_left;
    if fee <= 0 {
        return Err(ReasonDraft::new("promotions.reason.no_shipping_fee"));
    }
    let amount = match (program.kind, program.value_bp, program.value_minor) {
        (Kind::FreeShipping, _, Some(cap)) => fee.min(cap),
        (Kind::FreeShipping, _, None) => fee,
        (_, Some(bp), _) => percent_of(fee, bp, input),
        (_, None, Some(value)) => fee.min(value),
        _ => 0,
    };
    let mut result = applied(
        program,
        ReasonDraft::new("promotions.reason.free_shipping")
            .with("amount", show_money(amount, input)),
    );
    result.shipping = amount.clamp(0, fee);
    Ok(Outcome::Applied(result))
}

fn tiered(program: &Program, input: &Input<'_>, state: &State) -> Result<Outcome, ReasonDraft> {
    let lines = lines_for(program, input, state, &program.conditions_targets);
    if lines.is_empty() {
        return Err(ReasonDraft::new("promotions.reason.not_applicable"));
    }
    let quantity: i64 =
        super::saturating_sum(lines.iter().map(|&line| input.cart.lines[line].quantity));
    let subtotal: i64 = super::saturating_sum(lines.iter().map(|&line| state.remaining[line]));
    // Bậc cao nhất đạt được.
    let reached = program
        .tiers
        .iter()
        .filter(|tier| match tier.threshold_kind {
            ThresholdKind::Quantity => quantity >= tier.threshold,
            ThresholdKind::Amount => subtotal >= tier.threshold,
        })
        .max_by_key(|tier| tier.threshold);
    let Some(tier) = reached else {
        let lowest = program.tiers.iter().min_by_key(|tier| tier.threshold);
        return Err(match lowest {
            Some(tier) if tier.threshold_kind == ThresholdKind::Amount => {
                ReasonDraft::new("promotions.reason.add_more")
                    .with("amount", show_money(tier.threshold - subtotal, input))
                    .with("min_order", show_money(tier.threshold, input))
            }
            Some(tier) => ReasonDraft::new("promotions.reason.min_quantity")
                .with("count", tier.threshold - quantity),
            None => ReasonDraft::new("promotions.reason.not_applicable"),
        });
    };
    let mut amount = match (tier.value_bp, tier.value_minor) {
        (Some(bp), _) => percent_of(subtotal, bp, input),
        (None, Some(value)) => value,
        _ => 0,
    }
    .clamp(0, subtotal);
    if let Some(max) = program.max_discount_minor {
        amount = amount.min(max);
    }
    let mut result = applied(
        program,
        ReasonDraft::new("promotions.reason.tiered")
            .with(
                "threshold",
                match tier.threshold_kind {
                    ThresholdKind::Quantity => tier.threshold.to_string(),
                    ThresholdKind::Amount => show_money(tier.threshold, input),
                },
            )
            .with("amount", show_money(amount, input)),
    );
    result.lines = allocate(amount, &lines, state, input);
    Ok(Outcome::Applied(result))
}

fn flash_sale(program: &Program, input: &Input<'_>, state: &State) -> Result<Outcome, ReasonDraft> {
    let all = TargetSet::default();
    let mut parts = Vec::new();
    let mut sold_out = false;
    for line in lines_for(program, input, state, &all) {
        let cart_line = &input.cart.lines[line];
        let Some(item) = program.flash_item(cart_line.variant_id) else { continue };
        let mut units = cart_line.quantity;
        if let Some(left) = item.remaining() {
            if left == 0 {
                sold_out = true;
                continue;
            }
            units = units.min(left);
        }
        if let Some(per_customer) = item.per_customer_quantity_limit {
            units = units.min(per_customer);
        }
        let unit = state.remaining[line] / cart_line.quantity;
        let amount = match (item.sale_price_minor, item.value_bp) {
            (Some(price), _) => (unit - price).max(0).saturating_mul(units),
            (None, Some(bp)) => percent_of(unit.saturating_mul(units), bp, input),
            _ => 0,
        };
        let mut found = part(line, amount.clamp(0, state.remaining[line]));
        found.units = units;
        parts.push(found);
    }
    cap_total(program, input, &mut parts);
    if parts.is_empty() {
        return Err(ReasonDraft::new(if sold_out {
            "promotions.reason.flash_sold_out"
        } else {
            "promotions.reason.not_applicable"
        }));
    }
    let mut result = applied(program, ReasonDraft::new("promotions.reason.flash_sale"));
    result.lines = parts;
    Ok(Outcome::Applied(result))
}

/// Mỗi đơn vị của các dòng: (dòng, giá còn lại một đơn vị), đắt trước, bằng nhau theo `line_key`.
fn units(lines: &[usize], input: &Input<'_>, state: &State) -> Vec<(usize, i64)> {
    let mut units = Vec::new();
    for &line in lines {
        let quantity = input.cart.lines[line].quantity;
        let unit = state.remaining[line] / quantity.max(1);
        for _ in 0..quantity.min(10_000) {
            units.push((line, unit));
        }
    }
    units.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| input.cart.lines[a.0].line_key.cmp(&input.cart.lines[b.0].line_key))
    });
    units
}

/// Gom phần giảm theo dòng (xếp theo `line_key`).
fn by_line(amounts: Vec<(usize, i64)>, input: &Input<'_>, state: &State) -> Vec<LinePart> {
    let mut parts: Vec<LinePart> = Vec::new();
    for (line, amount) in amounts {
        match parts.iter_mut().find(|part| part.line == line) {
            Some(found) => found.amount += amount,
            None => parts.push(part(line, amount)),
        }
    }
    for found in &mut parts {
        found.amount = found.amount.clamp(0, state.remaining[found.line]);
    }
    parts.retain(|part| part.amount > 0);
    parts.sort_by(|a, b| input.cart.lines[a.line].line_key.cmp(&input.cart.lines[b.line].line_key));
    parts
}

fn buy_x_get_y(
    program: &Program,
    input: &Input<'_>,
    state: &State,
) -> Result<Outcome, ReasonDraft> {
    let buy = i64::from(program.buy_quantity.unwrap_or(1).max(1));
    let get = i64::from(program.get_quantity.unwrap_or(1).max(1));
    let bp = program.reward_value_bp.unwrap_or(10_000);
    let max_apps = program.max_applications.map_or(i64::MAX, |max| i64::from(max.max(0)));
    let condition = lines_for(program, input, state, &program.conditions_targets);
    let reason =
        ReasonDraft::new("promotions.reason.buy_x_get_y").with("buy", buy).with("get", get);
    let mut amounts = Vec::new();
    let mut gifts = Vec::new();
    match program.reward_selection.unwrap_or(RewardSelection::Cheapest) {
        RewardSelection::SameItem => {
            for &line in &condition {
                let quantity = input.cart.lines[line].quantity;
                let apps = (quantity / (buy + get)).min(max_apps);
                let unit = state.remaining[line] / quantity.max(1);
                amounts.push((line, percent_of(unit.saturating_mul(apps * get), bp, input)));
            }
        }
        RewardSelection::Cheapest => {
            let pool = units(&condition, input, state);
            let apps = (i64::try_from(pool.len()).unwrap_or(0) / (buy + get)).min(max_apps);
            let free = usize::try_from(apps * get).unwrap_or(0);
            // Món rẻ nhất được giảm.
            for &(line, unit) in pool.iter().rev().take(free) {
                amounts.push((line, percent_of(unit, bp, input)));
            }
        }
        RewardSelection::Specified => {
            let bought: i64 = super::saturating_sum(
                condition.iter().map(|&line| input.cart.lines[line].quantity),
            );
            let apps = (bought / buy).min(max_apps);
            let mut free = apps * get;
            let rewards = lines_for(program, input, state, &program.reward_targets);
            let pool = units(&rewards, input, state);
            for &(line, unit) in pool.iter().rev() {
                if free == 0 {
                    break;
                }
                amounts.push((line, percent_of(unit, bp, input)));
                free -= 1;
            }
            // Món thưởng chưa có trong giỏ, tặng 100%: thành hàng tặng giá 0.
            if free > 0
                && bp == 10_000
                && let Some(variant) = program.reward_targets.variants.iter().next()
            {
                gifts.push((*variant, free));
            }
        }
    }
    let lines = by_line(amounts, input, state);
    if lines.is_empty() && gifts.is_empty() {
        return Err(ReasonDraft::new("promotions.reason.min_quantity").with("count", buy + get));
    }
    let mut result = applied(program, reason);
    result.lines = lines;
    result.gifts = gifts;
    Ok(Outcome::Applied(result))
}

fn bundle(program: &Program, input: &Input<'_>, state: &State) -> Result<Outcome, ReasonDraft> {
    let size = usize::try_from(program.bundle_quantity.unwrap_or(0)).unwrap_or(0);
    let price = program.bundle_price_minor.unwrap_or(0);
    if size == 0 {
        return Err(ReasonDraft::new("promotions.reason.not_applicable"));
    }
    let lines = lines_for(program, input, state, &program.conditions_targets);
    let pool = units(&lines, input, state);
    let max_apps =
        program.max_applications.map_or(usize::MAX, |max| usize::try_from(max).unwrap_or(0));
    let apps = (pool.len() / size).min(max_apps);
    if apps == 0 {
        return Err(ReasonDraft::new("promotions.reason.min_quantity")
            .with("count", size.saturating_sub(pool.len())));
    }
    // Món đắt nhất vào combo trước (tất định); mỗi combo chia phần giảm theo giá từng món.
    let mut amounts = Vec::new();
    for group in pool.chunks(size).take(apps) {
        let total: i64 = super::saturating_sum(group.iter().map(|(_, unit)| *unit));
        let discount = (total - price).max(0);
        let mut parts: Vec<LinePart> = group.iter().map(|&(line, unit)| part(line, unit)).collect();
        spread(discount, &mut parts, input);
        amounts.extend(parts.into_iter().map(|part| (part.line, part.amount)));
    }
    let mut result = applied(
        program,
        ReasonDraft::new("promotions.reason.bundle_price")
            .with("quantity", size)
            .with("price", show_money(price, input)),
    );
    result.lines = by_line(amounts, input, state);
    if result.lines.is_empty() {
        return Err(ReasonDraft::new("promotions.reason.not_applicable"));
    }
    Ok(Outcome::Applied(result))
}

fn add_on(program: &Program, input: &Input<'_>, state: &State) -> Result<Outcome, ReasonDraft> {
    let condition = lines_for(program, input, state, &program.conditions_targets);
    let rewarded: Vec<usize> = lines_for(program, input, state, &TargetSet::default())
        .into_iter()
        .filter(|&line| program.flash_item(input.cart.lines[line].variant_id).is_some())
        .collect();
    // Món điều kiện phải là món khác món thưởng.
    let condition: Vec<usize> =
        condition.into_iter().filter(|line| !rewarded.contains(line)).collect();
    thresholds(program, input, state, &condition)?;
    let mut left = program.max_add_on_quantity.map_or(i64::MAX, |max| i64::from(max.max(0)));
    let mut parts = Vec::new();
    for line in rewarded {
        let cart_line = &input.cart.lines[line];
        let Some(item) = program.flash_item(cart_line.variant_id) else { continue };
        let units = cart_line.quantity.min(left).min(item.remaining().unwrap_or(i64::MAX));
        if units <= 0 {
            continue;
        }
        left -= units;
        let unit = state.remaining[line] / cart_line.quantity;
        let amount = match (item.sale_price_minor, item.value_bp) {
            (Some(price), _) => (unit - price).max(0).saturating_mul(units),
            (None, Some(bp)) => percent_of(unit.saturating_mul(units), bp, input),
            _ => 0,
        };
        let mut found = part(line, amount.clamp(0, state.remaining[line]));
        found.units = units;
        parts.push(found);
    }
    parts.retain(|part| part.amount > 0);
    if parts.is_empty() {
        return Err(ReasonDraft::new("promotions.reason.add_on_missing"));
    }
    let mut result = applied(program, ReasonDraft::new("promotions.reason.add_on"));
    result.lines = parts;
    Ok(Outcome::Applied(result))
}

fn gift_choice(
    program: &Program,
    input: &Input<'_>,
    state: &State,
) -> Result<Outcome, ReasonDraft> {
    let condition = lines_for(program, input, state, &program.conditions_targets);
    thresholds(program, input, state, &condition)?;
    let allowed = i64::from(program.gift_quantity.unwrap_or(1).max(1));
    let available: Vec<uuid::Uuid> = program
        .gift_options
        .iter()
        .filter(|option| option.remaining() != Some(0))
        .map(|option| option.variant_id)
        .collect();
    let mut gifts: Vec<(uuid::Uuid, i64)> = Vec::new();
    let mut taken = 0;
    let mut selections: Vec<_> = input
        .cart
        .gift_selections
        .iter()
        .filter(|found| found.promotion_id == program.id)
        .collect();
    selections.sort_by_key(|found| found.variant_id);
    for selection in selections {
        let Some(option) =
            program.gift_options.iter().find(|option| option.variant_id == selection.variant_id)
        else {
            continue;
        };
        let quantity =
            selection.quantity.min(allowed - taken).min(option.remaining().unwrap_or(i64::MAX));
        if quantity > 0 {
            gifts.push((selection.variant_id, quantity));
            taken += quantity;
        }
    }
    if gifts.is_empty() {
        if available.is_empty() {
            return Err(ReasonDraft::new("promotions.reason.gift_out_of_stock"));
        }
        return Ok(Outcome::Choice(GiftChoice {
            promotion_id: program.id,
            options: available,
            quantity: allowed,
        }));
    }
    let mut result = applied(program, ReasonDraft::new("promotions.reason.gift"));
    result.gifts = gifts;
    Ok(Outcome::Applied(result))
}

#[cfg(test)]
mod tests {
    use super::show_percent;

    #[test]
    fn phan_tram_doc_duoc() {
        assert_eq!(show_percent(1050), "10.5");
        assert_eq!(show_percent(5000), "50");
    }

    #[test]
    fn toan_tu_khong_dung_cho_chu() {
        assert!(!super::matches_text(crate::model::Operator::Gte, &[], Some("x")));
    }
}
