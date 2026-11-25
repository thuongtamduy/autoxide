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
4. **Role mặc định của nhà cung cấp:** (a) mọi lần sửa nhà cung cấp còn `default_role_id` đều kiểm người sửa trao được
   role đó ở nút đó (`check_defaults`), không chỉ khi đổi trường mặc định — người sửa issuer, client, tự đăng ký hay tên
   miền quyết định ai nhận role. (b) Lúc tự đăng ký có role mặc định, kiểm lại người cấu hình như lời mời
   (`inviter_still_covers`): còn tồn tại, còn hoạt động, là nhân sự, còn `auth.oidc.manage` và hiện tại trao được role đó
   (`grant_problems`). Không đạt — kể cả `configured_by` NULL (đã bị bỏ khi khoá, hay dữ liệu trước m0046) — thì
   **từ chối đăng ký** (403 `auth.oidc_default_role_unavailable`), không tạo tài khoản không role: tài khoản nhân sự
   không role vẫn là một tài khoản mà đăng ký nhân sự công khai có thể đang đóng; quản trị viên lưu lại nhà cung cấp để
   nhận vai người cấu hình.
5. **Các đường khoá còn lại:** lấy lại tài khoản bằng mail và đổi mật khẩu kèm thu hồi mọi thứ (`PasswordWrite::Recovery`
   / `ChangeRevokingAll`) và xoá tài khoản cũng gọi `revoke_durable_grants_in` trong cùng transaction.

Rà mẫu "quyền hiện tại của người tạo / cấu hình X quyết định sức mạnh của X":

- `auth` — an toàn: lời mời (`inviter_still_covers`: role cố định lúc tạo, kiểm lại lúc nhận cả `auth.invitations.manage`
  — sức mạnh ≤ min(lúc tạo, lúc dùng)); API key (danh sách permission cố định, kiểm lúc tạo; tài khoản bị khoá thì key
  không dùng được, `lock_out_in` thu hồi); luật ánh xạ nhóm SCIM (`created_by` chỉ để ghi; đồng bộ dùng quyền người
  đang làm); uỷ quyền cho khách con (D-310) và lần cấp hỗ trợ (D-313): sức mạnh theo role được chỉ định, `granted_by`
  chỉ để ghi. Đã sửa: token SCIM, `configured_by` của nhà cung cấp OIDC.
- `automations` — **cần xem xét (ngoài module `auth`, chưa đổi):** luật tự động chạy bằng quyền hiện tại của `run_as`
  (`runner/mod.rs`), nên khi quyền của người đó thay đổi thì quyền của luật đổi theo. `auth` không được đụng bảng của
  `automations`; hướng xử lý (để module đó quyết): nghe event khoá tài khoản để tạm dừng luật, và / hay giới hạn quyền
  chạy bằng quyền lúc tạo. Mục `[ ]` dưới P16.
- `webhooks`: endpoint thuộc khách, gửi event theo permission của khách, không chạy bằng quyền người tạo — an toàn.

Lý do: rà soát độc lập #519 và #522 chỉ ra rằng vai người cấu hình nhà cung cấp đăng nhập (tự gộp, role mặc định) và
token SCIM giữ sức mạnh theo quyền sau này của tài khoản đã tạo chúng, kể cả khi tài khoản đó đã bị khoá hết cửa vào;
cùng loại với token SCIM của D-20261125-scim-sync-report.

Đã cân nhắc:
- Xoá nhà cung cấp do tài khoản cấu hình: làm hỏng đăng nhập của mọi người đã liên kết — chỉ bỏ người cấu hình là đủ chặn
  việc tự gộp.
- Lưu bản chụp quyền của người cấu hình lúc lưu nhà cung cấp: cần migration; kiểm "còn `auth.oidc.manage`" + thu hồi khi
  khoá đóng được kịch bản với ít thay đổi hơn.
