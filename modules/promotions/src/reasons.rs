//! Lý do người mua đọc được (Luật 19/2023): `code` là i18n key `promotions.reason.*`, câu tiếng
//! Anh ở đây là bản mặc định, bản tiếng Việt ở `locales/vi.json`. Dịch bằng `Translations` của
//! kernel như thông báo lỗi.

use std::collections::BTreeMap;

use autoxide_kernel::error::{Error, ErrorKind};
use autoxide_kernel::i18n::{Language, Translations};

use crate::api::Reason;
use crate::engine::ReasonDraft;

/// Câu tiếng Anh của từng lý do.
fn english(code: &str) -> &'static str {
    match code {
        "promotions.reason.percent_off" => "{percent}% off",
        "promotions.reason.amount_off" => "{amount} off each item",
        "promotions.reason.fixed_price" => "Fixed price {price}",
        "promotions.reason.order_percent" => "{percent}% off orders from {min_order}",
        "promotions.reason.order_amount" => "{amount} off orders from {min_order}",
        "promotions.reason.free_shipping" => "{amount} off shipping",
        "promotions.reason.tiered" => "Reached {threshold}: {amount} off",
        "promotions.reason.flash_sale" => "Flash sale price",
        "promotions.reason.buy_x_get_y" => "Buy {buy} get {get}",
        "promotions.reason.bundle_price" => "{quantity} items for {price}",
        "promotions.reason.add_on" => "Add-on deal price",
        "promotions.reason.gift" => "Free gift with your order",
        "promotions.reason.capped_by_law" => {
            "The discount is limited to {max}% of the price before the promotion, as the law requires"
        }
        "promotions.reason.capped_by_floor" => {
            "The discount is limited so the price stays above the store's price floor"
        }
        "promotions.reason.no_reference_price" => {
            "This product has no price before the promotion yet, so it cannot be discounted"
        }
        "promotions.reason.budget_exhausted" => "The promotion has used up its budget",
        "promotions.reason.usage_exhausted" => "The promotion has no uses left",
        "promotions.reason.not_running" => "The promotion has not started or has ended",
        "promotions.reason.outside_schedule" => "Only during the announced hours",
        "promotions.reason.not_applicable" => "No item in the cart qualifies",
        "promotions.reason.customer_group" => "Only for selected customer groups",
        "promotions.reason.channel" => "Only on selected sales channels",
        "promotions.reason.first_order_only" => "Only for your first order",
        "promotions.reason.new_customer_only" => "Only for new customers",
        "promotions.reason.returning_customer_only" => "Only for returning customers",
        "promotions.reason.condition_unmet" => "Condition {condition} is not met",
        "promotions.reason.min_items" => "Needs {count} items in the cart",
        "promotions.reason.min_quantity" => "Add {count} more items to get the discount",
        "promotions.reason.add_more" => "Add {amount} to use it (orders from {min_order})",
        "promotions.reason.no_shipping_fee" => "The cart has no shipping fee yet",
        "promotions.reason.flash_sold_out" => "The flash-sale units are sold out",
        "promotions.reason.add_on_missing" => "Add the add-on item to get the deal price",
        "promotions.reason.gift_out_of_stock" => "The gifts are out of stock",
        "promotions.reason.not_combinable" => "Cannot be combined with the promotions applied",
        "promotions.reason.too_many_vouchers" => {
            "At most {max} vouchers per promotion class and payer"
        }
        "promotions.reason.code_invalid" => "The code does not exist",
        "promotions.reason.code_expired" => "The code has expired or has not started",
        "promotions.reason.voucher_not_owned" => "The voucher is not in your wallet",
        "promotions.reason.voucher_used" => "The voucher has been used",
        "promotions.reason.voucher_expired" => "The voucher has expired",
        "promotions.reason.sign_in_required" => "Sign in to use vouchers",
        _ => "The promotion does not apply",
    }
}

/// Câu của lý do bằng ngôn ngữ `locale` (không có bản dịch thì tiếng Anh).
pub(crate) fn render(translations: &Translations, draft: &ReasonDraft, locale: &str) -> Reason {
    let language = Language::from_tag(locale).unwrap_or(Language::En);
    let mut error = Error::new(ErrorKind::Validation, draft.code, english(draft.code));
    for (name, value) in &draft.params {
        error = error.with_param(*name, value);
    }
    Reason {
        code: draft.code.to_owned(),
        params: draft
            .params
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect::<BTreeMap<_, _>>(),
        text: translations.message(&error, language),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moi_ly_do_co_ban_dich_tieng_viet() {
        let vi: serde_json::Value =
            serde_json::from_str(include_str!("../locales/vi.json")).unwrap();
        for code in [
            "promotions.reason.percent_off",
            "promotions.reason.add_more",
            "promotions.reason.capped_by_law",
            "promotions.reason.too_many_vouchers",
            "promotions.reason.sign_in_required",
        ] {
            assert!(vi.get(code).is_some(), "{code}");
            assert_ne!(english(code), english("x"), "{code}");
        }
    }
}
