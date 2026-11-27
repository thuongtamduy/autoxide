# Mô hình mối đe doạ của core

*English: [THREAT_MODEL.md](THREAT_MODEL.md)*

> **Đây là gì:** mô hình của nhóm duy trì về những thứ kẻ tấn công có thể nhắm tới trong một máy chủ Autoxide, những
> chốt chặn (control) đang có và những gì còn mở. Tài liệu được viết với AI hỗ trợ, dựa trên code và nhật ký quyết
> định, làm đầu vào cho một đợt kiểm thử xâm nhập (penetration test) do người ngoài dự án thực hiện. Đây **không**
> phải kiểm toán và không khẳng định Autoxide không có lỗ hổng. Muốn báo lỗi, làm theo
> [`SECURITY.vi.md`](../SECURITY.vi.md).

- **Ngày:** 2026-11-26, `main` của loạt `0.x` trước 1.0
- **Mục lộ trình:** bước 71, G13 ([`docs/ROADMAP.md`](ROADMAP.md))
- **Dựa trên:** [đợt rà soát tháng 11/2026](security/review-2026-11.md) (rà code và test hộp đen) và
  [đợt test bảo mật và hiệu năng tháng 11/2026](security/pentest-2026-11/README.vi.md) (29.566 case trên cả hai
  database)
- **Cách làm:** [STRIDE](https://learn.microsoft.com/en-us/azure/security/develop/threat-modeling-tool-threats) theo
  từng ranh giới tin cậy (trust boundary) — **S**poofing (giả danh), **T**ampering (sửa lén), **R**epudiation (chối
  bỏ), **I**nformation disclosure (lộ thông tin), **D**enial of service (làm nghẽn), **E**levation of privilege
  (leo thang quyền)

## Cách đọc tài liệu này

Mỗi ranh giới trong tám ranh giới dưới đây có cùng các phần:

- **Tài sản** — thứ kẻ tấn công muốn có ở ranh giới đó.
- **Điểm vào** — nơi dữ liệu đi qua ranh giới.
- **Giả định tin cậy** — điều thiết kế coi là đúng. Một bài test phá được giả định là một phát hiện.
- **Bảng STRIDE** — mối đe doạ → control đang có → rủi ro còn lại hay việc còn mở. Control ghi nơi nó nằm: đường dẫn
  code (tính từ gốc repo), mã quyết định (`D-…` trong [`docs/DECISIONS.md`](DECISIONS.md) hay
  `docs/decisions/D-….md`) hoặc một luật gate của `cargo xtask check`. Control ghi **(cần kiểm chứng)** là control do
  thiết kế hay tài liệu mô tả, nhưng chưa được lần theo trọn trong code khi viết tài liệu này; danh sách ở cuối, mục
  [Control cần kiểm chứng](#control-cần-kiểm-chứng).

Phát hiện của các đợt trước giữ mã cũ: `H-`, `M-`, `L-`, `I-`, `P-` (rà soát) và `PT-` (đợt test).

## Tổng quan hệ thống

```text
           trình duyệt, ứng dụng, AI agent, máy chủ đối tác, callback thanh toán / vận chuyển / ngân hàng
                                         │  HTTPS (TLS kết thúc ở reverse proxy)
  ┌──────────────────────────────────────▼───────────────────────────────────────────────┐
  │ (1) Lớp HTTP: request id → security headers → log → giới hạn → CORS → khách → auth   │
  │     REST /api/v1/…  ·  OpenAPI  ·  SSE / WebSocket  ·  route công khai  ·  webhook vào│
  │ (2) MCP endpoint /mcp, /mcp/<module>                                                 │
  ├──────────────────────────────────────────────────────────────────────────────────────┤
  │ (3) kernel: Principal và permission, đa khách, throttle, idempotency, outbox, job,   │
  │     cấu hình và secret, seal, lọc địa chỉ gọi ra, trait lưu file và AI               │
  ├──────────────────────────────────────────────────────────────────────────────────────┤
  │ (4) module (mỗi module một crate, bảng riêng, nói chuyện qua trait và event)         │
  ├──────────────────────────────────────────────────────────────────────────────────────┤
  │ (5) database: database điều khiển + placement (SQLite hay PostgreSQL), kho file      │
  └───────────────┬───────────────────────────────────────────────────┬──────────────────┘
                  │ (6) placement: khách ở database chung hay riêng   │
                  ▼                                                   ▼
  (7) nhà cung cấp bên ngoài: thanh toán, biến động số dư,     (8) nhà cung cấp AI do người cài
      vận chuyển, SMS, hoá đơn điện tử, mail, webhook gửi đi,      chọn (TextModel / Embedder)
      web push
```

**Chế độ.** `AUTOXIDE_TENANCY_MODE=single` phục vụ một khách mặc định, bỏ qua `Host` và header chọn khách. `multi`
phục vụ nhiều khách trong một tiến trình; đây là chế độ có nhiều ranh giới nhất và là chế độ nên test.

### Tác nhân

| Tác nhân | Credential | Được tin cho việc gì |
| -------- | ---------- | -------------------- |
| Khách vãng lai | không có | route công khai của một khách (cửa hàng, nội dung đã đăng, form, bình luận, tìm kiếm công khai) |
| Người mua | access token của tài khoản người mua | bản ghi của chính họ; không bao giờ là dữ liệu của nhân viên |
| Nhân viên của khách | access token, có thể kèm TOTP / passkey; permission từ role, có thể giới hạn theo một nút tổ chức | những gì permission cho phép, trong khách và nhánh của họ |
| Quản trị viên của khách | như nhân viên, giữ permission ngang admin | mọi thứ trong khách của họ; **không** phải khách khác (xem [`SECURITY.vi.md`](../SECURITY.vi.md), mục "không phải lỗ hổng") |
| Máy / AI agent | API key `axk1_…` (phạm vi ∩ quyền hiện tại của chủ key, có thể rào IP) | phạm vi của key; không bao giờ hơn chủ key |
| Client SCIM | token SCIM gắn với quyền của người tạo | cấp tài khoản trong tầm của người tạo |
| Nhân sự nền tảng | tài khoản của khách nền tảng có permission `kernel.tenants.*` | vận hành khách; vào một khách chỉ qua lần cấp hỗ trợ có thời hạn |
| Khách cha | uỷ quyền (delegation) | làm việc ở khách con, tại một nút, với permission của một role |
| Nhà cung cấp | URL webhook riêng của khách + chữ ký của nhà cung cấp | báo thanh toán, tiền vào tài khoản, trạng thái vận đơn |
| Người vận hành | shell trên máy chủ, biến môi trường, database | mọi thứ (ngoài phạm vi) |
| Người viết module ngoài | code biên dịch vào binary | mọi thứ tiến trình làm được (xem ranh giới 4) |

### Giả định chung

- TLS kết thúc ở reverse proxy hay load balancer phía trước; bản thân máy chủ nói HTTP
  ([`docs/DEPLOY.vi.md`](DEPLOY.vi.md)). `AUTOXIDE_HTTP_CLIENT_IP_HEADER` / `AUTOXIDE_HTTP_TRUSTED_PROXIES` chỉ được
  đặt khi có proxy như vậy (`crates/kernel/src/config/http.rs`).
- `AUTOXIDE_SECRET_KEY` và `AUTOXIDE_AUTH_TOKEN_SECRET` được đặt giá trị ngẫu nhiên ở production; giá trị mẫu của
  `.env.example` bị từ chối lúc khởi động (H-3; `crates/kernel/src/security/secret.rs`, `modules/auth/src/config.rs`).
- Chỉ máy chủ và người vận hành tới được database và kho file.
- Code là Rust an toàn bộ nhớ: `unsafe_code = "forbid"` cho mọi crate của workspace (`Cargo.toml`); bản release giữ
  kiểm tràn số (tràn là 500, không quay vòng — PT-5).

---

## 1. Client ↔ lớp HTTP

**Tài sản:** API còn phục vụ được; ánh xạ request → khách; credential trên đường truyền; dữ liệu của khách trong
response; phiên trình duyệt trên website của khách.

**Điểm vào:** mọi route REST dưới `/api/v1/` (`docs/openapi.json`, khoảng 1.300 operation); route công khai
(`security((), ("bearer" = []))`); `/api/v1/openapi.json`, `/api/docs` (phải bật), `/llms.txt`; `/health/*`;
`/metrics` (phải bật); `GET /api/v1/kernel/events/stream` (SSE) và `/events/ws` (WebSocket); webhook từ nhà cung
cấp (ranh giới 7); các header `Host`, `Origin`, `Autoxide-Tenant`, `Authorization`, `Idempotency-Key`, `If-Match`,
header địa chỉ client.

**Giả định tin cậy:**

- Khách của một request có xác thực lấy **từ credential**, không bao giờ chỉ từ `Host` hay header
  (`crates/kernel/src/data/tenancy/request.rs`).
- Một `Host` chỉ trỏ tới một khách khi khách đó đã **xác minh** tên miền bằng bản ghi DNS TXT
  (`crates/kernel/src/data/tenancy/domains.rs`).
- Route công khai chỉ trả dữ liệu khách đã công bố; chọn khách khác cho một request công khai (bằng host, header hay
  `?site=`) là được phép và chỉ lộ dữ liệu công khai của khách đó.

| STRIDE | Mối đe doạ | Control đang có | Còn lại / còn mở |
| ------ | ---------- | --------------- | ---------------- |
| S | Giả làm khách khác bằng cách gửi `Host` hay `Autoxide-Tenant` của họ kèm credential của mình | Khách của credential được so với header và tên miền đã xác minh → `tenant_mismatch`; chỉ nhân sự nền tảng có `kernel.tenants.operate`, uỷ quyền hay lần cấp hỗ trợ mới vào được khách khác (`data/tenancy/request.rs`, `modules/auth/src/delegation.rs`, `modules/auth/src/support.rs`) | Đã test trên cả hai database ở P14 và đợt test 2026-11 (nhóm IDOR, TOKEN) |
| S | Nhận một tên miền không phải của mình | DNS TXT `_autoxide-challenge.<tên miền>`; kiểm lại hằng ngày, 3 lần không thấy → `failed`; mỗi tên miền tối đa một dòng `verified` (partial unique index, P-4, D-20261125-domain-claims) | Test chắc chắn đi qua nhánh "thua cuộc đua" còn mở (P16) |
| S | Giả địa chỉ client để né giới hạn theo địa chỉ | Địa chỉ chỉ đọc từ header đã cấu hình, đếm từ các proxy tin cậy (`http/client.rs`); khoá là một địa chỉ IPv4 hay một khối IPv6 /64 (`throttle::address_key`, PT-4, D-20261125-address-keys, gate `address-key`) | Gate chỉ bắt một mẫu viết (P17) |
| T | Request chéo trang từ trình duyệt | CORS: danh sách `AUTOXIDE_HTTP_CORS_ORIGINS`, hoặc `https://<tên miền đã xác minh>` chỉ cho đúng khách đó (`http/cors.rs`, D-20261125-tenant-cors); credential là header bearer, không phải cookie | `*` là cấu hình sai đã ghi trong tài liệu và có cảnh báo trong log (L-5) |
| T | Lệnh ghi bị gửi lại hay chạy hai lần (retry, agent) | `Idempotency-Key` lưu theo người gọi và request; thân khác → 422; đang chạy → 409 (`security/idempotency.rs`, `http/idempotency.rs`, D-058); kiểm đồng thời lạc quan bằng `If-Match` | — |
| T | Chèn mã qua tham số query, cursor, khoá sắp xếp, thân request | SeaORM với giá trị bind; câu SQL viết tay chỉ qua `database::statement` (D-20261121-raw-statement); cursor mang giá trị, cột do máy chủ chọn; `ValidJson<T>` + kiểm ở service (D-083); ký tự NUL trong URL bị từ chối (`http/mod.rs`) | 8.061 case INPUT mỗi database đều qua (đợt test 2026-11) |
| R | Người gọi chối đã gửi request | Request id trên mọi response; nhật ký request theo khách có người dùng, API key, loại tác nhân, mẫu route, status, `Idempotency-Key` (`ops/request_log.rs`, D-202); chuỗi audit của mọi thay đổi (ranh giới 4) | Request công khai (không xác thực) cố ý không vào nhật ký request |
| I | Thân lỗi, header hay log lộ chi tiết bên trong hay secret | `Problem` theo RFC 9457 với `code` ổn định, không có stack hay câu SQL; log ghi mẫu route, không bao giờ ghi đường dẫn thô, query hay thân (`http/log.rs`); security headers trên mọi response, kể cả CSP `default-src 'none'; sandbox` (`http/security_headers.rs`, D-320) | `/metrics` trên port công khai là cấu hình sai đã ghi trong tài liệu (có cảnh báo lúc khởi động) |
| I | Biết bản ghi hay trường nào tồn tại khi không có quyền | Lớp kiểm permission chung đọc `x-permissions` và trả 403 trước khi đọc thân, `If-Match` và tra bản ghi (`http/permission_gate.rs`, PT-13/14, D-20261126-permission-gate) | Endpoint có `x-permissions: []` (mọi người đã đăng nhập) vẫn kiểm đầu vào trước |
| I | Đọc bản nháp hay dữ liệu riêng qua route công khai | Danh sách, tìm kiếm, feed, sitemap và slug công khai lọc theo trạng thái đã đăng; token xem trước ký HMAC, so trong thời gian không đổi (`modules/content/src/publishing/preview.rs`) | PT-11 (thuật ngữ phân loại chưa có bài đăng nào dùng) còn mở (P17) |
| D | Thân lớn hay lồng sâu, bom nén | Giới hạn thân theo route (`http/body_limit.rs`, D-066); thân request có `Content-Encoding` không được giải nén (rà soát P14); độ sâu JSON do bộ parse xử lý (nhóm DOS 2026-11) | — |
| D | Kết nối chậm hay im lặng | Hết hạn đọc header 30 giây, đóng kết nối im lặng, `AUTOXIDE_HTTP_MAX_CONNECTIONS` (M-5, L-10; `http/server.rs`) | — |
| D | Dội request từ một client, một key hay một khách | GCRA theo địa chỉ client (`http/rate_limit.rs`), theo API key, theo khách từ gói của họ (`http/tenant_rate_limit.rs`, D-20261126-tenant-rate-limit) | DoS theo lưu lượng nằm ngoài phạm vi (`SECURITY.vi.md`) |
| D | Tìm kiếm công khai tốn kém | Trần số lượt cùng lúc mỗi tiến trình, giới hạn theo địa chỉ và thời gian (`AUTOXIDE_CONTENT_PUBLIC_SEARCH_*`; D-20261126-idle-polling) | Mỗi lượt tìm vẫn quét bằng `LIKE`; chỉ mục tìm chữ sau một trait còn mở (PT-15, P17) |
| D | Đẩy bật cache `domain_site` bằng `Host` ngẫu nhiên | Cache giới hạn 10.000 mục | Cách bỏ mục còn mở (P16): khi bị dội như vậy, tên miền thật có thể phải đọc database ở mỗi request |
| E | Gọi endpoint mà không có permission | `Principal::require` trong mọi handler (luật 7) cộng lớp kiểm `x-permissions` chung; 2.691 case PERM mỗi database đều qua | — |
| E | Dùng stream event để xem event của khách hay nhánh khác | `StreamHub` lọc theo khách, permission và tầm với (`event/stream.rs`, D-378); giới hạn kết nối mỗi người dùng | Endpoint WebSocket chưa kiểm `Origin` (mức thấp: xác thực bằng header mà trình duyệt không gửi được khi mở WebSocket) (P16) |

## 2. API và MCP endpoint

**Tài sản:** cùng dữ liệu và hành động như REST; mô hình permission; lời hứa tương đương "tool MCP không bao giờ làm
được hơn REST với cùng permission" (luật 8 của CLAUDE.md).

**Điểm vào:** `POST /mcp` và `/mcp/<module>` (Streamable HTTP, JSON-RPC): `tools/list`, `tools/call`,
`resources/list|read`, `prompts/list|get`, `server/discover` (`crates/kernel/src/http/mcp.rs`,
`http/mcp/protocol.rs`, `http/mcp/transport.rs`; D-068, D-20261121-mcp-server). Danh mục ở `docs/mcp-tools.json`
(khoảng 310 tool).

**Giả định tin cậy:**

- MCP xác thực đúng như REST (`Authorization: Bearer` access token hay API key), ở một lớp chạy trước phần giao thức.
- Handler của tool là lớp chuyển mỏng: gọi cùng hàm service với handler REST, và truyền `Principal` mỗi khi REST giới
  hạn permission đó theo phạm vi.

| STRIDE | Mối đe doạ | Control đang có | Còn lại / còn mở |
| ------ | ---------- | --------------- | ---------------- |
| S | Gọi tool không có credential hay với credential giả | Cùng bộ xác thực với REST, 401 trước phần giao thức (`http/mcp.rs`); 312 case MCPAUTH mỗi database đều qua | — |
| S | DNS rebinding nhắm vào MCP client chạy trên máy | `Origin` kiểm theo cùng `OriginGrant` với CORS; origin của tên miền khách gắn với khách đó (`http/mcp/transport.rs`) | — |
| T | Tool ghi ra ngoài phạm vi của người gọi (nhánh, bản ghi của mình) | Gate `mcp-principal`: tool có permission mà REST kiểm theo phạm vi phải dùng `Principal` của nó (`xtask/src/mcp_principal.rs`, D-20261121-security-lints); tool gọi hàm service, không đụng bảng | Gate kiểm theo văn bản: permission truyền qua biến và handler là hàm đặt tên riêng không được kiểm (giới hạn đã ghi trong luật) |
| T | Hành động tiền hay phá huỷ từ một phiên cũ | Step-up áp cho lời gọi tool đúng như REST (`security/access/step_up.rs`, D-20261119-step54-step-up; M-3 cho automations) | — |
| R | Không biết agent nào đã làm gì | Mỗi lời gọi tool một dòng nhật ký request (`mcp:<tool>`, không ghi tham số) kèm API key; thay đổi vào chuỗi audit với tác nhân và key (`ops/request_log.rs`) | — |
| I | `tools/list` lộ thứ người gọi không được dùng | Danh sách tool, resource và prompt lọc theo permission; tool bị cấm trả lời như tool không tồn tại (`http/mcp.rs`) | — |
| I | Tool danh sách trả nhiều dòng hơn REST | Cùng `limit` / `cursor` và `pagination` như REST (CLAUDE.md "Luật code") | 362 case MCPIDOR mỗi database: không trả gì của khách kia |
| D | `tools/list` quá lớn, lời gọi tốn kém | Mỗi module một endpoint (`/mcp/<module>`); REST và MCP xác thực qua cùng bộ xác thực có giới hạn theo API key (`http/mod.rs`, `rate_limit::ApiKeyLimited`) | — |
| E | Tool làm được điều REST cùng permission không làm được | Luật 8; gate `mcp-principal`; L-2 đã sửa (tool thiếu một bước kiểm của REST) | Tool MCP chưa được nối với `operationId` REST từng cái một; nhóm test đã đề nghị (P17, mục cùng PT-11, PT-12, PT-18) |

## 3. Kernel

Kernel (`crates/kernel`) giữ những thứ mọi module dựa vào. Module xác thực `auth` là một module, nhưng với mô hình
này các bảo đảm của nó ở tầm kernel, nên được xét ở đây.

**Tài sản:** khoá ký và secret đã mã hoá; tài khoản, mật khẩu, yếu tố thứ hai, phiên, API key; danh mục permission
và việc gán role; cây tổ chức; bản ghi idempotency; outbox và job queue; cấu hình.

**Điểm vào:** `/api/v1/auth/*` (đăng nhập, refresh, đăng xuất, MFA, passkey, link đăng nhập, đặt lại mật khẩu, lời
mời, OIDC, SCIM, API key, role, uỷ quyền, cấp hỗ trợ); `/api/v1/kernel/*` (cài đặt, khách, tên miền, job, bản ghi,
tag…); job nền và bộ phát event; cấu hình tiến trình.

**Giả định tin cậy:**

- Module nghiệp vụ chỉ kiểm permission qua `Principal` (`security/access/mod.rs`) và không phụ thuộc `auth` (D-046).
- Job và bộ nhận event lấy khách từ dòng đã lưu, không từ request.
- Cấu hình đến từ biến môi trường qua `ConfigReader`; cài đặt theo khách đi qua API cài đặt của tenancy.

| STRIDE | Mối đe doạ | Control đang có | Còn lại / còn mở |
| ------ | ---------- | --------------- | ---------------- |
| S | Access token giả (lẫn `alg`, `tid` của khách khác) | JWT tự viết chỉ nhận `HS256`, so chữ ký trong thời gian không đổi, khoá dẫn xuất theo khách (`modules/auth/src/token.rs`, D-045, D-285); token được đối chiếu với phiên còn sống; mặc định sống 900 giây (`AUTOXIDE_AUTH_ACCESS_TOKEN_SECONDS`) | 47 case TOKEN mỗi database đều qua |
| S | Refresh token bị trộm | Xoay vòng theo họ (family) và phát hiện dùng lại: dùng lại thì thu hồi cả họ (`modules/auth/src/session/`); chỉ lưu bản băm (`session/entity.rs` `token_hash`) | — |
| S | Đoán mật khẩu | Argon2id (tham số OWASP, `modules/auth/src/password.rs`); login throttling theo cặp email + địa chỉ cộng trần rộng hơn theo email, đếm trong database (`session/throttle.rs`, D-079, D-116, D-20261121-login-throttle-pair) | — |
| S | Vượt yếu tố thứ hai | Mã TOTP chỉ dùng một lần mỗi bước (`mfa/service.rs`), recovery code; role đòi đăng nhập hai bước áp cả khi uỷ quyền và hỗ trợ (M-4); tài khoản chỉ có passkey phải dùng passkey (M-8); passkey qua `webauthn_rp` với nghi thức dùng một lần (D-077) | Passkey và OIDC chưa được test trọn trong trình duyệt (đợt 2026-11, mục "chưa phủ") |
| S | Mã SMS dùng một lần bị đoán hay bị dội | Trần theo số, theo khối client và trần chung (PT-4, PT-7, D-20261125-otp-wrong-by-block) | Trần chung vẫn để người lạ khoá đăng nhập SMS của một số trong một thời gian (việc tiếp theo của PT-7, P17) |
| S | Token dùng một lần (đặt lại, xác minh, link đăng nhập) dùng lại hay chuyển sang khách khác | 32 byte ngẫu nhiên, lưu SHA-256, gắn mục đích và khách, tiêu thụ nguyên tử (`security/tokens.rs`, D-117); token trong link mang khách (`data/tenancy/link_token.rs`, D-281) | — |
| S | API key bị dùng từ nơi khác | Key = phạm vi ∩ quyền hiện tại của chủ key, lưu SHA-256, có thể rào IP / CIDR, xoay key (`modules/auth/src/api_key/`, `access.rs`); key không tạo được key, token SCIM hay nhà cung cấp đăng nhập (P-1, `auth.session_required`); key chỉ xem và thu hồi được chính nó, qua REST cũng như MCP, và thu hồi key của người khác cần bao trùm quyền của chủ key (UT-2, #572) | Tới khi #572 được merge, key xem và thu hồi được các key khác của cùng chủ (đợt dùng thử phát hiện) |
| T | Sửa phiên hay role để có thêm quyền | "Chỉ cấp cái mình có", kể cả cho chính mình (PT-9, D-20261126-no-self-grant); thay đổi qua SCIM xét theo quyền người tạo token (PT-1..3, D-20261125-scim-grant-cover) | Vài chỗ bất nhất của luật cấp quyền (so theo tên hay theo phạm vi, mã trạng thái 422 hay 403, thu hẹp role người khác đang giữ) ghi ở P16 |
| T | Quyết định trên một nút tổ chức không thuộc khách | `OrgTree::contains` / `ensure_exists` (PT-8, D-20261125-org-must-exist); bảng closure (`data/tenancy/org_tree.rs`) | Dòng cũ trỏ tới nút không có cần lệnh dọn (P17) |
| T | Job hay event bị xử lý hai lần hay bởi hai worker | Job được nhận bằng `SKIP LOCKED` (PostgreSQL) hay một câu nguyên tử (SQLite) sau trait `Claim` (`job/claim.rs`, D-009); event ghi trong cùng transaction nghiệp vụ, nhận bằng câu cập nhật có điều kiện (`event/outbox.rs`, `event/dispatcher.rs`, D-054); bên nhận phải chịu được nhận lại | Test CI nhiều máy chủ là một mục riêng của G13 |
| T | Đổi cấu hình lúc đang chạy | Biến khai một lần bằng `Setting`, đọc qua `ConfigReader`; không gọi thẳng `std::env::var` (gate `env-access`); cài đặt secret theo khách chỉ ghi được, đã seal (`contracts/provider.rs`, `data/tenancy/settings.rs`) | — |
| R | Hành động bảo mật không để lại dấu | Event cho đăng nhập, đổi role, cấp quyền, uỷ quyền, truy cập hỗ trợ, đổi key vào chuỗi audit; truy cập hỗ trợ phải có lý do (`modules/auth/src/support.rs`) | Chuỗi audit không có khoá: xem ranh giới 5 |
| I | Đọc được secret từ bản dump database | Secret TOTP, secret webhook, khoá nhà cung cấp và URL placement được seal bằng AES-256-GCM gắn với mục đích, khách và dòng (`security/seal.rs`, D-257); xoay khoá bằng `autoxide secrets reseal` | Không đặt `AUTOXIDE_SECRET_KEY` thì khoá gốc nằm trong database (`kernel_secrets`) và đi theo bản sao lưu (`security/secret.rs`, D-113) |
| I | Dữ liệu cá nhân trong log | Log không có thân, token, query hay IP thô; IP lưu dạng HMAC có khoá theo khối /24 hay /64 (`security/ip_hash.rs`, D-115); email admin đầu tiên không vào log (I-4) | Mã hoá ở mức trường cho dữ liệu cá nhân nhạy cảm chưa làm (mục G13) |
| D | Bảng throttle hay idempotency phình không giới hạn | Dòng có hạn và job dọn xoá chúng **(cần kiểm chứng cho từng bảng throttle)** | — |
| D | Tải do poll outbox / job | Lượt rảnh chỉ tốn một câu `EXISTS` mỗi database (D-20261126-idle-polling); dừng êm giữa hai lượt (PT-17) | — |
| E | Principal hẹp chạm tới việc của admin | Permission tên `<module>.<việc>`, phạm vi theo nút tổ chức, `Principal::only` cho automations (D-20261126-automation-owner-revoked) | `run_as_limit` chỉ so tên permission, không so phạm vi; event thu hồi vào dead-letter thì luật không tạm dừng (P16) |
| E | Nhân sự nền tảng đọc dữ liệu khách khi khách không đồng ý | Vào khách cần lần cấp hỗ trợ có thời hạn và lý do; khách tắt được truy cập hỗ trợ (`modules/auth/src/support.rs`, D-313) | — |

## 4. Module

**Tài sản:** bảng và bất biến của từng module (tồn kho, tiền, trạng thái đăng); hợp đồng giữa các module (trait trong
`api`, event); API Rust công khai mà module ngoài repo dùng.

**Điểm vào:** route, tool, đăng ký nhận event, job của module; `context.require::<dyn Trait>()`;
`Autoxide::new().module(…)` cho module ngoài (`crates/app/src/lib.rs`).

**Giả định tin cậy:**

- Module trong repo theo luật ranh giới, do `cargo xtask check` kiểm (D-031): module chỉ mở `pub mod api` và
  `pub fn module(…)`, không import phần bên trong của module khác (`module-import`), không truy vấn database trong
  handler (`handler-database`), không đọc biến môi trường (`env-access`).
- Module sở hữu bảng của mình (`<module>_…`), không foreign key xuyên module, không join bảng của module khác (luật 1
  và 4).
- **Module ngoài là code được tin.** Nó được biên dịch vào cùng tiến trình, dùng cùng kết nối database và khoá
  (D-20261121-external-modules). Luật gate của repo không chạy trên code ngoài repo.

| STRIDE | Mối đe doạ | Control đang có | Còn lại / còn mở |
| ------ | ---------- | --------------- | ---------------- |
| S | Một module giả làm module khác để phát event hay cung cấp trait | Tên event mang tiền tố module và khai qua `Module::events`; trait đăng ký một lần lúc khởi động (`module/registry/`) | Trong một tiến trình, đây là việc của rà soát code, không phải ranh giới lúc chạy |
| T | Một module ghi bảng của module khác | Tiền tố tên bảng theo module và bài test sở hữu bảng trong gate; luật `module-import` | Không áp cho module ngoài |
| T | Bất biến nghiệp vụ bị phá bởi race (bán quá tồn, dùng mã giảm giá hai lần, thanh toán hai lần) | Cập nhật nguyên tử có cận và nhận có điều kiện (rà soát, mục "Money"; M-2); test đồng thời trên cả hai database (CLAUDE.md "Gate") | Việc tiếp theo của P17: trần giá cho giá trị rất lớn, kiểm tiền tệ ở vài đường mua hàng và POS |
| T | Bên nhận tin mù quáng payload của event | Bên nhận đọc lại trạng thái từ bảng của mình hay trait của bên phát; phải chịu được nhận lại (D-054) | Event của module có dữ liệu theo đơn vị phải mang `org_id` ở mọi nơi (P16, D-20261125-event-org) |
| R | Thay đổi qua một module không lần lại được | `audit` nhận mọi event và nối vào chuỗi băm (`modules/audit/src/lib.rs`, D-055, D-183) | — |
| I | Dữ liệu cá nhân còn sót sau khi xoá tài khoản, hay xuất thiếu | Module khai dữ liệu cá nhân của mình cho xuất và xoá (M-7; `contracts/privacy.rs`) | Mỗi module mới phải tự khai; không gate nào kiểm **(cần kiểm chứng)** |
| D | Module đã tắt vẫn gọi được | Module tắt không phục vụ route, tool, job hay event (`module/selection.rs`, `module/dormant.rs`, [`docs/MODULES.vi.md`](MODULES.vi.md)) | — |
| E | Module thử nghiệm làm yếu core | Mặc định tắt; module chính thức không bao giờ phụ thuộc module thử nghiệm ([`docs/MODULES.vi.md`](MODULES.vi.md)) | Module thử nghiệm có cùng test tự động nhưng được rà ít hơn |
| E | Module ngoài vượt qua các bước kiểm của kernel | Không có lúc chạy: Rust không có sandbox trong cùng tiến trình | Người cài phải rà module ngoài như mọi thư viện phụ thuộc; [hướng dẫn viết module](MODULE_GUIDE.vi.md) dặn tác giả dùng `Principal`, `.scoped(&tenant)` và hợp đồng `api` |

## 5. Database

**Tài sản:** mọi dữ liệu lưu trữ; tính toàn vẹn của schema; bản sao lưu; lịch sử migration; chuỗi audit.

**Điểm vào:** kết nối của máy chủ (pool đọc và kết nối ghi cho mỗi database); `autoxide migrate`, `autoxide backup`,
`autoxide restore`, `autoxide copy` (CLI trên máy chủ); sao lưu theo lịch vào kho file; job xuất dữ liệu khách.

**Giả định tin cậy:**

- Mọi tính năng chạy trên SQLite và PostgreSQL (CLAUDE.md "Luật số một"); một bước kiểm chỉ đúng trên một database là
  một phát hiện.
- Mọi bảng cấp khách có `tenant_id`, và mọi truy vấn trên nó lọc qua `.scoped(&tenant)` (`data/tenancy/scope.rs`,
  D-253) — kể cả khi khách có database riêng.
- Chỉ người vận hành tới được file database, thông tin kết nối và bản sao lưu.

| STRIDE | Mối đe doạ | Control đang có | Còn lại / còn mở |
| ------ | ---------- | --------------- | ---------------- |
| S | Request ghi qua kết nối đọc | Kết nối đọc chỉ đọc trên cả hai database (SQLite `read_only`, PostgreSQL `default_transaction_read_only`) (`data/database.rs`, D-009) | — |
| T | Truy vấn quên lọc theo khách | `.scoped(&tenant)` và gate `tenant-scope` (`xtask/src/tenant_scope.rs`, D-20261121-security-lints; M-6 mở rộng cho macro của module); chỗ cố ý đọc mọi khách phải ghi lý do `// tenant-scope:` | Luật kiểm theo văn bản: `E::find()` generic, truy vấn truyền qua hàm phụ và `update` / `delete` một dòng không được thấy (giới hạn đã ghi). 1.119 case REST + 362 case MCP chéo khách mỗi database không thấy gì |
| T | SQL injection qua câu viết tay | `database::statement` với giá trị bind có kiểu (D-20261121-raw-statement); SQL trực tiếp chỉ ở chỗ đã đo (CLAUDE.md "Luật dữ liệu") | — |
| T | Hai tiến trình chạy migration cùng lúc | Lock migration: advisory lock của PostgreSQL, lock file của hệ điều hành cho SQLite (`data/migration/lock.rs`); migration đã commit không bao giờ sửa | — |
| T | Sửa dòng audit mà không bị phát hiện | Dòng audit nối thành chuỗi SHA-256; `autoxide doctor` báo dòng đầu tiên bị sửa, bị xoá hay chèn (D-183) | Chuỗi **không có khoá và không neo ra ngoài**: ai có quyền ghi database dựng lại được chuỗi. Nhật ký khó sửa lén là một mục của G13 |
| T | Bút toán sổ cái bị sửa ngoài module | Bút toán chỉ thêm, sửa bằng bút toán đảo, chuỗi băm (`modules/ledger/src/lib.rs`, D-342) | Cùng giới hạn như chuỗi audit |
| R | Thao tác của người vận hành trên database không được ghi | Ngoài phạm vi (người vận hành được tin) | — |
| I | Người dùng khác trên máy đọc được bản sao lưu | Thư mục sao lưu tạo với quyền `0700` (`data/backup.rs`, D-158); sao lưu theo lịch bỏ qua các bản sao lưu trước | Máy chủ không tự mã hoá file sao lưu; bản sao lưu chứa hash mật khẩu, secret đã seal và, nếu không đặt `AUTOXIDE_SECRET_KEY`, cả khoá gốc. Mã hoá trước khi chép ra ngoài là việc của người vận hành ([`docs/DEPLOY.vi.md`](DEPLOY.vi.md), mục sao lưu mã hoá ngoài máy) |
| I | Bản xuất dữ liệu khách lộ secret | Job xuất bỏ job queue, outbox và cột secret (hash, secret); tải về cần request đã đăng nhập hay link ký sống 24 giờ; nhân sự nền tảng chỉ qua lần cấp hỗ trợ còn hạn (`data/tenancy/exports.rs`, D-328) | — |
| I | `autoxide copy` trộn dữ liệu vào database đang dùng | Database đích phải mới (chưa có migration nào), một transaction, cùng phiên bản schema (`data/transfer.rs`, D-070) | — |
| D | Tranh chấp ghi trên SQLite | Một kết nối ghi, lệnh ghi xếp hàng trong ứng dụng (D-009); không thấy lỗi `busy` dưới 1.024 kết nối (đợt test 2026-11) | Hướng dẫn sức chứa theo database là mục riêng của G13 |
| D | Danh sách không giới hạn hay offset sâu | Chỉ dùng keyset pagination (`fetch_page`, không `OFFSET`), index cho cột sắp; gate `api-unpaged-list` | Vài danh sách cũ còn chờ trong `xtask/api-shape-allow.txt` (P9) |
| E | Code của khách chạm bảng của mặt điều khiển | Bảng mặt điều khiển (khách, tên miền, placement, lượt chuyển) nằm ở database điều khiển; route của khách chỉ chạm qua service dành cho nền tảng (`data/tenancy/admin.rs`, `require_platform`) | — |

## 6. Placement

**Tài sản:** database và kho file nào giữ khách nào; URL kết nối của placement; ánh xạ tên miền → khách; cam kết lưu
trú dữ liệu; khách trong lúc chuyển, tách hay gộp.

**Điểm vào:** `autoxide tenant move` và các endpoint REST / MCP chỉ cho nền tảng về placement, chuyển và tách gộp
(`data/tenancy/placements.rs`, `moves*`, `relocate*`; permission `kernel.placements.read`, `kernel.tenants.move`,
`kernel.tenants.relocate`); endpoint tên miền của từng khách (`data/tenancy/domains*`); sandbox
(`data/tenancy/sandbox*`).

**Giả định tin cậy:**

- Chỉ tài khoản của khách nền tảng mới chuyển, tách, gộp hay xoá khách được (`require_platform`); nhân sự vào một
  khách bằng truy cập hỗ trợ thì không.
- Khách ở database riêng vẫn được lọc theo `tenant_id` (phòng thủ nhiều lớp).

| STRIDE | Mối đe doạ | Control đang có | Còn lại / còn mở |
| ------ | ---------- | --------------- | ---------------- |
| S | Quản trị viên của khách khởi động một lượt chuyển hay tách gộp | `require_platform` + `kernel.tenants.move` / `relocate` (`data/tenancy/moves/http.rs`, `relocate/http.rs`) | — |
| S | Khách giữ lưu lượng của tên miền không còn thuộc mình | Kiểm DNS lại hằng ngày; ba lần không thấy → `failed`; lỗi DNS không tính là không thấy (`data/tenancy/domains.rs`) | Tới khoảng ba ngày định tuyến cũ sau khi tên miền đổi chủ |
| T | Mất hay nhân đôi dữ liệu khi chuyển | Mặc định chạy thử; kiểm trước (phiên bản schema, lượt chuyển đang chạy); sao chép chạy bằng job qua các pha trạng thái; bảng chép theo `tenant_id` với cột có kiểu (`data/tenancy/moves/`, `data/transfer.rs`) | Chuyển khi đang tải và placement database riêng chưa được đợt test 2026-11 phủ |
| T | Tách hay gộp làm gãy tham chiếu | Mỗi module khai cách từng bảng đi theo (`Module::relocation`, `data/tenancy/relocate/policy.rs`, D-20261118-step38-relocation) | Cách suy từ cột chỉ là đoán; test của binary chặn bảng chưa khai (`crates/kernel/src/module.rs`), chỉ phủ module trong repo |
| I | Lộ thông tin kết nối của placement | URL kết nối và cài đặt kho file riêng được seal, gắn với ID placement (`data/tenancy/registry.rs`) | — |
| I | Dữ liệu rời khỏi quốc gia đã cam kết | Lưu trú dữ liệu theo khách; chuyển tới placement ở nước khác hay không rõ nước bị từ chối (`data/tenancy/residency.rs`, D-360) | Quốc gia của placement do người vận hành khai, không đo được |
| I | Sandbox sao chép dữ liệu cá nhân | Sandbox chỉ chép cấu hình, ở cùng placement với khách gốc và tự hết hạn (`data/tenancy/sandbox.rs`, D-232) | Tính trọn vẹn của "chỉ cấu hình" ở từng module **(cần kiểm chứng)** |
| D | Mở quá nhiều pool | Tối đa `AUTOXIDE_TENANCY_MAX_OPEN_DATABASES` pool, bỏ pool ít dùng nhất (`data/tenancy/registry.rs`) | — |
| D | Cache cũ sau khi tạm ngừng khách hay gỡ tên miền | Xoá cache ngay trên máy chủ thực hiện thay đổi; máy chủ khác trong `AUTOXIDE_TENANCY_CACHE_SECONDS` | Tạm ngừng và gỡ tên miền mất tới một TTL ở máy chủ khác |
| E | Khách chạm database của khách khác | Sổ database chỉ mở placement cho khách tìm từ credential, header hay tên miền đã xác minh; khách không tự đưa chuỗi kết nối | — |

## 7. Nhà cung cấp bên ngoài

**Tài sản:** dòng tiền (thanh toán, hoàn tiền, tiền vào tài khoản — core không bao giờ giữ tiền của người mua,
D-187); trạng thái vận đơn; hoá đơn thuế; thông tin tài khoản nhà cung cấp theo khách; secret của webhook gửi đi; vị
trí mạng của máy chủ (SSRF).

**Điểm vào:**

- **Vào:** `/api/v1/ledger/webhooks/{provider}/{hook}`, `/api/v1/ledger/bank-feeds/{provider}/{hook}`,
  `/api/v1/shipping/webhooks/{carrier_code}/{hook}/{token}` (và dạng cũ); IPN của MoMo, biến động số dư SePay,
  callback trạng thái của GHN (`modules/momo`, `modules/sepay`, `modules/ghn`).
- **Ra:** lời gọi thanh toán, vận chuyển, SMS (`modules/esms`), hoá đơn điện tử (`modules/einvoice_sepay`) và mail;
  webhook gửi đi (`modules/webhooks`, chữ ký Standard Webhooks, D-081); OIDC discovery; SMTP của khách; web push
  (hợp đồng `WebPushSender`).
- **Địa chỉ do người dùng chọn:** đích webhook, issuer OIDC, máy chủ SMTP, endpoint của push subscription,
  `return_url` của thanh toán, `return_url` của KYC.

**Giả định tin cậy:**

- Mỗi tài khoản nhà cung cấp thuộc một khách và do khách đó cấu hình; máy chủ không bao giờ dùng tạm tài khoản của
  khách khác (`crates/kernel/src/contracts/provider.rs`, D-20261121-provider-accounts).
- Bộ nối nhà cung cấp trong repo ở **hạng thử nghiệm**: viết theo tài liệu công khai, chưa chạy với dịch vụ thật (đầu
  `lib.rs` của từng bộ nối).

| STRIDE | Mối đe doạ | Control đang có | Còn lại / còn mở |
| ------ | ---------- | --------------- | ---------------- |
| S | Callback thanh toán, ngân hàng hay vận chuyển giả | Tìm khách trước từ mã định danh không đoán được trong URL riêng của khách, rồi kiểm chữ ký của nhà cung cấp bằng secret của đúng khách đó, so trong thời gian không đổi (`contracts/provider.rs`, `modules/momo/src/gateway.rs`, `modules/sepay/src/feed.rs`) | Chữ ký thật của nhà cung cấp chưa được test (chỉ có bản giả) |
| S | Callback từ tài khoản vận chuyển của người bán này cập nhật vận đơn của người bán khác | Chỉ khớp vận đơn đặt qua đúng tài khoản vận chuyển nhận callback (H-1, `modules/shipping/src/webhooks.rs`) | — |
| T | Lệch số tiền, tiền tệ hay tài khoản mà vẫn ghi nhận thanh toán | Số tiền và tiền tệ so với thanh toán đã lưu; tiền vào phải đúng tài khoản ngân hàng của thanh toán (M-1); mục đối soát được nhận trước khi ghi nhận (M-2); hoàn tiền giới hạn theo số đã thu (L-6); nhà cung cấp hoàn tiền phải khớp (I-1) | Khoản thanh toán khác tiền tệ sau đối soát vào dead-letter, chưa có hướng dẫn cho người vận hành (P17) |
| T | Callback bị gửi lại | Lượt gửi được chống trùng (rà soát, mục "Inbound webhooks") — cơ chế của từng bộ nối **(cần kiểm chứng)** | — |
| R | Tranh cãi về điều nhà cung cấp đã báo | Lời gọi nhà cung cấp ghi log không có secret, kèm số đo (`contracts/provider.rs` `send`); bút toán sổ cái chỉ thêm | — |
| I | Lộ secret của nhà cung cấp | Cài đặt secret được seal theo khách và dòng, chỉ ghi được qua API cài đặt (`contracts/provider.rs`, D-257) | — |
| I | Chuyển hướng mở sau thanh toán hay xác minh danh tính | `return_url` của thanh toán phải là `https://` trên tên miền đã xác minh của khách hay origin trong danh sách (P-5, `ledger.return_url_not_allowed`) | `return_url` của KYC chưa có danh sách (L-11, P16); hiện chưa có bộ nối xác minh danh tính nào |
| I | SSRF: máy chủ gọi vào địa chỉ nội bộ | Bộ lọc chung `crates/kernel/src/security/net.rs` (D-149, D-20261121-ssrf-targets): IP không công khai bị từ chối lúc lưu; tên miền phân giải qua `PublicOnly` lúc kết nối (chống DNS rebinding), tắt chuyển hướng, bỏ qua proxy hệ thống (L-4); giao thức không phải HTTP phân giải một lần rồi nối đúng IP đó | 65 case SSRF mỗi database: không gì tới được listener trên máy |
| I | Web push tới địa chỉ nội bộ | Push subscription chỉ nhận máy chủ công khai; hợp đồng `WebPushSender` đòi chỉ nối IP công khai (PT-10, `contracts/push.rs`) | Repo chưa có bộ gửi web push thật |
| D | Lời gọi ra bị treo hay trả thân rất lớn | `http_client` có timeout, không chuyển hướng; thân response giới hạn bởi `AUTOXIDE_PROVIDER_MAX_RESPONSE_BYTES` (I-2); thân webhook giới hạn theo route (`webhook_max_bytes` của `ledger`) | — |
| D | Lạm dụng chi phí SMS (mỗi tin do cửa hàng trả) | Giới hạn theo số, theo khối client và theo giờ (P-2, PT-4) | — |
| E | Gọi nhà cung cấp trong transaction ghi làm khoá database | Luật: không gọi mạng trong transaction ghi (CLAUDE.md "Luật dữ liệu", `contracts/provider.rs`) | Do rà soát giữ, chưa có gate |
| E | Webhook gửi đi lộ event mà chủ nơi nhận không được xem | Tạo webhook đòi permission stream của event; payload có chữ ký (`modules/webhooks`) | — |

## 8. AI

**Tài sản:** nội dung của khách gửi tới mô hình; ngân sách mô hình của người cài; tính đúng đắn của những gì được
đăng; luật đưa cho mô hình.

**Điểm vào:** các tính năng gọi `TextModel` hay `Embedder` (`crates/kernel/src/contracts/ai.rs`, do `modules/ai`
công bố): gợi ý SEO và alt text, dịch, bản nháp mô tả sản phẩm (`catalog`), bản nháp trả lời hỗ trợ
(`support/src/ai.rs`), tóm tắt đánh giá (`orders/src/reviews.rs`), bản nháp hội thoại, tìm theo nghĩa. AI agent đóng
vai client của API thuộc ranh giới 1–3.

**Giả định tin cậy:**

- AI tắt cho tới khi người cài cấu hình nhà cung cấp ([`docs/AI.vi.md`](AI.vi.md)).
- Kết quả của mô hình là dữ liệu không tin cậy: phải khớp JSON Schema và chỉ thành **gợi ý hay bản nháp**; người
  (hay agent có permission đăng) quyết định (D-109).
- Luật cho mô hình ở `instructions`; dữ liệu người dùng chỉ ở `input` (`Prompt::new(…).input(…)`).

| STRIDE | Mối đe doạ | Control đang có | Còn lại / còn mở |
| ------ | ---------- | --------------- | ---------------- |
| S | Nội dung do kẻ tấn công kiểm soát (một đánh giá, một phiếu hỗ trợ, một mô tả sản phẩm) bảo mô hình làm việc khác (prompt injection) | Tách `instructions` và `input`; kết quả phải khớp JSON Schema (`ai.output_invalid`); mô hình không có tool và không có credential; kết quả chỉ là bản nháp (D-109) | Injection vẫn có thể làm lệch bản nháp; người duyệt là control. Bài do AI tạo mang `ai_generated`, lúc đăng ghi `ai_reviewed` ([`docs/AI.vi.md`](AI.vi.md)) |
| T | Bản nháp ghi đè sửa đổi của người | Bản dịch chỉ lưu khi không ai sửa bản đích từ lúc yêu cầu (`content.translation_conflict`) | — |
| T | Kết quả mô hình mang markup hay link không an toàn | Bản dịch giữ link, ảnh và ID từ bản gốc; thẻ đánh số phải về đủ và đúng | Việc làm sạch mọi bản nháp AI trước khi client hiển thị **(cần kiểm chứng theo từng tính năng)** |
| R | Ai đã đăng chữ do AI tạo | `ai_reviewed` kèm người và thời điểm khi đăng; chuỗi audit | — |
| I | Gửi dữ liệu cá nhân hay riêng tư tới nhà cung cấp | Danh sách những gì được gửi có trong tài liệu; ảnh riêng tư chỉ gửi khi `AUTOXIDE_MEDIA_AI_SEND_PRIVATE=true`; lịch sử hỗ trợ gửi đi đã bỏ email và số điện thoại; prompt, nội dung và khoá không bao giờ vào log ([`docs/AI.vi.md`](AI.vi.md), `modules/support/src/ai.rs`) | Nhà cung cấp do người cài chọn; thoả thuận xử lý dữ liệu là việc của họ |
| D | Cạn ngân sách | Hạn mức token mỗi ngày theo khách, từ gói (`ai.daily_tokens`) hay `AUTOXIDE_AI_DAILY_TOKEN_LIMIT`, đếm trong database chung mọi máy chủ (`modules/ai/src/config.rs`, `ai_usage`); job bị cắt và chạy lại; rate limiting theo key | — |
| E | AI agent đăng hay xoá vượt vai trò | Permission `_own` giới hạn key của agent vào bài tạo bằng chính key đó; không lưu trữ / xoá khi thiếu permission (`content.write_own`, `content.publish_own`) | — |

---

## Ngoài phạm vi

- Người vận hành: bất kỳ ai có shell, biến môi trường, database hay bản sao lưu. Các cấu hình sai mà
  [`SECURITY.vi.md`](../SECURITY.vi.md) liệt kê (CORS `*`, `/metrics` công khai, rate limiting bằng `0`, header địa chỉ
  client khi không có proxy, secret chưa đặt).
- DoS theo lưu lượng; khuếch đại rẻ bằng một request đơn lẻ thì **nằm trong** phạm vi.
- Quản trị viên làm điều quản trị viên được làm trong chính khách của họ.
- Giao diện người dùng và bên dùng SDK (repo riêng), reverse proxy, cấu hình TLS, hệ điều hành, môi trường chạy
  container, DNS của tên miền khách.
- Tài khoản nhà cung cấp thật (thanh toán, ngân hàng, SMS, vận chuyển, hoá đơn điện tử, xác minh danh tính) và nhà
  cung cấp AI; bảo mật của chính họ.
- Chuỗi cung ứng ngoài những gì `cargo deny`, `gitleaks`, action ghim theo commit và bản phát hành có chữ ký đã phủ
  ([`docs/VERIFYING.vi.md`](VERIFYING.vi.md)).
- Bản thân việc tuân thủ pháp luật (xem [`docs/COMPLIANCE.vi.md`](COMPLIANCE.vi.md)); mô hình này chỉ xét control
  kỹ thuật của nó.

## Việc còn mở đã biết

Trạng thái nằm ở [`docs/ROADMAP.md`](ROADMAP.md), bước 70, mục **P16** (việc tiếp theo của đợt rà soát tháng 11/2026)
và **P17** (sửa lỗi của đợt test tháng 11/2026), và ở bảng trạng thái của
[`docs/security/pentest-2026-11/README.vi.md`](security/pentest-2026-11/README.vi.md). Lúc viết, các lớp còn mở là:

| Lớp lỗi | Ranh giới | Theo dõi ở |
| ------- | --------- | ---------- |
| Chuyển hướng mở: `return_url` của xác minh danh tính chưa có danh sách cho phép | 7 | P16 (L-11) |
| Bất nhất của luật cấp quyền (tên permission hay phạm vi; mã trạng thái; thu hẹp role người khác đang giữ; thành viên SCIM chờ, người tạo liên kết SCIM) | 3 | P16 |
| Luật tự động hoá: chưa chụp phạm vi của chủ luật; event thu hồi vào dead-letter không tạm dừng luật | 3 | P16 |
| Event của dữ liệu theo đơn vị chưa mang `org_id` ở vài module | 4 | P16 |
| Bỏ mục cache khi bị dội `Host` ngẫu nhiên; `Origin` của WebSocket; test chắc chắn cho cuộc đua nhận tên miền và cho lượt đọc ảnh chụp tên miền bị treo | 1, 6 | P16 |
| Tìm kiếm công khai tốn kém (chỉ mục tìm chữ sau một trait) | 1 | P17 (PT-15) |
| Lộ thông tin giá trị thấp: thuật ngữ phân loại chưa dùng, analytics bị làm bẩn, trần mail xác nhận newsletter | 1 | P17 (PT-11, PT-12, PT-18) |
| Giá hay cân nặng rất lớn bị bão hoà thay vì bị từ chối; kiểm tiền tệ ở vài đường mua hàng / POS | 4, 7 | P17 |
| Bản ghi trỏ tới nút tổ chức không có, tạo trước PT-8 | 3 | P17 |
| Trần chung của đăng nhập SMS có thể khoá một số trong một thời gian | 3 | P17 (việc tiếp theo của PT-7) |
| Gate `address-key` chỉ bắt một mẫu viết | 1 | P17 |
| Nhật ký audit khó sửa lén (chuỗi có khoá hay neo ra ngoài); mã hoá ở mức trường cho dữ liệu cá nhân nhạy cảm, khoá quản lý được và xoay được | 3, 5 | G13 (bước này) |

Ở đây chỉ nêu lớp lỗi. Chi tiết của một lỗi còn mở chỉ chia sẻ riêng, qua kênh trong
[`SECURITY.vi.md`](../SECURITY.vi.md).

## Dùng tài liệu này cho đợt kiểm thử bên ngoài

**Chuẩn bị.** Dùng `AUTOXIDE_TENANCY_MODE=multi`, ít nhất hai khách cộng khách nền tảng, một cây tổ chức có nhân
viên giới hạn theo nút, và cả hai database. Script của đợt tháng 11/2026
([`docs/security/pentest-2026-11/scripts/`](security/pentest-2026-11/scripts/)) dựng sẵn bộ như vậy và một ma trận
case; dùng lại làm nền thay vì chạy lại các nhóm sinh tự động. Trỏ mọi nhà cung cấp tới bản giả trên máy hay tài khoản
sandbox của chính bạn; chỉ test trên bản cài của chính bạn.

**Trọng tâm đề xuất, giá trị cao nhất trước:**

1. **Những gì các đợt trước chưa phủ:** passkey và OIDC trọn vòng trong trình duyệt; callback thanh toán, biến động
   số dư và vận chuyển với chữ ký thật của nhà cung cấp (tài khoản sandbox); chuyển, tách và gộp khách giữa các
   database khi đang tải; khách ở database riêng; tải file và media ở giới hạn kích thước; WebSocket vượt trần mỗi
   người dùng; `ai` với mô hình thật.
2. **Logic cấp quyền và phạm vi (ranh giới 3):** role, thành viên, uỷ quyền, cấp hỗ trợ, SCIM, phạm vi API key ∩ chủ
   key, `run_as` của automations, step-up — tìm mọi đường làm một principal có nhiều hơn người tạo hay người giữ nó.
   Danh sách P16 nêu các vùng đã biết là bất nhất.
3. **Cách ly khách và nhánh ở chỗ gate không thấy (ranh giới 5):** đường code mà luật `tenant-scope` và
   `mcp-principal` không thấy (truy vấn generic, hàm phụ, handler là hàm đặt tên riêng) — so từng tool MCP với
   endpoint REST tương ứng.
4. **Chọn khách theo `Host` và CORS (ranh giới 1):** tên miền đã xác minh, `?site=`, tên miền đổi chủ giữa các khách,
   TTL của cache khi có nhiều máy chủ, khách bị tạm ngừng.
5. **Đường tiền (ranh giới 4, 7):** kiểm tiền tệ và số tiền ở mọi đường thanh toán, hoàn tiền, POS, số dư trả trước
   và khuyến mãi; race trên cả hai database.
6. **Bản nháp AI (ranh giới 8):** nội dung cài vào đánh giá, phiếu hỗ trợ hay mô tả sản phẩm có tạo được bản nháp mà
   client hiển thị không an toàn hay đánh lừa người duyệt không.
7. **Từng control ghi (cần kiểm chứng)** ở dưới.

**Báo cáo.** Dùng thang mức độ của [đợt rà soát tháng 11/2026](security/review-2026-11.md#severity) và kênh riêng
trong [`SECURITY.vi.md`](../SECURITY.vi.md). Ghi ranh giới và loại STRIDE theo tài liệu này.

## Control cần kiểm chứng

Các control do thiết kế hay tài liệu mô tả nhưng chưa được lần theo trọn trong code khi viết tài liệu này:

1. Mọi bảng throttle và idempotency có hạn và job dọn (ranh giới 3).
2. Mọi module giữ dữ liệu cá nhân đều khai nó cho xuất và xoá; chưa có gì buộc module mới làm vậy (ranh giới 4).
3. Sandbox chỉ chép cấu hình, ở mọi module (ranh giới 6).
4. Callback từ nhà cung cấp: cơ chế chống gửi lại / chống trùng của từng bộ nối (ranh giới 7).
5. Bản nháp AI: việc làm sạch kết quả ở mọi tính năng lưu chữ do mô hình viết (ranh giới 8).

## Giữ tài liệu này luôn đúng

Sửa file này trong cùng pull request với thay đổi thêm một ranh giới, một loại điểm vào (một loại credential mới, một
callback vào mới, một giao thức gọi ra mới) hay một control ghi ở đây, và sửa bản tiếng Anh cùng lúc.
