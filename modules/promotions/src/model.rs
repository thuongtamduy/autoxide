//! Mô hình trong bộ nhớ của một chương trình khuyến mại: các giá trị liệt kê (lưu dạng chữ trong
//! database) và [`Program`] — chương trình đã biên dịch (điều kiện thành struct, mục tiêu thành
//! tập) mà bộ tính dùng. Không chạm database.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::api::{GiftReturnPolicy, PayerKind, PromotionClass};

/// Một giá trị liệt kê lưu bằng chữ: `as_str` / `parse` khớp nhau.
macro_rules! text_enum {
    ($(#[$meta:meta])* $name:ident as $schema:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord,
            serde::Serialize, serde::Deserialize, utoipa::ToSchema,
        )]
        #[serde(rename_all = "snake_case")]
        #[schema(as = $schema)]
        pub(crate) enum $name { $($variant),+ }

        impl $name {
            // Chữ lưu database trùng tên serde (`snake_case`), test bên dưới kiểm.
            pub(crate) fn as_str(self) -> &'static str {
                match self { $($name::$variant => $text),+ }
            }

            pub(crate) fn parse(text: &str) -> Option<Self> {
                match text { $($text => Some($name::$variant),)+ _ => None }
            }
        }
    };
}

text_enum!(
    /// Kind of promotion.
    Kind as PromotionsKind {
        PercentOff => "percent_off",
        AmountOff => "amount_off",
        FixedPrice => "fixed_price",
        BuyXGetY => "buy_x_get_y",
        FreeShipping => "free_shipping",
        FlashSale => "flash_sale",
        Tiered => "tiered",
        BundlePrice => "bundle_price",
        AddOn => "add_on",
        GiftChoice => "gift_choice",
    }
);

text_enum!(
    /// What the promotion discounts: `items` (product class), `order` or `shipping`.
    Scope as PromotionsScope { Items => "items", Order => "order", Shipping => "shipping" }
);

impl Scope {
    pub(crate) fn class(self) -> PromotionClass {
        match self {
            Scope::Items => PromotionClass::Product,
            Scope::Order => PromotionClass::Order,
            Scope::Shipping => PromotionClass::Shipping,
        }
    }
}

text_enum!(
    /// How the promotion reaches buyers: automatically, by coupon code or by wallet voucher.
    Trigger as PromotionsTrigger { Automatic => "automatic", Code => "code", Voucher => "voucher" }
);

text_enum!(
    Status as PromotionsStatus {
        Draft => "draft",
        Scheduled => "scheduled",
        Active => "active",
        Paused => "paused",
        Ended => "ended",
        Cancelled => "cancelled",
    }
);

text_enum!(Exclusivity as PromotionsExclusivity { None => "none", Class => "class", Global => "global" });

text_enum!(
    RewardSelection as PromotionsRewardSelection { Cheapest => "cheapest", SameItem => "same_item", Specified => "specified" }
);

text_enum!(
    /// Period of the per-customer limit.
    Period as PromotionsPeriod { Lifetime => "lifetime", Day => "day", Week => "week", Month => "month" }
);

text_enum!(
    /// Legal kind of programme: `regular` or a `national_focus` promotion programme.
    ProgramKind as PromotionsProgramKind { Regular => "regular", NationalFocus => "national_focus" }
);

text_enum!(GiftPolicy as PromotionsGiftPolicy { ReturnGift => "return_gift", DeductGiftValue => "deduct_gift_value", Keep => "keep" });

impl GiftPolicy {
    pub(crate) fn api(self) -> GiftReturnPolicy {
        match self {
            GiftPolicy::ReturnGift => GiftReturnPolicy::ReturnGift,
            GiftPolicy::DeductGiftValue => GiftReturnPolicy::DeductGiftValue,
            GiftPolicy::Keep => GiftReturnPolicy::Keep,
        }
    }
}

text_enum!(
    /// Stacking class: `product`, `order` or `shipping`.
    ClassName as PromotionsClassName { Product => "product", Order => "order", Shipping => "shipping" }
);

impl ClassName {
    pub(crate) fn class(self) -> PromotionClass {
        match self {
            ClassName::Product => PromotionClass::Product,
            ClassName::Order => PromotionClass::Order,
            ClassName::Shipping => PromotionClass::Shipping,
        }
    }

    pub(crate) fn of(class: PromotionClass) -> Self {
        match class {
            PromotionClass::Product => ClassName::Product,
            PromotionClass::Order => ClassName::Order,
            PromotionClass::Shipping => ClassName::Shipping,
        }
    }
}

text_enum!(TargetType as PromotionsTargetType { Product => "product", Variant => "variant", Category => "category" });

text_enum!(Role as PromotionsRole { Condition => "condition", Reward => "reward" });

text_enum!(ThresholdKind as PromotionsThresholdKind { Quantity => "quantity", Amount => "amount" });

text_enum!(
    Payer as PromotionsPayer {
        Platform => "platform",
        Seller => "seller",
        Brand => "brand",
        PaymentPartner => "payment_partner",
    }
);

impl Payer {
    pub(crate) fn api(self) -> PayerKind {
        match self {
            Payer::Platform => PayerKind::Platform,
            Payer::Seller => PayerKind::Seller,
            Payer::Brand => PayerKind::Brand,
            Payer::PaymentPartner => PayerKind::PaymentPartner,
        }
    }
}

text_enum!(
    /// Kind of extra condition.
    ConditionKind as PromotionsConditionKind {
        PaymentMethod => "payment_method",
        Carrier => "carrier",
        ShipToProvince => "ship_to_province",
        MembershipTier => "membership_tier",
        Client => "client",
        Brand => "brand",
        SellerOrg => "seller_org",
        Attribute => "attribute",
        NewCustomer => "new_customer",
        ReturningCustomer => "returning_customer",
        MinItems => "min_items",
        ExcludeDiscounted => "exclude_discounted",
        ExcludeTargets => "exclude_targets",
    }
);

text_enum!(Operator as PromotionsOperator { In => "in", NotIn => "not_in", Gte => "gte", Lte => "lte" });

text_enum!(Purpose as PromotionsPurpose { Redeem => "redeem", Claim => "claim" });

/// Lớp từ dạng chữ (`combines_with` lưu danh sách nối `,`).
pub(crate) fn parse_class(text: &str) -> Option<PromotionClass> {
    ClassName::parse(text).map(ClassName::class)
}

/// Mục tiêu đã biên dịch: biến thể, sản phẩm, danh mục.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TargetSet {
    pub variants: BTreeSet<Uuid>,
    pub products: BTreeSet<Uuid>,
    pub categories: BTreeSet<Uuid>,
}

impl TargetSet {
    pub(crate) fn is_empty(&self) -> bool {
        self.variants.is_empty() && self.products.is_empty() && self.categories.is_empty()
    }

    pub(crate) fn insert(&mut self, kind: TargetType, id: Uuid) {
        match kind {
            TargetType::Variant => self.variants.insert(id),
            TargetType::Product => self.products.insert(id),
            TargetType::Category => self.categories.insert(id),
        };
    }

    pub(crate) fn matches(&self, variant: Uuid, product: Uuid, categories: &[Uuid]) -> bool {
        self.variants.contains(&variant)
            || self.products.contains(&product)
            || categories.iter().any(|category| self.categories.contains(category))
    }
}

/// Điều kiện đã biên dịch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Condition {
    pub kind: ConditionKind,
    pub operator: Operator,
    pub values: Vec<String>,
}

/// Khung giờ trong tuần theo múi giờ cửa hàng: `weekdays` bit 0 = thứ Hai … bit 6 = Chủ nhật;
/// phút `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Window {
    pub weekdays: i32,
    pub start_minute: i32,
    pub end_minute: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Tier {
    pub threshold_kind: ThresholdKind,
    pub threshold: i64,
    pub value_bp: Option<i32>,
    pub value_minor: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FlashItem {
    pub variant_id: Uuid,
    pub sale_price_minor: Option<i64>,
    pub value_bp: Option<i32>,
    pub quantity_limit: Option<i64>,
    pub quantity_used: i64,
    pub per_customer_quantity_limit: Option<i64>,
}

impl FlashItem {
    /// Suất còn (thật, theo số đã giữ); `None` = không giới hạn.
    pub(crate) fn remaining(&self) -> Option<i64> {
        self.quantity_limit.map(|limit| (limit - self.quantity_used).max(0))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GiftOption {
    pub variant_id: Uuid,
    pub stock_limit: Option<i64>,
    pub quantity_used: i64,
}

impl GiftOption {
    pub(crate) fn remaining(&self) -> Option<i64> {
        self.stock_limit.map(|limit| (limit - self.quantity_used).max(0))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Funding {
    pub payer: Payer,
    pub payer_org_id: Option<Uuid>,
    pub share_bp: i32,
}

/// Chương trình đã biên dịch: mọi thứ bộ tính cần, không đọc JSON hay database khi tính.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Program {
    pub id: Uuid,
    pub org_id: Uuid,
    pub kind: Kind,
    pub scope: Scope,
    pub trigger: Trigger,
    pub status: Status,
    pub currency: String,
    pub value_bp: Option<i32>,
    pub value_minor: Option<i64>,
    pub min_order_minor: Option<i64>,
    pub min_quantity: Option<i32>,
    pub max_discount_minor: Option<i64>,
    pub buy_quantity: Option<i32>,
    pub get_quantity: Option<i32>,
    pub reward_selection: Option<RewardSelection>,
    pub reward_value_bp: Option<i32>,
    pub max_applications: Option<i32>,
    pub bundle_quantity: Option<i32>,
    pub bundle_price_minor: Option<i64>,
    pub max_add_on_quantity: Option<i32>,
    pub gift_quantity: Option<i32>,
    pub first_order_only: bool,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub priority: i32,
    pub exclusivity: Exclusivity,
    pub combines_with: BTreeSet<PromotionClass>,
    pub stop_after: bool,
    pub usage_limit: Option<i64>,
    pub usage_count: i64,
    pub per_customer_limit: Option<i64>,
    pub per_customer_period: Period,
    pub daily_usage_limit: Option<i64>,
    pub budget_minor: Option<i64>,
    pub budget_used_minor: i64,
    pub return_usage_on_cancel: bool,
    pub gift_return_policy: GiftPolicy,
    pub program_kind: ProgramKind,
    pub public: bool,
    /// Tiêu đề và điều kiện theo ngôn ngữ.
    pub texts: Vec<(String, String, String)>,
    pub conditions_targets: TargetSet,
    pub reward_targets: TargetSet,
    pub customer_groups: BTreeSet<Uuid>,
    pub channels: BTreeSet<String>,
    pub conditions: Vec<Condition>,
    pub windows: Vec<Window>,
    pub tiers: Vec<Tier>,
    pub flash_items: Vec<FlashItem>,
    pub gift_options: Vec<GiftOption>,
    pub funding: Vec<Funding>,
}

impl Program {
    pub(crate) fn class(&self) -> PromotionClass {
        self.scope.class()
    }

    /// Tiêu đề và điều kiện theo `locale`, không có thì `en`, không có nữa thì bản đầu.
    pub(crate) fn text(&self, locale: &str) -> (String, String) {
        let primary = locale.split(['-', '_']).next().unwrap_or(locale);
        self.texts
            .iter()
            .find(|(found, _, _)| found == locale)
            .or_else(|| self.texts.iter().find(|(found, _, _)| found == primary))
            .or_else(|| self.texts.iter().find(|(found, _, _)| found == "en"))
            .or_else(|| self.texts.first())
            .map(|(_, title, terms)| (title.clone(), terms.clone()))
            .unwrap_or_default()
    }

    /// Còn lượt dùng tổng (theo số đã giữ).
    pub(crate) fn remaining_uses(&self) -> Option<i64> {
        self.usage_limit.map(|limit| (limit - self.usage_count).max(0))
    }

    /// Ngân sách còn (đơn vị nhỏ nhất).
    pub(crate) fn remaining_budget(&self) -> Option<i64> {
        self.budget_minor.map(|budget| (budget - self.budget_used_minor).max(0))
    }

    pub(crate) fn flash_item(&self, variant: Uuid) -> Option<&FlashItem> {
        self.flash_items.iter().find(|item| item.variant_id == variant)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tên trong JSON (serde `snake_case`) trùng chữ lưu database.
    #[test]
    fn chu_luu_trung_ten_json() {
        fn same<T: serde::Serialize>(value: T, text: &str) {
            assert_eq!(serde_json::to_value(value).unwrap(), serde_json::json!(text));
        }
        same(Kind::BuyXGetY, Kind::BuyXGetY.as_str());
        same(Kind::GiftChoice, Kind::GiftChoice.as_str());
        same(ConditionKind::ShipToProvince, ConditionKind::ShipToProvince.as_str());
        same(Operator::NotIn, Operator::NotIn.as_str());
        same(GiftPolicy::DeductGiftValue, GiftPolicy::DeductGiftValue.as_str());
        same(Payer::PaymentPartner, Payer::PaymentPartner.as_str());
        same(ProgramKind::NationalFocus, ProgramKind::NationalFocus.as_str());
        same(RewardSelection::SameItem, RewardSelection::SameItem.as_str());
        same(ClassName::Shipping, ClassName::Shipping.as_str());
    }
}
