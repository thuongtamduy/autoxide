## D-20261127-voice-recording-provider-off · Bản ghi âm còn ở nhà cung cấp thoại có bộ nối tắt: không xoá dấu vết; xoá tài khoản giao cho job dọn theo hạn — 2026-11-27 — agent quyết

Chủ đề: Module nội dung, file, liên lạc

Quyết định (`modules/conversations/src/voice/recordings.rs`, bước 70 P9):

1. Xoá theo yêu cầu (`DELETE /api/v1/conversations/calls/{id}/recording`) và job dọn theo hạn lưu: còn
   `provider_recording_id` mà bộ nối của nhà cung cấp giữ bản ghi không bật thì `erase_recording` báo
   `503 conversations.voice_provider_unavailable` **trước** khi đụng tới nơi lưu hay database; dòng còn nguyên.
2. Job dọn theo hạn bỏ qua dòng lỗi, dọn các dòng khác, rồi báo lỗi đầu tiên để được thử lại (rồi `jobs_dead` — tín hiệu
   cho người vận hành). Job `Purger` làm vậy cả giữa các khách trên cùng database: một khách lỗi không chặn khách sau
   (`warn!` kèm `tenant_id`, giữ lỗi đầu, quét hết rồi báo).
3. Xoá tài khoản (`auth.user_deleted`, cả đường "nghỉ" `RecordingEraser::Dormant` của D-20261125-dormant-modules)
   **không** trả lỗi cho event khi bộ nối tắt: bộ phát event của kernel (`crates/kernel/src/event/dispatcher.rs`) giao
   theo thứ tự đăng ký và dừng ở bên nhận đầu tiên lỗi, nên một lỗi ở đây làm event bị thử lại mãi, `conversations` không
   ẩn danh tin nhắn / yêu cầu gọi lại và mọi module đăng ký sau không nhận được event. Thay vào đó: xoá bản sao trong nơi
   lưu (`recording_ref`, nội dung trước, cột sau), **giữ** `provider_recording_id`, đưa `recording_expires_at` về bây giờ
   (hạn sớm hơn thì giữ) để job dọn theo hạn nhặt dòng, ghi `tracing::warn!`, xoá / ẩn danh mọi thứ khác như thường, trả
   `Ok`. Không ghi event `recording_deleted` lúc đó: job dọn ghi khi xoá xong ở nhà cung cấp. Đường "nghỉ" vẫn báo lỗi khi
   thiếu kho (bản sao trong kho không có ai dọn khi module tắt).
4. Job tải bản ghi (`fetch_recording`) gặp dòng không có `recording_ref` mà đã có `recording_expires_at` (bản sao đã bị
   xoá theo mục 3) thì không tải lại: không đưa dữ liệu của tài khoản đã xoá trở lại nơi lưu; job dọn theo hạn xoá ở nhà
   cung cấp.

Lý do: xoá dấu vết khi bản ghi còn nằm ở nhà cung cấp làm dữ liệu cá nhân nằm lại không ai biết (Luật số hai, bảo vệ dữ
liệu cá nhân). Nhưng tín hiệu "chưa xoá được" phải đi qua đường không chặn việc khác: job dọn theo hạn có thử lại và
dead-letter queue, event xoá tài khoản thì không được kẹt vì một module.

Đã cân nhắc: để `erase_recording` bỏ qua nhà cung cấp và ghi cảnh báo (bản ghi nằm lại không dấu vết — đúng lỗi cần
sửa); trả lỗi cho event xoá tài khoản (chặn ẩn danh và chặn bảy module đăng ký sau); thêm cột "chờ xoá ở nhà cung cấp"
(thêm migration trong khi `recording_expires_at` đã là hàng đợi của job dọn).
