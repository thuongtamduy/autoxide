## D-20261120-phone-accounts · Người mua đăng nhập / tạo tài khoản bằng số điện thoại + mã SMS; tài khoản không bắt buộc email (bước 47, việc tồn đọng sau v0.15.0) — 2026-11-20 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Bối cảnh: ROADMAP bước 47 ("đăng nhập / tạo tài khoản công khai cho người mua bằng OTP qua SMS hay passkey") và việc tồn
đọng "đăng nhập bằng số điện thoại + OTP (tài khoản không bắt buộc email — thay đổi phá vỡ của `auth`, cần quyết định)".
Hiện trạng: `auth_user.email` NOT NULL, unique `(tenant_id, realm, email)`; `api::User.email: String` được `plans`,
`orders`, `invoices`, `compliance`, `notifications` đọc; bốn response (`MeResponse`, `UserResponse`, `CustomerResponse`…)
khai `email` là chuỗi bắt buộc. `auth` đăng ký **trước** `compliance` (giữ OTP, quy tắc số, bên gửi SMS) nên không
`require` được nó.

Quyết định:

1. **Không phá vỡ hợp đồng:** tài khoản chỉ có số giữ `email = ""` (chuỗi rỗng), không đổi `email` thành nullable.
   Cột vẫn NOT NULL (đổi NOT NULL → NULL trên SQLite phải dựng lại bảng có khoá ngoài); unique email thành **index một
   phần** `WHERE email <> ''` (SQLite và Postgres đều có). Client và module khác coi `""` là "chưa có email". Đã cân
   nhắc `email: null` — chuẩn hơn về kiểu nhưng đổi kiểu ở bốn response và `api::User` (thay đổi phá vỡ, phải làm endpoint
   mới theo D-059) cho một trường hợp mà `""` diễn đạt đủ.
2. **`auth_user.phone_key`** (HMAC của số theo khách, `SecretKey::phone_key` — database không giữ số rõ), unique
   `(tenant_id, realm, phone_key)`; NULL không đụng nhau. Một số → một tài khoản người mua mỗi khách.
3. **Mã SMS do `compliance` làm, cắm vào `auth`:** `auth::api` công bố `PhoneCodes` (gửi, kiểm, gắn số vào tài khoản)
   và điểm cắm `AuthExtensions::set_phone_codes` (cùng cách `orders` nhận tender của `customers`). `compliance` dùng lại
   bảng mã, cách sinh / băm mã có pepper, quy tắc số được nhận (`posting.phone_countries`), giới hạn theo số và theo IP,
   bên gửi SMS của xác thực số; mã đăng nhập có `user_id` = UUID rỗng (chưa có tài khoản). Không có bên gửi SMS → 503.
4. **Endpoint công khai:** `POST /api/v1/auth/phone/codes {phone}` (202, trả lời như nhau dù số có tài khoản chưa — không
   dò được) và `POST /api/v1/auth/phone/login {challenge_id, code, name?}`: số đã có tài khoản người mua → đăng nhập (200,
   vẫn hỏi TOTP nếu bật); chưa có và khách bật `auth.customer_registration` → tạo tài khoản không email, không mật khẩu
   (201). Hai lần tạo cùng lúc cùng số: index unique quyết, bên thua dùng tài khoản bên thắng (test). Chỉ realm
   `customer`; nhân sự vẫn cần email.
5. **Số vừa đăng nhập thành số đã xác thực** của tài khoản (`compliance_account_verifications`, event
   `compliance.phone_verified`) để `customers` gắn hồ sơ theo số và cổng đăng nội dung thấy — trừ khi quy tắc
   `max_accounts_per_phone` của khách không cho (thì bỏ qua, đăng nhập vẫn được).
6. **Passkey cho tài khoản chỉ có số:** xác nhận lại (D-308) thêm cách `phone_code` (mã gửi tới chính số đăng nhập của
   tài khoản; số khác → lỗi ô `phone_code`); `email_link` với tài khoản không email → 422. Có token xác nhận lại thì thêm
   passkey như mọi tài khoản (tên của passkey là tên hiển thị khi không có email), rồi đăng nhập bằng passkey.
7. Chỗ dùng email để gửi thư bỏ qua `""`: `notifications` không coi là địa chỉ email; gửi lại thư xác minh email cho tài
   khoản chưa có email → 422.

Chưa làm (mục riêng của ROADMAP bước 45): `POST /api/v1/auth/phone-signups` từ token xem đơn vãng lai của `orders`.

Căn cứ: NĐ 13/2023/NĐ-CP và Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15 (số điện thoại là dữ liệu cá nhân — chỉ lưu HMAC ở
`auth`, số rõ ở `compliance` như trước) — **cần đối chiếu văn bản gốc / người có chuyên môn pháp lý**.

Hệ quả: website bán hàng cho khách đăng nhập bằng số như thói quen ở Việt Nam mà không phá client đang dùng `email`;
client nên hiện "chưa có email" khi `email == ""`.
