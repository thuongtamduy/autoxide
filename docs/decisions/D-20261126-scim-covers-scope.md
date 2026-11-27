## D-20261126-scim-covers-scope · Token SCIM sửa tài khoản đã liên kết: người tạo phải "giữ được" từng membership của tài khoản — 2026-11-26 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: mọi chỗ token SCIM tác động lên một tài khoản qua `require_creator_covers` (sửa hồ sơ liên kết — `userName`,
`externalId`, tên, email, `active` —, khoá, xoá, nhận về tài khoản có sẵn) ngoài `RoleService::require_covers` (so theo
TÊN permission) còn đòi `require_grantable` với mọi cặp (nút, role) tài khoản đang giữ: người tạo token, tính lúc dùng
token, phải cấp được từng role ở đúng nút đó — cùng luật với nhóm SCIM và luật ánh xạ (D-20261125-scim-grant-cover).

Lý do: PT-19 (đợt test 2026-11) — `require_covers` bỏ qua phạm vi, nên người tạo token có một permission ở nhánh A vẫn
"bao trùm" tài khoản giữ cùng permission ở nhánh B và đổi được khoá danh tính của tài khoản đó.

Đã cân nhắc: sửa `require_covers` cho mọi đường (khoá, gỡ MFA, đặt mật khẩu đầu… qua REST) — đó là mục (i) của nợ
P16, đổi hành vi của nhiều endpoint REST và cần rà riêng; phần SCIM sửa trước vì token chạy không người giám sát.
