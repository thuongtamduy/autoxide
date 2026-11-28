## D-20261128-return-url-kernel · Kiểm `return_url` dùng chung ở kernel; xác minh danh tính dùng nó (L-11) — 2026-11-28 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: kernel có `tenancy::check_return_url(tenant, url, domains, listed)` và `invalid_return_origins(raw)` — cùng
luật của D-20261125-return-url-allowlist (origin đọc như trình duyệt; `https://` trên tên miền đã xác minh của khách, hay
origin trong danh sách khách khai). `compliance` kiểm `return_url` của `POST /api/v1/compliance/kyc/profiles/{id}/sessions`
(và `/me/kyc/sessions`) bằng hàm đó, với cấu hình theo khách mới
`compliance.kyc.return_origins`; sai thì 422, lỗi ô `return_url` mã `compliance.return_url_not_allowed` (hay `format`).
Chỉ kiểm địa chỉ do NGƯỜI GỌI API gửi (REST, MCP — `start_caller_session`). `KycGate::start` (module gọi module) tin địa
chỉ module truyền vào: trách nhiệm của module gọi, ghi trên doc của trait — vd `plans` truyền
`AUTOXIDE_PLANS_SIGNUP_RETURN_URL` của người vận hành, vốn không nằm trên tên miền hay cấu hình của khách nền tảng (kiểm
kiểu đó làm mọi lượt đăng ký cần KYC trả 422). Kiểu riêng `ReturnUrl` trong `compliance` buộc mọi đường gọi
`start_session` chọn rõ một trong hai.
Chuỗi gửi nhà cung cấp là chuỗi đã chuẩn hoá. `ledger` còn bản riêng (`ledger/src/return_url.rs`) — chuyển sang hàm
kernel là việc dọn, không đổi hành vi, làm khi phiên sửa `ledger`.

Lý do: L-11 của rà soát 2026-11 — nhà cung cấp chuyển trình duyệt người xác minh về `return_url`, nhận mọi địa chỉ là
open redirect như P-5 của thanh toán. Hai module cần cùng một luật thì luật ở kernel, không chép.

Thay đổi phá vỡ có chủ đích (sửa lỗ hổng, `docs/COMPATIBILITY.md` cho phép): client gửi `return_url` ngoài địa chỉ của
khách nhận 422 tới khi khách khai origin hay xác minh tên miền.

Đã cân nhắc: dùng chung cấu hình `ledger.payments.return_origins` (cấu hình thuộc module `ledger`, `compliance` không phụ
thuộc `ledger`); một cấu hình chung của kernel cho mọi module (cần chuyển dữ liệu cấu hình của `ledger`, ngoài phạm vi).
