## D-20261119-step61-conversations · Module `conversations` (bước 61): cuộc trò chuyện, chống trùng, thu hồi, thông báo, chống lạm dụng, mặc định — 2026-11-19 — agent quyết

Chủ đề: Thương mại

Bối cảnh: đặc tả `docs/steps/61-conversations.md` để agent tự quyết thời gian thu hồi, cách gom thông báo, mẫu che liên
hệ; vài chỗ đặc tả cần chọn cách làm. Mục này ghi các lựa chọn của ba mục đầu của bước (chat); gọi điện (D-232) ghi
quyết định riêng khi làm.

Quyết định:

1. **Chỉ người mua có tài khoản** mở cuộc trò chuyện (`/api/v1/conversations/me/…`); khách vãng lai xác thực OTP chưa
   làm — chat cần stream SSE, mà stream cần đăng nhập. Người mua không có permission (như `support`, D-20261118-step53-support).
2. **Một cuộc đang mở cho mỗi (người mua, nút, bản ghi)**: cột `open_key` có giá trị khi mở, NULL khi đóng; unique trên
   cột NULL chạy trên cả hai database. Người mua viết lại về cùng sản phẩm thì vào cuộc đang mở (200), không mở cuộc
   mới. `subject` kiểm qua sổ đối tượng (D-123).
3. **Gửi lại không trùng**: unique (`tenant_id`, `thread_id`, `client_id`), chèn bằng `ON CONFLICT DO NOTHING` (D-158);
   gửi lại cùng `client_id` trả tin cũ (200). Tin hệ thống dùng `client_id` dành riêng `auto:<tin>` / `call:<cuộc gọi>`,
   client không được dùng hai tiền tố đó.
4. **Thu hồi trong 2 phút**, chỉ tin của chính mình: bỏ chữ và ảnh, giữ dòng với dấu `recalled`. Không sửa tin.
5. **Bộ đếm chưa đọc** theo phía (`unread_staff`, `unread_customer`) cộng trong câu `UPDATE`; con trỏ đọc theo người
   (`conversations_reads`) chỉ tiến. Đọc tới tin mới nhất thì bộ đếm của phía mình về 0, trừ khi có tin mới hơn tới
   giữa lúc đọc.
6. **Thông báo gom lô**: mỗi tin hẹn job sau `AUTOXIDE_CONVERSATIONS_NOTIFY_AFTER_MINUTES` (mặc định 5) phút, khoá chống
   trùng theo (cuộc, phía nhận, khung thời gian) — một khung, một thông báo. Tới giờ phía nhận đã đọc tới tin đó thì
   không báo. Phía nhân viên chỉ báo người được giao (cuộc chưa giao nằm ở hộp thư chung).
7. **Che liên hệ ngoài sàn** là cài đặt theo nút `conversations.mask_contacts` (tắt mặc định) và
   `conversations.mask_terms`: che lúc lưu số điện thoại dạng `+…`, `0…`, `84…` 9–15 chữ số, email, link / tên miền và
   từ nút khai; giá tiền và mã đơn không bị che. Bản gốc không giữ.
8. **Chống lạm dụng**: throttle `AUTOXIDE_CONVERSATIONS_MESSAGES_PER_MINUTE` (20) mỗi người; chặn theo (nút, người mua)
   bởi `conversations.manage`; báo vi phạm từ người mua hay nhân viên, xử lý bởi `conversations.manage`. Xác thực tài
   khoản (NĐ 147/2024) dùng `PostingVerification` của kernel (D-202) khi nút bật
   `conversations.require_verified_account` hay khách đòi ở mọi chỗ đăng nội dung.
9. **Trả lời tự động** gọi `AutoResponder` của `support` trong job sau khi tin đã lưu; quyết `Draft` lưu thành
   `draft_reply` của cuộc; `Handoff` bật `needs_agent`. "Gặp nhân viên" dừng tự động trong cuộc đó. Nhân viên trả lời
   thì đếm lại số lần tự động liên tiếp và bỏ bản nháp cũ.
10. **Chuyển thành phiếu** gọi `SupportIntake::open` với `reference = conversations.thread:<id>` (gọi lại trả cùng
    phiếu), 20 tin gần nhất, liên kết về cuộc trò chuyện (đăng ký loại đối tượng `conversations.thread`).
11. **Hạn lưu** `AUTOXIDE_CONVERSATIONS_RETENTION_DAYS` (365) tính từ tin cuối; job mỗi ngày xoá cuộc quá hạn cùng tin,
    con trỏ đọc, báo vi phạm. Xoá tài khoản: bỏ chữ và ảnh của tin người mua viết, bỏ liên kết tài khoản, đóng cuộc;
    xoá chặn và con trỏ đọc của người đó.
12. **Nhân viên sàn** đọc theo phạm vi permission như mọi module: cuộc của gian hàng thuộc nhánh của gian hàng; người có
    quyền ở cả khách (sàn) thấy hết như ở `support`. Giới hạn "chỉ khi có tranh chấp" chưa làm — cần cơ chế quyền theo
    vụ việc chung cho nhiều module.

Đã cân nhắc: lưu bản gốc rồi che lúc hiển thị — vẫn giữ liên hệ người mua không muốn sàn giữ; một bảng việc riêng cho
thông báo — job có khoá chống trùng đã đủ; bộ quyết trả lời tự động riêng cho chat — D-230 buộc dùng của `support`.

Hệ quả: căn cứ Luật 91/2025/QH15, NĐ 356/2025 (dữ liệu cá nhân, hạn lưu, xoá) và NĐ 147/2024/NĐ-CP (xác thực tài khoản
đăng nội dung) — cần đối chiếu văn bản gốc, nhất là phạm vi NĐ 147 với chat riêng tư.
