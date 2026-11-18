//! Chính sách tách / gộp khách của các bảng của `analytics` (bước 38, D-20261118-step38-relocation):
//! sự kiện và bảng gộp theo website (`site_org_id`, không phải `org_id`) ở lại khách nguồn khi tách —
//! khách mới bắt đầu số liệu từ đầu; khoá ẩn danh theo ngày, dấu đã gộp và bộ đếm chống dò là dữ liệu tạm.

use autoxide_kernel::tenancy::Relocation;

pub(crate) fn relocation() -> Relocation {
    Relocation::new()
        .stay(&["analytics_events", "analytics_daily", "analytics_funnel_daily"])
        .ephemeral(&["analytics_day_keys", "analytics_rollups", "analytics_throttle"])
}
