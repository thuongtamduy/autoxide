//! Chính sách tách / gộp khách của các bảng của `automations` (bước 38, D-20261118-step38-relocation):
//! luật và lượt chạy theo nút; khoá chống chạy lặp đi theo lượt chạy; bộ đếm số liệu theo giờ và
//! sổ event đã thấy là dữ liệu tạm, không đi đâu cả.

use autoxide_kernel::tenancy::Relocation;

pub(crate) fn relocation() -> Relocation {
    Relocation::new()
        .by_org(&["automations_rules", "automations_runs"])
        .follow("automations_run_keys", "run_id", "automations_runs")
        .ephemeral(&["automations_metric_hours", "automations_metric_seen"])
}
