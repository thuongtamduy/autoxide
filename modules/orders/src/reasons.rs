//! Danh sách lý do của `orders` (bước 51 P18, D-223): service kiểm mã bằng `ReasonCodes::ensure`,
//! dòng dữ liệu lưu mã, không lưu nhãn. Khách đổi nhãn, thêm mã, tắt mã không phải `legal`.

use autoxide_kernel::reasons::{ReasonDefault, ReasonList};

/// Lý do ẩn đánh giá (Luật 19/2023: chỉ ẩn đánh giá trái quy định — mã `legal` không tắt được).
pub(crate) const REVIEW_HIDDEN: &str = "orders.review_hidden";
/// Lý do từ chối yêu cầu trả hàng.
pub(crate) const RETURN_REJECTED: &str = "orders.return_rejected";
/// Lý do trả hàng; `allowed_return_reasons` của chính sách lấy mã từ đây (D-202, D-209).
pub(crate) const RETURN_REASON: &str = "orders.return_reason";
/// Lý do chênh lệch tiền két khi đóng ca (D-227).
pub(crate) const CASH_VARIANCE: &str = "orders.cash_variance";
/// Lý do chặn thanh toán khi nhận hàng của một người mua (D-194).
pub(crate) const COD_BLOCK: &str = "orders.cod_block";

pub(crate) fn lists() -> Vec<ReasonList> {
    vec![
        ReasonList::new(
            REVIEW_HIDDEN,
            vec![
                ReasonDefault::new("unlawful_content", "Content that breaks the law").legal(),
                ReasonDefault::new("personal_data", "Discloses personal data of someone").legal(),
                ReasonDefault::new("abusive_language", "Insults or abusive language").legal(),
                ReasonDefault::new("spam_or_advertising", "Spam or advertising").legal(),
                ReasonDefault::new("not_about_product", "Not about the product or the purchase")
                    .legal(),
            ],
        ),
        ReasonList::new(
            RETURN_REJECTED,
            vec![
                ReasonDefault::new("outside_policy", "Outside the return policy"),
                ReasonDefault::new("used_or_damaged_by_buyer", "Used or damaged by the buyer"),
                ReasonDefault::new("missing_items", "Items or accessories are missing"),
                ReasonDefault::new("no_evidence", "Not enough evidence for the reason given"),
                ReasonDefault::new("other", "Other reason"),
            ],
        ),
        ReasonList::new(
            RETURN_REASON,
            vec![
                ReasonDefault::new("changed_mind", "Changed my mind"),
                ReasonDefault::new("damaged", "Arrived damaged"),
                ReasonDefault::new("wrong_item", "Wrong item received"),
                ReasonDefault::new("not_as_described", "Not as described"),
                ReasonDefault::new("missing_parts", "Parts or accessories missing"),
                ReasonDefault::new("other", "Other reason"),
            ],
        ),
        ReasonList::new(
            CASH_VARIANCE,
            vec![
                ReasonDefault::new("counting_error", "Counting error"),
                ReasonDefault::new("wrong_change", "Wrong change given"),
                ReasonDefault::new("unrecorded_sale", "Sale not recorded"),
                ReasonDefault::new("theft_or_loss", "Theft or loss"),
                ReasonDefault::new("other", "Other reason"),
            ],
        ),
        ReasonList::new(
            COD_BLOCK,
            vec![
                ReasonDefault::new("refused_deliveries", "Refused several deliveries"),
                ReasonDefault::new("fake_orders", "Placed orders with fake details"),
                ReasonDefault::new("unreachable", "Could not be reached for delivery"),
                ReasonDefault::new("other", "Other reason"),
            ],
        ),
    ]
}
