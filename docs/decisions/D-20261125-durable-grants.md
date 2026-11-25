## D-20261125-durable-grants · Uỷ quyền bền không sống lâu hơn quyền của người tạo: khoá tài khoản thu hồi token SCIM và vai người cấu hình nhà cung cấp đăng nhập — 2026-11-25 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định:

1. **Uỷ quyền bền** là thứ một tài khoản tạo ra mà sức mạnh đo bằng quyền HIỆN TẠI (hay sau này) của tài khoản đó:
   token SCIM nó tạo (D-20261125-scim-grant-cover) và vai "người cấu hình" (`configured_by`) của nhà cung cấp đăng
   nhập ngoài — quyết định nhà cung cấp có tự gộp danh tính ngoài vào tài khoản có sẵn không (D-20261121-oidc-link-covers).
   `user::revoke_durable_grants_in` thu hồi cả hai trong transaction của việc khoá: token SCIM bị xoá (event
   `auth.scim_token_revoked`), nhà cung cấp bị bỏ người cấu hình (`configured_by = NULL`, event
   `auth.oidc_provider_updated`; liên kết sẵn có vẫn đăng nhập được, chỉ ngừng tự gộp cho tới khi người quản lý lưu lại
   nhà cung cấp).
2. **Gọi ở mọi đường khoá:** `lock_out_in` (SCIM đổi email, "không phải tôi" của báo cáo email), nhận lại tài khoản chưa
   xác minh bằng link (`login_link`), khoá tài khoản qua SCIM (`active = false`) và qua REST (`disabled = true`) — mở
   khoá lại không làm chúng sống lại (như API key trên đường SCIM).
3. **Kiểm lúc dùng:** người tạo token SCIM phải còn `auth.scim.manage` (D-20261125-scim-sync-report), người cấu hình nhà
   cung cấp phải còn `auth.oidc.manage` (`configurer_covers`) — mất quyền quản lý thì thứ họ tạo không cấp / gộp được gì.

Rà mẫu "quyền hiện tại của người tạo / cấu hình X quyết định sức mạnh của X":

- `auth` — an toàn: lời mời (`inviter_still_covers`: role cố định lúc tạo, kiểm lại lúc nhận cả `auth.invitations.manage`
  — sức mạnh ≤ min(lúc tạo, lúc dùng)); API key (danh sách permission cố định, kiểm lúc tạo; tài khoản bị khoá thì key
  không dùng được, `lock_out_in` thu hồi); luật ánh xạ nhóm SCIM (`created_by` chỉ để ghi; đồng bộ dùng quyền người
  đang làm); uỷ quyền cho khách con (D-310) và lần cấp hỗ trợ (D-313): sức mạnh theo role được chỉ định, `granted_by`
  chỉ để ghi. Đã sửa: token SCIM, `configured_by` của nhà cung cấp OIDC.
- `automations` — **cần xem xét (ngoài module `auth`, chưa sửa):** luật tự động chạy bằng quyền HIỆN TẠI của `run_as`
  (`runner/mod.rs`); mất quyền thì luật tạm dừng, nhưng **được thêm quyền** (vd tài khoản cắm sẵn sau đó được trao role
  cao) thì luật chạy với quyền mới — tool đã chọn lúc tạo nhưng đối số / phạm vi của nó rộng theo quyền mới. `auth`
  không được đụng bảng của `automations`; cách sửa: `automations` nghe event khoá tài khoản để tạm dừng luật `run_as`
  người đó, và / hay giới hạn quyền chạy bằng quyền lúc tạo. Mục `[ ]` dưới P16.
- `webhooks`: endpoint thuộc khách, gửi event theo permission của khách, không chạy bằng quyền người tạo — an toàn.

Lý do: rà soát độc lập #519 chỉ ra một tài khoản cắm sẵn có thể tạm có `auth.oidc.manage`, dựng nhà cung cấp với issuer
do kẻ tấn công kiểm soát, rồi sau khi bị khoá hết cửa vào và được trao toàn quyền thì nhà cung cấp đó tự gộp danh tính
ngoài vào tài khoản admin (MFA của admin, nếu có, vẫn chặn). Cùng loại với token SCIM của D-20261125-scim-sync-report.

Đã cân nhắc:
- Xoá nhà cung cấp do tài khoản cấu hình: làm hỏng đăng nhập của mọi người đã liên kết — chỉ bỏ người cấu hình là đủ chặn
  việc tự gộp.
- Lưu bản chụp quyền của người cấu hình lúc lưu nhà cung cấp: cần migration; kiểm "còn `auth.oidc.manage`" + thu hồi khi
  khoá đóng được kịch bản với ít thay đổi hơn.
