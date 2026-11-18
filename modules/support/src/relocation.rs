//! Chính sách tách / gộp khách của các bảng của `support` (bước 38, D-20261118-step38-relocation):
//! phiếu theo nút; tin nhắn và liên kết đi theo phiếu; bộ đếm chống dò và mã chống spam là dữ liệu
//! tạm, không đi đâu cả.

use autoxide_kernel::tenancy::Relocation;

pub(crate) fn relocation() -> Relocation {
    Relocation::new()
        .by_org(&["support_tickets"])
        .follow("support_messages", "ticket_id", "support_tickets")
        .follow("support_links", "ticket_id", "support_tickets")
        .ephemeral(&["support_throttle", "support_antispam_nonces"])
}
