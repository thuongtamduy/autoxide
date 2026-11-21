//! Bộ tính khuyến mại: hàm thuần, không database, không mạng (D-222). Service chuẩn bị đầu vào
//! (biến thể từ `Catalog`, chương trình từ `CompiledPromotions`, mã / voucher đã kiểm), gọi
//! [`compute`], hỏi giá tham chiếu và sàn giá cho các dòng được giảm, rồi gọi [`caps::apply`].
//!
//! Thứ tự cố định để cùng giỏ luôn ra cùng kết quả (đặc tả 43, "Cách tính"):
//!
//! 1. Ứng viên: đang chạy, trong khung giờ, đúng nút, nhóm khách / kênh / điều kiện khớp;
//!    chương trình `code` / `voucher` chỉ khi đã mở bằng mã / voucher hợp lệ.
//! 2. Lớp `product` → `order` (trên phần còn lại sau lớp `product`) → `shipping`; trong mỗi lớp
//!    chọn theo luật cộng dồn (`priority` hay `best_for_buyer`, `exclusivity`, `combines_with`,
//!    `stop_after`).
//! 3. Trần pháp luật và sàn giá cắt khoản giảm theo thứ tự ngược lại (khoản áp sau bị cắt trước).

pub(crate) mod caps;
mod kinds;
pub(crate) use kinds::show_percent;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap};

use autoxide_kernel::money::Money;
use uuid::Uuid;

use crate::api::{Cart, GiftChoice, PromotionClass};
use crate::model::{ConditionKind, Exclusivity, Operator, Program, Trigger};

/// Thông tin của một biến thể bộ tính cần (từ `Catalog::variants`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VariantFacts {
    pub product_id: Uuid,
    /// Nút sở hữu sản phẩm (người bán).
    pub org_id: Uuid,
    /// Danh mục kèm tổ tiên.
    pub category_ids: Vec<Uuid>,
}

/// Chương trình đã được mở bằng mã hay voucher hợp lệ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Unlock {
    pub code: Option<String>,
    pub code_id: Option<Uuid>,
    pub voucher_id: Option<Uuid>,
}

/// Lý do chưa dịch: `code` là i18n key, `params` điền vào câu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReasonDraft {
    pub code: &'static str,
    pub params: BTreeMap<&'static str, String>,
}

impl ReasonDraft {
    pub(crate) fn new(code: &'static str) -> Self {
        ReasonDraft { code, params: BTreeMap::new() }
    }

    pub(crate) fn with(mut self, name: &'static str, value: impl ToString) -> Self {
        self.params.insert(name, value.to_string());
        self
    }
}

/// Đầu vào của một lần tính.
pub(crate) struct Input<'a> {
    pub cart: &'a Cart,
    pub variants: &'a HashMap<Uuid, VariantFacts>,
    /// Chuỗi nút của cửa hàng, gồm chính nó.
    pub ancestors: &'a [Uuid],
    /// `cart.at` theo giờ địa phương của cửa hàng (khung giờ).
    pub local: jiff::civil::DateTime,
    pub best_for_buyer: bool,
    pub max_vouchers_per_class: usize,
    /// promotion_id → mã / voucher đã kiểm hợp lệ.
    pub unlocked: &'a HashMap<Uuid, Unlock>,
    /// Chạy thử: chương trình này được tính cả khi chưa bật (D-222).
    pub include_inactive: Option<Uuid>,
}

/// Một phần giảm trên một dòng.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinePart {
    pub line: usize,
    pub amount: i64,
    /// Số đơn vị dùng suất giá sốc (flash sale, mua kèm); 0 với loại khác.
    pub units: i64,
    pub capped_by_law: bool,
    pub capped_by_floor: bool,
}

/// Một chương trình đã áp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Applied {
    pub program: Uuid,
    pub class: PromotionClass,
    pub unlock: Option<Unlock>,
    pub lines: Vec<LinePart>,
    pub shipping: i64,
    /// (biến thể, số lượng) hàng tặng.
    pub gifts: Vec<(Uuid, i64)>,
    pub reason: ReasonDraft,
}

impl Applied {
    pub(crate) fn total(&self) -> i64 {
        self.lines.iter().map(|part| part.amount).sum::<i64>() + self.shipping
    }

    fn is_empty(&self) -> bool {
        self.total() == 0 && self.gifts.is_empty()
    }
}

/// Mã / voucher / chương trình không áp được, kèm lý do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rejection {
    pub program: Option<Uuid>,
    pub code: Option<String>,
    pub voucher_id: Option<Uuid>,
    pub reason: ReasonDraft,
}

/// Kết quả trước khi cắt trần.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Draft {
    pub applied: Vec<Applied>,
    pub choices: Vec<GiftChoice>,
    pub rejected: Vec<Rejection>,
}

/// Trạng thái trong lúc tính: phần tiền còn lại của từng dòng, dòng đã có giảm lớp `product`.
#[derive(Debug, Clone)]
pub(crate) struct State {
    pub remaining: Vec<i64>,
    pub product_discounted: Vec<bool>,
    pub shipping_left: i64,
}

impl State {
    fn new(cart: &Cart) -> Self {
        State {
            remaining: cart
                .lines
                .iter()
                .map(|line| line.unit_price.minor().saturating_mul(line.quantity))
                .collect(),
            product_discounted: vec![false; cart.lines.len()],
            shipping_left: cart.shipping_fee.map_or(0, Money::minor),
        }
    }

    fn take(&mut self, applied: &Applied) {
        for part in &applied.lines {
            self.remaining[part.line] -= part.amount;
            if applied.class == PromotionClass::Product {
                self.product_discounted[part.line] = true;
            }
        }
        self.shipping_left -= applied.shipping;
    }
}

/// Tính mọi chương trình ứng viên cho giỏ.
pub(crate) fn compute(programs: &[&Program], input: &Input<'_>) -> Draft {
    let mut draft = Draft::default();
    // Thứ tự ổn định: ưu tiên giảm dần rồi `id` (UUIDv7 — tạo trước đứng trước).
    let mut ordered: Vec<&Program> = programs.to_vec();
    ordered.sort_by(|a, b| b.priority.cmp(&a.priority).then(a.id.cmp(&b.id)));
    ordered.dedup_by_key(|program| program.id);

    let mut candidates: Vec<&Program> = Vec::new();
    for program in ordered {
        let unlock = input.unlocked.get(&program.id);
        if program.trigger != Trigger::Automatic && unlock.is_none() {
            continue;
        }
        match kinds::admissible(program, input) {
            Ok(()) => candidates.push(program),
            Err(reason) => {
                if let Some(unlock) = unlock {
                    draft.rejected.push(rejection(program.id, unlock, reason));
                }
            }
        }
    }
    let candidates = limit_vouchers(candidates, input, &mut draft);

    let mut state = State::new(input.cart);
    let mut applied_all: Vec<(&Program, Applied)> = Vec::new();
    for class in [PromotionClass::Product, PromotionClass::Order, PromotionClass::Shipping] {
        let in_class: Vec<&Program> =
            candidates.iter().copied().filter(|program| program.class() == class).collect();
        let earlier: Vec<&Program> = applied_all.iter().map(|(program, _)| *program).collect();
        let chosen = if input.best_for_buyer {
            best_for_buyer(&in_class, &earlier, input, &state, &mut draft)
        } else {
            by_priority(&in_class, &earlier, input, &state, &mut draft)
        };
        for (program, applied) in chosen {
            state.take(&applied);
            applied_all.push((program, applied));
        }
    }
    // Chương trình mở bằng mã / voucher mà không được chọn (không cộng được, không giảm gì).
    for program in &candidates {
        if let Some(unlock) = input.unlocked.get(&program.id)
            && !applied_all.iter().any(|(applied, _)| applied.id == program.id)
            && !draft.rejected.iter().any(|found| found.program == Some(program.id))
            && !draft.choices.iter().any(|choice| choice.promotion_id == program.id)
        {
            draft.rejected.push(rejection(
                program.id,
                unlock,
                ReasonDraft::new("promotions.reason.not_combinable"),
            ));
        }
    }
    draft.applied = applied_all.into_iter().map(|(_, applied)| applied).collect();
    draft
}

fn rejection(program: Uuid, unlock: &Unlock, reason: ReasonDraft) -> Rejection {
    Rejection {
        program: Some(program),
        code: unlock.code.clone(),
        voucher_id: unlock.voucher_id,
        reason,
    }
}

/// Mỗi lớp tối đa `max_vouchers_per_class` voucher cho mỗi bên trả tiền (nút sở hữu chương
/// trình) — kiểu Shopee: một voucher của mỗi người bán + một voucher sàn cho mỗi lớp (D-222).
fn limit_vouchers<'p>(
    candidates: Vec<&'p Program>,
    input: &Input<'_>,
    draft: &mut Draft,
) -> Vec<&'p Program> {
    let mut used: HashMap<(PromotionClass, Uuid), usize> = HashMap::new();
    let mut kept = Vec::with_capacity(candidates.len());
    for program in candidates {
        if program.trigger == Trigger::Voucher {
            let count = used.entry((program.class(), program.org_id)).or_default();
            if *count >= input.max_vouchers_per_class {
                if let Some(unlock) = input.unlocked.get(&program.id) {
                    draft.rejected.push(rejection(
                        program.id,
                        unlock,
                        ReasonDraft::new("promotions.reason.too_many_vouchers")
                            .with("max", input.max_vouchers_per_class),
                    ));
                }
                continue;
            }
            *count += 1;
        }
        kept.push(program);
    }
    kept
}

/// Hai chương trình cộng được với nhau khi CẢ HAI cho phép (D-222).
pub(crate) fn compatible(a: &Program, b: &Program) -> bool {
    if a.exclusivity == Exclusivity::Global || b.exclusivity == Exclusivity::Global {
        return false;
    }
    if a.class() == b.class() {
        return a.exclusivity == Exclusivity::None && b.exclusivity == Exclusivity::None;
    }
    a.combines_with.contains(&b.class()) && b.combines_with.contains(&a.class())
}

/// Tính một chương trình trên trạng thái hiện tại; ghi lý do khi mã / voucher không áp được.
fn attempt(
    program: &Program,
    input: &Input<'_>,
    state: &State,
    draft: &mut Draft,
    record: bool,
) -> Option<Applied> {
    let unlock = input.unlocked.get(&program.id).cloned();
    match kinds::compute(program, input, state) {
        Ok(kinds::Outcome::Applied(mut applied)) => {
            if applied.is_empty() {
                return None;
            }
            // Không đủ ngân sách thì không áp (không áp một phần, D-222).
            if program.remaining_budget().is_some_and(|left| applied.total() > left) {
                if record && let Some(unlock) = &unlock {
                    draft.rejected.push(rejection(
                        program.id,
                        unlock,
                        ReasonDraft::new("promotions.reason.budget_exhausted"),
                    ));
                }
                return None;
            }
            applied.unlock = unlock;
            Some(applied)
        }
        Ok(kinds::Outcome::Choice(choice)) => {
            if record
                && !draft.choices.iter().any(|found| found.promotion_id == choice.promotion_id)
            {
                draft.choices.push(choice);
            }
            None
        }
        Err(reason) => {
            if record && let Some(unlock) = &unlock {
                draft.rejected.push(rejection(program.id, unlock, reason));
            }
            None
        }
    }
}

/// Chế độ `priority`: theo ưu tiên, áp mọi chương trình cộng được với những gì đã áp.
fn by_priority<'p>(
    in_class: &[&'p Program],
    earlier: &[&'p Program],
    input: &Input<'_>,
    state: &State,
    draft: &mut Draft,
) -> Vec<(&'p Program, Applied)> {
    let mut state = state.clone();
    let mut chosen: Vec<(&'p Program, Applied)> = Vec::new();
    for program in in_class {
        let fits = earlier.iter().all(|other| compatible(program, other))
            && chosen.iter().all(|(other, _)| compatible(program, other));
        if !fits {
            continue;
        }
        if let Some(applied) = attempt(program, input, &state, draft, true) {
            state.take(&applied);
            let stop = program.stop_after;
            chosen.push((program, applied));
            if stop {
                break;
            }
        }
    }
    chosen
}

/// Chế độ `best_for_buyer`: so tập cộng dồn được (chương trình `exclusivity = none`) với từng
/// chương trình độc quyền đứng riêng; lấy khoản giảm lớn nhất, bằng nhau thì ưu tiên rồi `id`.
/// Số phương án ≤ số chương trình độc quyền + 1: tuyến tính, tất định.
fn best_for_buyer<'p>(
    in_class: &[&'p Program],
    earlier: &[&'p Program],
    input: &Input<'_>,
    state: &State,
    draft: &mut Draft,
) -> Vec<(&'p Program, Applied)> {
    let stackable: Vec<&'p Program> = in_class
        .iter()
        .copied()
        .filter(|program| program.exclusivity == Exclusivity::None)
        .collect();
    let mut options: Vec<Vec<(&'p Program, Applied)>> =
        vec![by_priority(&stackable, earlier, input, state, draft)];
    for program in in_class.iter().filter(|program| program.exclusivity != Exclusivity::None) {
        if !earlier.iter().all(|other| compatible(program, other)) {
            continue;
        }
        if let Some(applied) = attempt(program, input, state, draft, true) {
            options.push(vec![(program, applied)]);
        }
    }
    let score = |option: &Vec<(&Program, Applied)>| -> i64 {
        option.iter().map(|(_, applied)| applied.total()).sum()
    };
    let mut best: Option<Vec<(&'p Program, Applied)>> = None;
    for option in options {
        if option.is_empty() {
            continue;
        }
        let better = match &best {
            None => true,
            Some(current) => {
                let (new_total, old_total) = (score(&option), score(current));
                let (new_lead, old_lead) = (option[0].0, current[0].0);
                new_total > old_total
                    || (new_total == old_total
                        && (new_lead.priority > old_lead.priority
                            || (new_lead.priority == old_lead.priority
                                && new_lead.id < old_lead.id)))
            }
        };
        if better {
            best = Some(option);
        }
    }
    best.unwrap_or_default()
}

/// Khung giờ: không có dòng nào = cả ngày.
pub(crate) fn in_windows(program: &Program, local: jiff::civil::DateTime) -> bool {
    if program.windows.is_empty() {
        return true;
    }
    let day_bit = 1 << local.weekday().to_monday_zero_offset();
    let minute = i32::from(local.hour()) * 60 + i32::from(local.minute());
    program.windows.iter().any(|window| {
        window.weekdays & day_bit != 0
            && minute >= window.start_minute
            && minute < window.end_minute
    })
}

/// So một giá trị của giỏ với điều kiện `in` / `not_in` (giá trị không có: `in` sai, `not_in` đúng).
pub(crate) fn matches_text(operator: Operator, values: &[String], value: Option<&str>) -> bool {
    let found =
        value.is_some_and(|value| values.iter().any(|item| item.eq_ignore_ascii_case(value)));
    match operator {
        Operator::In => found,
        Operator::NotIn => !found,
        Operator::Gte | Operator::Lte => false,
    }
}

/// Điều kiện ở mức giỏ (không theo dòng); `Err` mang lý do người mua đọc được.
pub(crate) fn cart_conditions(program: &Program, cart: &Cart) -> Result<(), ReasonDraft> {
    for condition in &program.conditions {
        let field = match condition.kind {
            ConditionKind::PaymentMethod => cart.payment_method.as_deref(),
            ConditionKind::Carrier => cart.carrier.as_deref(),
            ConditionKind::ShipToProvince => cart.ship_to_province_code.as_deref(),
            ConditionKind::MembershipTier => cart.membership_tier.as_deref(),
            ConditionKind::Client => cart.client.as_deref(),
            ConditionKind::NewCustomer => {
                if !cart.first_order {
                    return Err(ReasonDraft::new("promotions.reason.new_customer_only"));
                }
                continue;
            }
            ConditionKind::ReturningCustomer => {
                if cart.first_order {
                    return Err(ReasonDraft::new("promotions.reason.returning_customer_only"));
                }
                continue;
            }
            ConditionKind::MinItems => {
                let items: i64 = cart.lines.iter().map(|line| line.quantity).sum();
                let wanted = condition.values.first().and_then(|value| value.parse::<i64>().ok());
                let ok = match (condition.operator, wanted) {
                    (Operator::Gte, Some(wanted)) => items >= wanted,
                    (Operator::Lte, Some(wanted)) => items <= wanted,
                    _ => false,
                };
                if !ok {
                    return Err(ReasonDraft::new("promotions.reason.min_items")
                        .with("count", wanted.unwrap_or_default()));
                }
                continue;
            }
            // Điều kiện theo dòng: xét khi chọn dòng.
            ConditionKind::Brand
            | ConditionKind::SellerOrg
            | ConditionKind::Attribute
            | ConditionKind::ExcludeDiscounted
            | ConditionKind::ExcludeTargets => continue,
        };
        if !matches_text(condition.operator, &condition.values, field) {
            return Err(ReasonDraft::new("promotions.reason.condition_unmet")
                .with("condition", condition.kind.as_str()));
        }
    }
    Ok(())
}
