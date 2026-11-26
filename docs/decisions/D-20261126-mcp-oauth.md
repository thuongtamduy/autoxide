## D-20261126-mcp-oauth · Đăng nhập chuẩn cho MCP: máy chủ uỷ quyền OAuth 2.1 trong module `auth`, kernel làm resource server (bước 71 G4) — 2026-11-26 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định:

Theo đặc tả MCP authorization bản 2026-07-28 (cùng các trang authorization-server-discovery, client-registration,
security-considerations), OAuth 2.1 (draft-ietf-oauth-v2-1-16), RFC 9728, RFC 8414, RFC 8707, RFC 9207, RFC 9068,
RFC 7009, RFC 7591 và Client ID Metadata Documents (draft-ietf-oauth-client-id-metadata-document-03):

1. **Hai vai, hai chỗ.** Kernel là resource server: phục vụ Protected Resource Metadata (RFC 9728), thêm
   `resource_metadata` vào `WWW-Authenticate`, kiểm audience của token. Module `auth` là authorization server và công bố
   trait `autoxide_kernel::access::AuthorizationServer` (metadata RFC 8414) cho kernel phục vụ. Không module nào cung
   cấp trait thì không có `/.well-known` và header giữ như cũ.
2. **Issuer và resource.** Chỉ bật khi có `AUTOXIDE_HTTP_PUBLIC_URL`. Issuer = public URL; request tới tên miền riêng đã
   xác minh của một khách thì issuer = `https://<tên miền>` (đăng nhập ràng vào khách đó). Host lạ dùng public URL — không
   phản chiếu `Host` tuỳ ý. Resource canonical: `{issuer}/mcp`, `{issuer}/mcp/<module>`, `{issuer}/api/v1`, và `{issuer}`
   (bao tất cả). PRM ở `/.well-known/oauth-protected-resource` + đường dẫn của resource. Phân cấp audience: token cho
   resource rộng dùng được ở resource hẹp nằm dưới nó, không ngược lại. Token phiên đăng nhập và API key (không có
   audience) giữ nguyên hành vi.
3. **Đăng ký client:** Client ID Metadata Documents là đường chính (tải bằng HTTP client chống SSRF, không theo
   redirect, tối đa 5 KB, cấm secret, cache có kẹp thời gian); client đăng ký trước do quản trị khách tạo; Dynamic Client
   Registration (RFC 7591) giữ cho client hiện tại nhưng khai deprecated như MCP 2026-07-28, tắt được, chỉ client
   public.
4. **Luồng:** chỉ `authorization_code` với PKCE `S256` bắt buộc, phản hồi có `iss` (RFC 9207), refresh token xoay vòng
   (dùng lại thì thu hồi cả grant), revoke RFC 7009. Không implicit, không password grant; PAR, DPoP, device flow xem lại
   sau 1.0.
5. **Consent theo luật kiến trúc 9 (core chỉ có API):** `/authorize` kiểm tham số rồi chuyển trình duyệt tới trang của
   giao diện `AUTOXIDE_AUTH_OAUTH_CONSENT_URL` (mô hình login/consent app); trang đó gọi API của `auth` để xem yêu cầu và
   duyệt / từ chối. Chỉ người (phiên đăng nhập, không phải API key hay agent) được duyệt. Thiếu biến thì OAuth tắt,
   `doctor` báo; người chưa có giao diện kết nối client MCP bằng API key như trước.
6. **Token:** JWT RFC 9068 (`typ: at+jwt`) HS256 bằng khoá theo khách sẵn có, claim `aud`, `client_id`, `scope`, `gid`
   (grant). Mỗi request đọc grant nên thu hồi có hiệu lực ngay. Quyền = quyền hiện tại của người dùng ∩ scope (như API
   key); loại người gọi `agent`; `Principal` mang thông tin grant (client, audience, permission bị scope giữ lại).
7. **Scope = tên permission**, thêm phân cấp `<module>.*` và `*`. PRM không khai `scopes_supported` (RFC 9728 chỉ
   khuyến nghị; khai hàng trăm permission thì client xin hết) — người duyệt chọn quyền ở trang consent. Người dùng có
   permission mà grant thiếu: 403 + `WWW-Authenticate: Bearer error="insufficient_scope", scope="…",
   resource_metadata="…"` (lý do `outside_grant_scope`), cả REST lẫn `tools/call` của MCP.
8. **Ngoại lệ luật repo:** endpoint giao thức OAuth (`/token`, `/revoke`, `/register`) và `/.well-known/*` dùng đúng wire
   format của RFC (form-urlencoded, JSON trần, lỗi `{error, error_description}` RFC 6749) thay vì `ValidJson`, envelope
   `Data` và `Problem`; vẫn khai đủ trong OpenAPI. `/token` không nhận `Idempotency-Key` (mã và refresh token vốn dùng
   một lần).

Lý do: client MCP (Claude, ChatGPT, IDE) chỉ kết nối được không cần dán khoá khi server theo đúng đặc tả uỷ quyền
MCP; đặc tả 2026-07-28 chọn CIMD và hạ DCR xuống deprecated, nên làm CIMD làm đường chính để không phải đập đi. Đặt
authorization server trong `auth` vì nó cần tài khoản, phiên, step-up, khoá ký sẵn có; kernel chỉ giữ phần resource
server vì mọi module đều được bảo vệ chung. Giữ luật 9 (chủ dự án qua phiên điều phối, 2026-11-26): trang consent thuộc
repo giao diện.

Đã cân nhắc: dùng máy chủ uỷ quyền ngoài (Keycloak, Ory Hydra) — người cài phải dựng thêm dịch vụ, trái mục tiêu tự cài
dùng được; trang HTML consent tối giản trong core — trái luật 9, chủ dự án không mở ngoại lệ; khai mọi permission trong
`scopes_supported` — client xin toàn quyền theo mặc định, trái nguyên tắc ít quyền nhất; chỉ DCR — đặc tả đã deprecated;
access token không trạng thái hoàn toàn — không thu hồi được trước khi hết hạn, không chấp nhận được với agent.
