//! Chính sách tách / gộp khách của các bảng của `conversations` (bước 38,
//! D-20261118-step38-relocation): cuộc trò chuyện, chặn và báo vi phạm theo nút; tin và con trỏ đọc
//! đi theo cuộc trò chuyện; đường dây, yêu cầu gọi lại, cuộc gọi theo nút; bộ đếm throttle và
//! webhook thoại là dữ liệu tạm, không đi đâu cả.

use autoxide_kernel::tenancy::Relocation;

pub(crate) fn relocation() -> Relocation {
    Relocation::new()
        .by_org(&[
            "conversations_threads",
            "conversations_blocks",
            "conversations_reports",
            "conversations_voice_lines",
            "conversations_callback_requests",
            "conversations_calls",
        ])
        .follow("conversations_messages", "thread_id", "conversations_threads")
        .follow("conversations_reads", "thread_id", "conversations_threads")
        // Số điện thoại mã hoá theo khách: mã hoá lại khi dời sang khách khác.
        .sealed(
            "conversations_callback_requests",
            "phone_sealed",
            "conversations.callback_phone",
            "id",
        )
        // Webhook đã xử lý: dữ liệu tạm của nhà cung cấp, không đi theo.
        .ephemeral(&["conversations_throttle", "conversations_voice_events"])
}
