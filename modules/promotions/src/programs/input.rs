//! Đầu vào của một chương trình (thân `POST /promotions`, tool `create_promotion`) và bộ kiểm.
//! Service gom mọi lỗi để báo MỘT lần (D-083); chỉ service kiểm, không `#[validate]` trên trường.

use std::collections::BTreeSet;

use autoxide_kernel::error::{Error, FieldError};
use autoxide_kernel::money::{Currency, Money};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::model::{
    ClassName, ConditionKind, Exclusivity, GiftPolicy, Kind, Operator, Payer, Period, ProgramKind,
    RewardSelection, Scope, ThresholdKind, Trigger,
};

/// An extra condition of a promotion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = PromotionsConditionInput)]
pub(crate) struct ConditionInput {
    pub kind: ConditionKind,
    /// `in` / `not_in` compare a text of the cart with `values`; `gte` / `lte` compare a number
    /// (`min_items`).
    pub operator: Operator,
    /// Values to compare with: payment methods, carriers, province codes, tiers, clients (`web`,
    /// `app`, `pos`), seller unit ids, or `variant:<id>` / `product:<id>` / `category:<id>` for
    /// `exclude_targets`. Up to 200.
    #[serde(default)]
    pub values: Vec<String>,
}

/// A time window in the store's time zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = PromotionsWindowInput)]
pub(crate) struct WindowInput {
    /// Days of the week, 1 (Monday) to 7 (Sunday).
    pub weekdays: Vec<u8>,
    /// Start, in minutes after midnight (0 to 1439).
    pub start_minute: i32,
    /// End (not included), in minutes after midnight (1 to 1440), after `start_minute`.
    pub end_minute: i32,
}

/// One tier of a `tiered` promotion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = PromotionsTierInput)]
pub(crate) struct TierInput {
    pub threshold_kind: ThresholdKind,
    /// Items (`quantity`) or minor units of money (`amount`) to reach.
    pub threshold: i64,
    /// Percent off in basis points (1% = 100); or give `value`.
    pub value_bp: Option<i32>,
    /// Money off the targeted lines.
    pub value: Option<Money>,
}

/// A gift a buyer may choose (`gift_choice`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = PromotionsGiftOptionInput)]
pub(crate) struct GiftOptionInput {
    pub variant_id: Uuid,
    /// Gifts of this variant that can be given; none = no limit.
    pub stock_limit: Option<i64>,
}

/// A share of who pays the discount.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
#[schema(as = PromotionsFundingInput)]
pub(crate) struct FundingInput {
    pub payer_kind: Payer,
    /// Unit that pays (the seller, brand or partner unit), when known.
    pub payer_org_id: Option<Uuid>,
    /// Share in basis points; all shares add up to 10000.
    pub share_bp: i32,
}

/// A promotion: the rule, its window, limits and texts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, validator::Validate)]
#[serde(deny_unknown_fields)]
#[schema(as = PromotionsPromotionInput)]
pub(crate) struct PromotionInput {
    /// Unit that runs the promotion (the store, chain or seller). Defaults to your unit when you
    /// work in only one.
    pub org_id: Option<Uuid>,
    pub kind: Kind,
    /// `items` (product class), `order` or `shipping`. Defaults to `shipping` for
    /// `free_shipping`, `items` otherwise.
    pub scope: Option<Scope>,
    /// `automatic`, `code` (coupons) or `voucher` (wallet vouchers). Defaults to `automatic`.
    pub trigger: Option<Trigger>,
    /// Currency of every amount and of the carts it applies to, e.g. `VND`.
    pub currency: String,
    /// Percent off in basis points (1% = 100), for `percent_off`, `flash_sale` defaults…
    pub value_bp: Option<i32>,
    /// Money off (per unit for `items`, per order for `order`), the fixed price, or the most
    /// shipping fee covered.
    pub value: Option<Money>,
    /// Order (or targeted lines) must reach this amount.
    pub min_order: Option<Money>,
    pub min_quantity: Option<i32>,
    /// Most discount per cart.
    pub max_discount: Option<Money>,
    pub buy_quantity: Option<i32>,
    pub get_quantity: Option<i32>,
    pub reward_selection: Option<RewardSelection>,
    /// Discount on the reward items in basis points; 10000 = free.
    pub reward_value_bp: Option<i32>,
    pub max_applications: Option<i32>,
    pub bundle_quantity: Option<i32>,
    pub bundle_price: Option<Money>,
    pub max_add_on_quantity: Option<i32>,
    pub gift_quantity: Option<i32>,
    #[serde(default)]
    pub first_order_only: bool,
    pub starts_at: DateTime<Utc>,
    /// Required: every promotion announces its end (Commercial Law, art. 97).
    pub ends_at: DateTime<Utc>,
    /// Higher first. Defaults to 0.
    #[serde(default)]
    pub priority: i32,
    pub exclusivity: Option<Exclusivity>,
    /// Classes this promotion may stack with; defaults to all three.
    pub combines_with: Option<Vec<ClassName>>,
    #[serde(default)]
    pub stop_after: bool,
    pub usage_limit: Option<i64>,
    pub per_customer_limit: Option<i64>,
    pub per_customer_period: Option<Period>,
    pub daily_usage_limit: Option<i64>,
    pub budget: Option<Money>,
    /// Give the usage back when a confirmed order is cancelled. Defaults to `true`.
    pub return_usage_on_cancel: Option<bool>,
    pub gift_return_policy: Option<GiftPolicy>,
    /// `national_focus` only for a national promotion programme announced by the authorities.
    pub program_kind: Option<ProgramKind>,
    pub voucher_quota: Option<i64>,
    pub per_user_claim_limit: Option<i32>,
    /// Days a voucher stays valid after it is issued; none = until `ends_at`.
    pub voucher_valid_days: Option<i32>,
    /// Buyers can save the voucher themselves ("Save" button).
    #[serde(default)]
    pub claimable: bool,
    #[serde(default)]
    pub claim_requires_verified_phone: bool,
    /// Shown on the public promotion list.
    #[serde(default)]
    pub public: bool,
    /// Language of `title` and `terms`, e.g. `vi`.
    pub locale: String,
    /// 1 to 200 characters.
    pub title: String,
    /// Conditions the buyer reads (Consumer Protection Law 19/2023), up to 5000 characters.
    #[serde(default)]
    pub terms: String,
    #[serde(default)]
    pub customer_groups: Vec<Uuid>,
    #[serde(default)]
    pub channels: Vec<String>,
    #[serde(default)]
    pub conditions: Vec<ConditionInput>,
    #[serde(default)]
    pub schedules: Vec<WindowInput>,
    #[serde(default)]
    pub tiers: Vec<TierInput>,
    #[serde(default)]
    pub gift_options: Vec<GiftOptionInput>,
    /// Who pays; defaults to the unit running the promotion, 100%.
    #[serde(default)]
    pub funding: Vec<FundingInput>,
}

impl PromotionInput {
    pub(crate) fn scope(&self) -> Scope {
        self.scope.unwrap_or(if self.kind == Kind::FreeShipping {
            Scope::Shipping
        } else {
            Scope::Items
        })
    }

    pub(crate) fn trigger(&self) -> Trigger {
        self.trigger.unwrap_or(Trigger::Automatic)
    }

    pub(crate) fn combines_with(&self) -> String {
        let classes: BTreeSet<ClassName> = self.combines_with.clone().map_or_else(
            || [ClassName::Product, ClassName::Order, ClassName::Shipping].into_iter().collect(),
            |list| list.into_iter().collect(),
        );
        classes.iter().map(|class| class.as_str()).collect::<Vec<_>>().join(",")
    }
}

/// Bitmask ngày trong tuần (bit 0 = thứ Hai).
pub(crate) fn weekday_mask(days: &[u8]) -> i32 {
    days.iter().filter(|day| (1..=7).contains(*day)).fold(0, |mask, day| mask | (1 << (day - 1)))
}

/// Kiểm đầu vào; trả tiền tệ đã đọc. Gom mọi lỗi.
pub(crate) fn check(input: &PromotionInput) -> Result<Currency, Error> {
    let mut problems = Vec::new();
    let currency = Currency::parse(&input.currency).ok();
    if currency.is_none() {
        problems.push(FieldError::new("currency", "invalid", "must be an ISO 4217 code like VND"));
    }
    let money = |field: &str, value: &Option<Money>, problems: &mut Vec<FieldError>| {
        if let Some(value) = value {
            if Some(value.currency()) != currency {
                problems.push(FieldError::new(
                    field,
                    "currency",
                    "must be in the promotion's currency",
                ));
            }
            if !value.is_positive() {
                problems.push(FieldError::new(field, "range", "must be more than 0"));
            }
        }
    };
    money("value", &input.value, &mut problems);
    money("min_order", &input.min_order, &mut problems);
    money("max_discount", &input.max_discount, &mut problems);
    money("budget", &input.budget, &mut problems);
    money("bundle_price", &input.bundle_price, &mut problems);
    let bp = |field: &str, value: Option<i32>, problems: &mut Vec<FieldError>| {
        if value.is_some_and(|value| !(1..=10_000).contains(&value)) {
            problems.push(FieldError::new(
                field,
                "range",
                "must be 1 to 10000 basis points (0.01% to 100%)",
            ));
        }
    };
    bp("value_bp", input.value_bp, &mut problems);
    bp("reward_value_bp", input.reward_value_bp, &mut problems);
    let positive = |field: &str, value: Option<i64>, problems: &mut Vec<FieldError>| {
        if value.is_some_and(|value| value < 1) {
            problems.push(FieldError::new(field, "range", "must be at least 1"));
        }
    };
    for (field, value) in [
        ("min_quantity", input.min_quantity),
        ("buy_quantity", input.buy_quantity),
        ("get_quantity", input.get_quantity),
        ("max_applications", input.max_applications),
        ("bundle_quantity", input.bundle_quantity),
        ("max_add_on_quantity", input.max_add_on_quantity),
        ("gift_quantity", input.gift_quantity),
        ("per_user_claim_limit", input.per_user_claim_limit),
        ("voucher_valid_days", input.voucher_valid_days),
    ] {
        positive(field, value.map(i64::from), &mut problems);
    }
    for (field, value) in [
        ("usage_limit", input.usage_limit),
        ("per_customer_limit", input.per_customer_limit),
        ("daily_usage_limit", input.daily_usage_limit),
        ("voucher_quota", input.voucher_quota),
    ] {
        positive(field, value, &mut problems);
    }
    if input.ends_at <= input.starts_at {
        problems.push(FieldError::new("ends_at", "invalid_window", "must be after `starts_at`"));
    }
    if input.title.trim().is_empty() || input.title.chars().count() > 200 {
        problems.push(FieldError::new("title", "length", "must be 1 to 200 characters long"));
    }
    if input.terms.chars().count() > 5000 {
        problems.push(FieldError::new("terms", "length", "must be at most 5000 characters long"));
    }
    if autoxide_kernel::i18n::canonical_locale(&input.locale).is_none() {
        problems.push(FieldError::new(
            "locale",
            "invalid",
            "must be a language code like `vi` or `en`",
        ));
    }
    check_kind(input, &mut problems);
    check_lists(input, &mut problems);
    if problems.is_empty() {
        currency.ok_or_else(|| Error::validation(Vec::new()))
    } else {
        Err(Error::validation(problems))
    }
}

/// Trường bắt buộc theo loại khuyến mại.
fn check_kind(input: &PromotionInput, problems: &mut Vec<FieldError>) {
    let scope = input.scope();
    let needs_value = |problems: &mut Vec<FieldError>| {
        if input.value_bp.is_none() && input.value.is_none() {
            problems.push(FieldError::new("value_bp", "required", "give `value_bp` or `value`"));
        }
    };
    match input.kind {
        crate::model::Kind::PercentOff if input.value_bp.is_none() => {
            problems.push(FieldError::new(
                "value_bp",
                "required",
                "a percent discount needs `value_bp`",
            ));
        }
        crate::model::Kind::AmountOff | crate::model::Kind::FixedPrice if input.value.is_none() => {
            problems.push(FieldError::new("value", "required", "this kind needs `value`"));
        }
        crate::model::Kind::FixedPrice if scope != Scope::Items => {
            problems.push(FieldError::new("scope", "invalid", "a fixed price applies to `items`"));
        }
        crate::model::Kind::BuyXGetY
            if input.buy_quantity.is_none() || input.get_quantity.is_none() =>
        {
            problems.push(FieldError::new(
                "buy_quantity",
                "required",
                "give `buy_quantity` and `get_quantity`",
            ));
        }
        crate::model::Kind::Tiered if input.tiers.is_empty() => {
            problems.push(FieldError::new(
                "tiers",
                "required",
                "a tiered promotion needs at least one tier",
            ));
        }
        crate::model::Kind::BundlePrice
            if input.bundle_quantity.is_none() || input.bundle_price.is_none() =>
        {
            problems.push(FieldError::new(
                "bundle_quantity",
                "required",
                "give `bundle_quantity` and `bundle_price`",
            ));
        }
        crate::model::Kind::GiftChoice if input.gift_options.is_empty() => {
            problems.push(FieldError::new(
                "gift_options",
                "required",
                "a gift choice needs gift options",
            ));
        }
        crate::model::Kind::FreeShipping if scope != Scope::Shipping => {
            problems.push(FieldError::new(
                "scope",
                "invalid",
                "free shipping has scope `shipping`",
            ));
        }
        crate::model::Kind::FlashSale
        | crate::model::Kind::BuyXGetY
        | crate::model::Kind::BundlePrice
        | crate::model::Kind::AddOn
        | crate::model::Kind::GiftChoice
            if scope != Scope::Items =>
        {
            problems.push(FieldError::new("scope", "invalid", "this kind has scope `items`"));
        }
        _ => {}
    }
    if scope == Scope::Shipping && input.kind != crate::model::Kind::FreeShipping {
        needs_value(problems);
    }
    if input.trigger() != Trigger::Voucher
        && (input.claimable
            || input.voucher_quota.is_some()
            || input.per_user_claim_limit.is_some())
    {
        problems.push(FieldError::new(
            "trigger",
            "invalid",
            "claims and voucher quotas need trigger `voucher`",
        ));
    }
    if input.reward_selection == Some(RewardSelection::Specified)
        && input.kind != crate::model::Kind::BuyXGetY
    {
        problems.push(FieldError::new("reward_selection", "invalid", "only for `buy_x_get_y`"));
    }
}

fn check_lists(input: &PromotionInput, problems: &mut Vec<FieldError>) {
    if input.customer_groups.len() > 100 || input.channels.len() > 50 || input.conditions.len() > 20
    {
        problems.push(FieldError::new(
            "conditions",
            "length",
            "too many groups, channels or conditions",
        ));
    }
    if input.channels.iter().any(|channel| channel.is_empty() || channel.len() > 64) {
        problems.push(FieldError::new("channels", "length", "each channel is 1 to 64 characters"));
    }
    for (index, condition) in input.conditions.iter().enumerate() {
        let field = format!("conditions[{index}]");
        let numeric = condition.kind == ConditionKind::MinItems;
        let operator_ok = if numeric {
            matches!(condition.operator, Operator::Gte | Operator::Lte)
                && condition.values.len() == 1
                && condition.values[0].parse::<i64>().is_ok()
        } else {
            matches!(condition.operator, Operator::In | Operator::NotIn)
        };
        if !operator_ok || condition.values.len() > 200 {
            problems.push(FieldError::new(
                &field,
                "invalid",
                "`min_items` takes `gte` / `lte` and one number; other kinds take `in` / `not_in`",
            ));
        }
    }
    for (index, window) in input.schedules.iter().enumerate() {
        if weekday_mask(&window.weekdays) == 0
            || window.weekdays.iter().any(|day| !(1..=7).contains(day))
            || !(0..1440).contains(&window.start_minute)
            || !(1..=1440).contains(&window.end_minute)
            || window.end_minute <= window.start_minute
        {
            problems.push(FieldError::new(
                format!("schedules[{index}]"),
                "invalid",
                "weekdays 1 to 7, minutes 0 to 1440, end after start",
            ));
        }
    }
    for (index, tier) in input.tiers.iter().enumerate() {
        let one_value = tier.value_bp.is_some() != tier.value.is_some();
        if tier.threshold < 1
            || !one_value
            || tier.value_bp.is_some_and(|bp| !(1..=10_000).contains(&bp))
            || tier.value.is_some_and(|value| !value.is_positive())
        {
            problems.push(FieldError::new(
                format!("tiers[{index}]"),
                "invalid",
                "a positive threshold and exactly one of `value_bp` / `value`",
            ));
        }
    }
    if !input.funding.is_empty() {
        let total: i64 = input.funding.iter().map(|share| i64::from(share.share_bp)).sum();
        if total != 10_000 || input.funding.iter().any(|share| share.share_bp <= 0) {
            problems.push(FieldError::new(
                "funding",
                "promotions.invalid_funding",
                "shares must add up to 10000",
            ));
        }
    }
    let gifts: BTreeSet<Uuid> = input.gift_options.iter().map(|gift| gift.variant_id).collect();
    if gifts.len() != input.gift_options.len()
        || input.gift_options.iter().any(|gift| gift.stock_limit.is_some_and(|limit| limit < 0))
    {
        problems.push(FieldError::new(
            "gift_options",
            "invalid",
            "each gift once, stock limit not negative",
        ));
    }
}
