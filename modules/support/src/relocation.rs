//! Chính sách tách / gộp khách của các bảng của `support` (bước 38, D-20261118-step38-relocation):
//! phiếu, nhóm và quy tắc phân công theo nút; tin nhắn và liên kết đi theo phiếu, thành viên theo
//! nhóm; bộ đếm chống dò và mã chống spam là dữ liệu
//! tạm, không đi đâu cả.

use autoxide_kernel::tenancy::Relocation;

pub(crate) fn relocation() -> Relocation {
    Relocation::new()
        .by_org(&[
            "support_tickets",
            "support_teams",
            "support_routing_rules",
            "support_policies",
            "support_macros",
        ])
        .follow("support_messages", "ticket_id", "support_tickets")
        .follow("support_team_members", "team_id", "support_teams")
        .follow("support_links", "ticket_id", "support_tickets")
        .follow("support_reply_drafts", "ticket_id", "support_tickets")
        .ephemeral(&["support_throttle", "support_antispam_nonces"])
}
