## D-20261121-oidc-link-covers · OIDC tự liên kết theo email chỉ khi người cấu hình nhà cung cấp bao trùm quyền tài khoản — 2026-11-21 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: `auth_oidc_provider` có thêm cột `configured_by` (người tạo hay sửa gần nhất, migration
`m0046_oidc_configured_by`). Đăng nhập lần đầu bằng nhà cung cấp mà email trùng một tài khoản đã xác minh chỉ tự
liên kết khi tài khoản đó là người mua, không có permission nào, hoặc người cấu hình (còn hoạt động, là nhân sự) có
mọi permission của nó. Không thì 409 `auth.oidc_link_required`: chủ tài khoản đăng nhập như thường rồi tự liên kết
trong phần tài khoản. Dòng có từ trước (`configured_by` rỗng) chỉ tự liên kết tài khoản không có permission cho tới
khi một người đủ quyền lưu lại nhà cung cấp.

Lý do: ai cấu hình nhà cung cấp thì quyết nhà cung cấp đó "xác minh" email nào; người chỉ có `auth.oidc.manage` không
được nhờ đó đăng nhập vào tài khoản quyền cao hơn mình — cùng luật với D-079 và D-20261121-scim-claim-covers.

Đã cân nhắc: bỏ hẳn tự liên kết cho nhân sự — buộc mọi nhân viên có role tự liên kết bằng tay kể cả khi quản trị
viên toàn quyền cấu hình Entra của công ty; ghi `created_by` thay vì người sửa gần nhất — người sửa issuer hay
client id mới là người quyết tin ai.
