## D-20261122-late-apis · Chỗ nối muộn `StartContext::later` và tạo tài khoản từ token xem đơn (`POST /auth/phone-signups`) — 2026-11-22 — agent quyết

Chủ đề: Kiến trúc module và kernel

Bối cảnh: mục "Tạo tài khoản từ số điện thoại đã xác thực qua tra đơn vãng lai" của bước 45 (D-202, D-316; đặc tả 34
mục "Tạo tài khoản từ số đã xác thực"). `auth` đăng ký TRƯỚC `orders` nên `require` không thấy `OrderLookups`;
D-20261120-step45-guest-orders mục 9 ghi cần "chỗ nối muộn ở kernel". `auth` cũng không được import `orders`.

Quyết định:

1. **Kernel thêm `StartContext::later::<dyn T>() -> Later<T>`** (`autoxide_kernel::module::Later`): tay cầm tới trait
   do bất kỳ module nào công bố bằng `provide`, kể cả module đăng ký sau. Kernel điền mọi tay cầm một lần, ngay sau khi
   mọi module khởi động xong (trước khi phục vụ hay chạy job); `get()` trả `None` khi không module nào công bố (tắt,
   chưa cài) — bên giữ coi là "tính năng không có". Chỉ thêm, không đổi hợp đồng cũ (D-038). `require` vẫn là cách
   chính: `later` chỉ khi thứ tự đăng ký buộc phải ngược. Không ghi vào sổ phụ thuộc `required` (phụ thuộc tuỳ chọn).
2. **Hợp đồng trung lập ở kernel `autoxide_kernel::sms::VerifiedPhoneTokens`** (`verified_phone(tenant, token) ->
   Option<PhoneNumber>`): token do module khác cấp sau khi người dùng qua mã một lần. `orders` công bố (cùng bộ cài
   đặt với `orders::api::OrderLookups`, giữ nguyên cho module khác đang dùng); `auth` lấy qua `later`. Module khác
   sau này cấp token tương tự thì công bố thay — mỗi chương trình một bên.
3. **`POST /api/v1/auth/phone-signups {lookup_token, name?}`** (công khai): tạo tài khoản người mua (realm `customer`)
   chỉ có số, không email, không mật khẩu, rồi đăng nhập (201). Lần sau đăng nhập bằng mã SMS (`/auth/phone/login`)
   hay passkey thêm sau (D-20261120-phone-accounts mục 6). Số đã có tài khoản → 409 `auth.phone_taken` — token xem
   đơn không đủ để vào tài khoản sẵn có (giữ cùng mức như đăng nhập bằng mã); token hỏng / hết hạn → 422 ô
   `lookup_token`; khách không cho người mua tự tạo tài khoản → 403 `auth.customer_registration_closed`; không module
   nào cấp token → 503 `auth.phone_signup_unavailable`; quá 10 lần mỗi giờ theo dải IP (đếm TRƯỚC khi hỏi token để
   không dò được token) → 429 `auth.phone_signup_throttled`. Hai request cùng lúc cùng số: index unique quyết, bên
   thua nhận 409. Đặt tên trường `name` (như `/auth/phone/login`) thay cho `display_name` của đặc tả: một cách cho
   mỗi việc. Số được ghi là số đã xác thực ở `compliance` khi có bộ mã. Gắn đơn vẫn là `POST
   /api/v1/orders/me/orders/claim` (giao diện gọi tiếp) — `orders` không tạo tài khoản.

Căn cứ: NĐ 13/2023/NĐ-CP và Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15 (tối thiểu hoá — `auth` chỉ giữ HMAC của số) —
**cần đối chiếu văn bản gốc / người có chuyên môn pháp lý**.

Đã cân nhắc: `orders` gọi một điểm cắm của `auth` (như `compliance` làm với `AuthExtensions`) — module nghiệp vụ không
được phụ thuộc `auth` (D-046); đổi thứ tự đăng ký — `orders` cần thứ `auth` công bố trước; một ô riêng trong kernel
cho đúng trait này (như `DnsTxtLookup`) — mỗi lần cần lại thêm một ô, `later` dùng chung được.

Hệ quả: module ở repo riêng cũng dùng được `later`; người mua vãng lai tạo được tài khoản ngay trên trang xem đơn.
