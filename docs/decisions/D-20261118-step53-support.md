## D-20261118-step53-support · Module `support` (bước 53): phiếu, khách vãng lai, quyền của người mua, số phiếu, mặc định — 2026-11-18 — agent quyết

Chủ đề: Thương mại

Bối cảnh: đặc tả `docs/steps/53-support.md` để agent tự quyết một số chỗ, và có vài chỗ lệch với code thật
(D-204, D-222, D-230). Mục này ghi các lựa chọn của mục 1 (phiếu và tin nhắn); các mục sau của bước 53 ghi thêm file
`D-<ngày>-step53-<tên>.md` riêng.

Quyết định:

1. **Không có permission `support.request`.** Tài khoản người mua (`Realm::Customer`, bước 34) không mang permission nào,
   nên endpoint `/me/…` chỉ cần đăng nhập và chỉ chạm phiếu có `requester_user_id` là người gọi. Nhân viên cũng dùng được
   `/me/…` cho phiếu của chính họ. Đặc tả sửa theo.
2. **Liên lạc của khách vãng lai lưu nguyên văn đã chuẩn hoá** (email viết thường, số điện thoại E.164) thay cho
   `requester_phone_hash`: phải gửi được mã xác thực và thông báo tới đó, và email vốn cũng lưu nguyên văn. Response cho
   người mua / khách vãng lai che email (`a•••@example.com`) và số (`+84•••678`); chỉ nhân viên có
   `support.tickets.read` ở nút của phiếu thấy đầy đủ. Event không mang liên lạc. Xoá tài khoản thì bỏ liên lạc.
3. **Mã xác thực một lần (OTP) giống nhau cho email và SMS:** 6 chữ số, sống 15 phút, sai 5 lần thì hết dùng, xin mã mới
   tối đa 5 lần mỗi giờ cho một phiếu; database giữ bản băm có "muối" theo khách (`SecretKey::derive_for`). Mã gửi qua
   `Notifier` (loại `support.verification_code`, nhóm `security`). Link trong thư (đặc tả nói "email: link") là việc
   của giao diện: `AUTOXIDE_SUPPORT_GUEST_URL` có `{id}` và `{token}`, trang đó gọi `verify` với mã.
4. **Gộp phiếu vãng lai (`POST /me/claim`)**: gộp phiếu đã xác thực có số điện thoại mà tài khoản đã xác thực (hỏi
   `compliance::AccountVerification`), và — khi người gọi đưa một phiếu vãng lai đã xác thực kèm token — mọi phiếu đã xác
   thực cùng email / số với phiếu đó. `support` không đọc email của tài khoản (không phụ thuộc `auth`, luật kiến trúc 7),
   nên chứng minh email bằng phiếu đã qua OTP.
5. **Số phiếu**: dãy `support:tickets` của khách (`kernel::numbering`, không nhảy số), dạng 6 chữ số `000123`. Không đặt
   tiền tố theo nút: số chỉ để người mua đọc lại cho nhân viên, và một dãy chung không lộ quy mô từng cửa hàng.
6. **Loại phiếu mặc định** `order`, `product`, `payment`, `account`, `other` (cho tới khi nút khai danh sách riêng ở
   chính sách hỗ trợ, mục 3 của bước).
7. **Người được giao (mục 1)**: chỉ chính người gọi (đã có `support.tickets.reply` ở nút của phiếu); mục 2 mở rộng cho
   thành viên nhóm nhân viên của nút. Không có cách hỏi "người khác có quyền ở nút này không" mà không phụ thuộc `auth`.
8. **Tệp đính kèm**: `media::api::Media` thêm hàm `uploaders` (có cài đặt mặc định) để kiểm người mua chỉ đính tệp chính
   họ tải lên (D-202); khách vãng lai không đính tệp (không có tài khoản để tải lên).
9. **Mã gọi quốc gia mặc định `84`** khi số không có `+`; số nước khác ghi dạng `+<mã>…`.
10. **Chống spam**: form mở phiếu dùng `SpamGuard` phạm vi `support:tickets`, giới hạn `AUTOXIDE_SUPPORT_GUEST_PER_HOUR`
    theo IP băm, email và số điện thoại. Bài bị bỏ thì trả như thường (phiếu giả không lưu) để không dạy bot cách né.

Đã cân nhắc: dùng `OneTimeTokens` của kernel cho mã xác thực — token đó là chuỗi 43 ký tự, không gõ tay được từ SMS;
tách bảng `support_verifications` — thêm một bảng cho ba cột luôn đi theo phiếu.

Hệ quả: đặc tả bước 53 sửa theo mục 1–2; `docs/openapi.json` có thêm nhóm `/api/v1/support`.
