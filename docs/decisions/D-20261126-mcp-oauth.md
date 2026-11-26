## D-20261126-mcp-oauth · Đăng nhập chuẩn cho MCP: máy chủ uỷ quyền OAuth 2.1 trong module `auth`, kernel làm resource server (bước 71 G4) — 2026-11-26 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định:

Theo đặc tả MCP authorization bản 2026-07-28 (cùng các trang authorization-server-discovery, client-registration,
security-considerations), OAuth 2.1 (draft-ietf-oauth-v2-1-16), RFC 9728, RFC 8414, RFC 8707, RFC 9207, RFC 7009,
RFC 7591 và Client ID Metadata Documents (draft-ietf-oauth-client-id-metadata-document-03). Thiết kế đã qua một lượt
kiểm độc lập (bảo mật và đối chiếu đặc tả) trước khi viết code; các điểm sửa đã gộp vào dưới đây.

1. **Hai vai, hai chỗ.** Kernel là resource server: phục vụ Protected Resource Metadata (RFC 9728), thêm
   `resource_metadata` vào `WWW-Authenticate`, kiểm audience của token. Module `auth` là authorization server và công bố
   trait `autoxide_kernel::access::AuthorizationServer` (metadata RFC 8414) cho kernel phục vụ. Không module nào cung
   cấp trait thì `/.well-known/*` trả `404 oauth_unavailable` và header giữ như cũ.
2. **Phân loại credential.** `Principal::credential()` = phiên đăng nhập | API key | grant OAuth. Mọi việc chỉ dành cho
   phiên của chính người dùng (tạo / duyệt credential, duyệt grant, xác nhận đã xem nội dung AI, vào khách khác qua uỷ
   quyền hay hỗ trợ) kiểm `Principal::is_session()`, không bao giờ kiểm "không có API key". Hạn mức, khoá chống gọi
   trùng và lệnh chờ duyệt gắn với `credential_id()` (API key hay grant).
3. **Một issuer, resource chặt.** Bản đầu có một issuer cho cả nền tảng: `AUTOXIDE_HTTP_PUBLIC_URL` (dạng chuẩn, không
   path; có path thì OAuth tắt và log cảnh báo). Khách của grant là khách của phiên đăng nhập lúc consent. Resource
   canonical (chữ thường scheme/host, không `/` cuối): `{issuer}/mcp`, `{issuer}/mcp/<module>`, `{issuer}/api/v1`; PRM ở
   `/.well-known/oauth-protected-resource` + đúng đường dẫn đó (không có tài liệu gốc). Thiếu `resource` → `/mcp`. Token
   cho `/mcp` dùng được ở mọi `/mcp/<module>`; `/api/v1` tách riêng; không có token "cả máy chủ". Mọi điểm vào nhận
   bearer đi qua cùng một chỗ kiểm: đường dẫn không thuộc resource nào thì không nhận token OAuth (fail-closed).
   `X-Forwarded-Host` chỉ tin từ proxy tin cậy (sẵn có).
4. **Đăng ký client:** Client ID Metadata Documents là đường chính (tải bằng HTTP client chống SSRF của kernel —
   `net::PublicOnly`, không proxy, không theo redirect, tối đa 5 KB; hạn mức theo IP và host, giới hạn tải đồng thời,
   cache lỗi ngắn; chỉ nhận `token_endpoint_auth_method` `none` hay không khai; không tải `logo_uri`; `client_name` là
   dữ liệu chưa tin). Client đăng ký trước do quản trị khách tạo (chỉ public client ở bản đầu). Dynamic Client
   Registration (RFC 7591) giữ cho client hiện tại nhưng khai deprecated như MCP 2026-07-28, tắt được, chỉ client
   public; dọn client DCR chưa từng có grant sau 30 ngày.
5. **Luồng:** chỉ `authorization_code` với PKCE `S256` bắt buộc. `/authorize` lỗi trước khi xác nhận client và
   `redirect_uri` → 400, không chuyển hướng; sau đó → chuyển hướng kèm `error`, `state`, `iss`. Mọi phản hồi uỷ quyền
   (cả lỗi) mang `iss` (RFC 9207) mới khai `authorization_response_iss_parameter_supported`; issuer giống hệt nhau ở PRM,
   metadata và `iss`. Code dùng lại → thu hồi token sinh từ code. Refresh token xoay vòng, luôn cấp cho public client;
   dùng lại token cũ → thu hồi cả grant, trừ ân hạn ngắn cho đúng token liền trước khi client gửi lại (trả
   `invalid_grant`). Revoke RFC 7009. Không implicit, không password grant; PAR, DPoP, device flow, client
   confidential, issuer theo tên miền khách xem lại sau.
6. **Consent theo luật kiến trúc 9 (core chỉ có API):** `/authorize` kiểm tham số rồi chuyển trình duyệt tới trang của
   giao diện `AUTOXIDE_AUTH_OAUTH_CONSENT_URL`; trang đó gọi API của `auth` để xem yêu cầu (host của `client_id` và của
   `redirect_uri`, cảnh báo redirect chỉ localhost) và duyệt / từ chối. Chỉ phiên đăng nhập (`is_session`) được duyệt và
   quản lý grant / client. Thiếu biến thì OAuth tắt, `doctor` báo; người chưa có giao diện kết nối client MCP bằng API
   key như trước.
7. **Token opaque**, không JWT: `axo1_<khách>_<ngẫu nhiên>` lưu băm như API key, phân luồng theo tiền tố trong bộ xác
   thực (không thể lẫn với access token của phiên). Mỗi request đọc grant nên thu hồi có hiệu lực ngay. Quyền = permission
   đã lưu của grant ∩ quyền hiện tại của người dùng (người dùng không còn → từ chối), qua `guardrails.apply`; principal
   OAuth mang loại người gọi `agent` và thông tin grant (client, audience) và không vào được khách khác (`enter` từ chối).
8. **Scope = tên permission.** Mẫu `<module>.*` chỉ dùng lúc xin; consent bung thành danh sách permission cụ thể và
   lưu danh sách đó. Permission rủi ro cao (xoá, tiền) không bao giờ tới từ mẫu: phải được chọn từng cái và consent đòi
   xác nhận lại (step-up). Không khai `*`. PRM không khai `scopes_supported` (người duyệt chọn quyền ở trang consent).
   Người dùng có permission mà grant thiếu: 403 + `WWW-Authenticate: Bearer error="insufficient_scope", scope="…",
   resource_metadata="…"` (lý do `outside_grant_scope`) cho REST và `tools/call`; với MCP challenge còn nằm trong
   `_meta["mcp/www_authenticate"]` của kết quả. `tools/list` chỉ liệt kê tool grant dùng được.
9. **Vòng đời:** thu hồi grant khi người dùng bị khoá, rời khách, khách bị xoá, đổi mật khẩu hay "đăng xuất khắp nơi".
10. **Ngoại lệ luật repo:** endpoint giao thức OAuth (`/token`, `/revoke`, `/register`) và `/.well-known/*` dùng đúng
    wire format của RFC (form-urlencoded, JSON trần, lỗi `{error, error_description}` RFC 6749) thay vì `ValidJson`,
    envelope `Data` và `Problem`; vẫn khai đủ trong OpenAPI. `/token` không nhận `Idempotency-Key` (mã và refresh token
    vốn dùng một lần). Tài liệu khám phá cho mọi origin đọc (`Access-Control-Allow-Origin: *`, không credentials).

Lý do: client MCP (Claude, ChatGPT, IDE) chỉ kết nối được không cần dán khoá khi server theo đúng đặc tả uỷ quyền
MCP; đặc tả 2026-07-28 chọn CIMD và hạ DCR xuống deprecated, nên làm CIMD làm đường chính để không phải đập đi. Đặt
authorization server trong `auth` vì nó cần tài khoản, phiên, step-up sẵn có; kernel chỉ giữ phần resource server vì mọi
module đều được bảo vệ chung. Token opaque vì bộ kiểm JWT sẵn có không kiểm loại token — nhầm loại là rủi ro thật, và
mỗi request vẫn phải đọc grant để thu hồi tức thì. Bung mẫu scope lúc consent để permission mới thêm sau không tự lọt vào
grant cũ. Giữ luật 9 (chủ dự án qua phiên điều phối, 2026-11-26): trang consent thuộc repo giao diện.

Đã cân nhắc: dùng máy chủ uỷ quyền ngoài (Keycloak, Ory Hydra) — người cài phải dựng thêm dịch vụ, trái mục tiêu tự cài
dùng được; trang HTML consent tối giản trong core — trái luật 9, chủ dự án không mở ngoại lệ; access token JWT — không
thu hồi tức thì, dễ lẫn với token phiên; issuer riêng theo tên miền khách ngay bản đầu — thêm bề mặt (kiểm issuer theo
Host) chưa cần; khai mọi permission trong `scopes_supported` — client xin toàn quyền theo mặc định; chỉ DCR — đặc tả đã
deprecated.
