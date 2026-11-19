//! Chính sách tách / gộp khách của các bảng của `conversations` (bước 38,
//! D-20261118-step38-relocation): cuộc trò chuyện, chặn và báo vi phạm theo nút; tin và con trỏ đọc
//! đi theo cuộc trò chuyện; bộ đếm throttle là dữ liệu tạm, không đi đâu cả.

use autoxide_kernel::tenancy::Relocation;

pub(crate) fn relocation() -> Relocation {
    Relocation::new()
        .by_org(&["conversations_threads", "conversations_blocks", "conversations_reports"])
        .follow("conversations_messages", "thread_id", "conversations_threads")
        .follow("conversations_reads", "thread_id", "conversations_threads")
        .ephemeral(&["conversations_throttle"])
}
