//! Chính sách tách / gộp khách của các bảng của `promotions` (bước 38, D-20261118-step38-relocation):
//! chương trình và lượt giữ của đơn theo nút; phần con của chương trình (mã, voucher, bộ đếm, số
//! liệu, lưu vết, thông báo) đi theo chương trình; số phiên bản bộ nhớ và bộ đếm chống dò là dữ
//! liệu tạm.

use autoxide_kernel::tenancy::Relocation;

pub(crate) fn relocation() -> Relocation {
    let mut relocation =
        Relocation::new().by_org(&["promotions_promotions", "promotions_redemptions"]);
    for table in [
        "promotions_translations",
        "promotions_targets",
        "promotions_customer_groups",
        "promotions_channels",
        "promotions_conditions",
        "promotions_schedules",
        "promotions_tiers",
        "promotions_gift_options",
        "promotions_codes",
        "promotions_vouchers",
        "promotions_flash_items",
        "promotions_funding",
        "promotions_participations",
        "promotions_customer_usage",
        "promotions_daily_usage",
        "promotions_stats_daily",
        "promotions_versions",
        "promotions_notices",
        "promotions_reminders",
    ] {
        relocation = relocation.follow(table, "promotion_id", "promotions_promotions");
    }
    relocation
        .follow("promotions_redemption_holds", "redemption_id", "promotions_redemptions")
        .ephemeral(&["promotions_revisions", "promotions_throttle"])
}
