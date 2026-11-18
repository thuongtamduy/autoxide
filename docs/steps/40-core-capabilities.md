# Bước 40 — Năng lực còn thiếu của core

> Lộ trình: `docs/ROADMAP.md` bước 40 · Thiết kế: `docs/research/business-foundation.md` trụ 1–2,
> mục 7; `docs/research/frameworks.md` (mục "Không mượn": realtime theo bảng) · Quyết định: D-038
> (hàm mới của `Module` có mặc định), D-045 (JWT tự viết, không `jsonwebtoken`), D-068 (MCP), D-077
> (ngoại lệ `rsa` trong `deny.toml`), D-116 (`Throttle`), D-149 (SSRF của webhooks), D-189, D-190, D-192

## Mục tiêu

Core có đủ sáu năng lực chung mà một lõi backend hiện đại thường có, không gắn ngành nào:
header bảo mật cho mọi response; MCP đủ ba phần (tools, **resources**, **prompts**) cộng trang
`/llms.txt` cho agent tự khám phá; đăng nhập qua Google / Microsoft / GitHub / OIDC bất kỳ; tải file
lớn thẳng lên S3; đẩy event theo thời gian thực bằng SSE; rate limit đúng khi nhiều máy chủ. Bốn mục
sau làm theo khách hàng và tổ chức: mỗi khách tự khai nhà cung cấp đăng nhập của mình, upload tính
vào hạn mức của khách, stream chỉ có event của khách và nhánh người gọi được xem, rate limit theo API
key trong khách. Phục vụ mọi mô hình của D-185 (chuỗi dùng Microsoft Entra của công ty mẹ; sàn cho
người mua đăng nhập Google; SaaS giới hạn request theo gói).

## Bắt đầu khi

| Phần | Bắt đầu | Cần có |
|---|---|---|
| **P1 Header bảo mật** | **ngay**, trên code hiện tại | `kernel/src/http/mod.rs` |
| **P2 MCP đầy đủ hơn** | **ngay**, trên code hiện tại | `kernel/src/mcp.rs`, `kernel/src/http/mcp.rs`, `rmcp` 3.9 |
| P3 OIDC · P4 tải thẳng S3 · P5 SSE · P6 rate limit | SAU bước 34 | bước 33: `TenantId`, `TenantContext`, `DatabaseRegistry`, secret theo khách (AEAD), khoá file có tiền tố khách, rate limit trong bộ nhớ có khách trong khoá; bước 34: `OrgId`, `Scope`, `Principal` theo nút, danh tính hai lớp; (P4 dùng `Entitlements::reserve` của bước 35 nếu đã có — không có thì bỏ qua hạn mức, ghi chú) |

P1 và P2 là hai PR độc lập, merge riêng; bước 40 chỉ đánh dấu xong khi cả sáu phần xong.

## Phạm vi

Sửa: `crates/kernel/src/{http/mod.rs, http/mcp.rs, mcp.rs, module.rs, module/registry.rs,
module_catalog.rs, storage.rs, throttle.rs, http/rate_limit.rs, config/http.rs, event/…}`,
`modules/{files,media,auth,webhooks}`, `crates/testkit`, `docs/openapi.json`, `.env.example`, `STACK.md`
(nếu thêm thư viện / feature), `README` + `README.vi.md` (mục MCP / đăng nhập), `docs/DEPLOY.md` +
`.vi.md`, `docs/MODULE_GUIDE.md` + `.vi.md` (resources, prompts, stream event), `CHANGELOG.md`.

File mới: `kernel/src/http/security_headers.rs`, `kernel/src/http/llms.rs`, `kernel/src/net.rs`
(lọc địa chỉ public dời từ `webhooks/src/network.rs`), `kernel/src/upload.rs`,
`kernel/src/event/stream.rs`, `kernel/src/http/stream.rs`, `modules/auth/src/oidc/` (thư mục).

NGOÀI phạm vi: realtime theo bảng (đẩy mọi thay đổi dòng — D-189 / frameworks.md); WebSocket; upload
multipart / resumable (S3 một lần PUT ≤ 5 GiB); SAML; làm nhà cung cấp OIDC (Autoxide là bên dùng, không
cấp `id_token` cho ứng dụng khác); MCP sampling / elicitation / subscribe resource; Redis.

## Thiết kế

### P1 — Header bảo mật (làm ngay)

Một lớp `http/security_headers.rs`, gắn trong `http::build` **ngoài** lớp `log` (ngay trong lớp
request id) để mọi response đều có — kể cả 404 của router, 429, preflight CORS, `/health/*`,
`/metrics`, `/mcp`, `/api/v1/openapi.json`. Chỉ đặt header **khi handler chưa đặt** (handler giữ quyền
đặt chặt hơn).

| Header | Giá trị mặc định | Ghi chú |
|---|---|---|
| `X-Content-Type-Options` | `nosniff` | |
| `Referrer-Policy` | `no-referrer` | cấu hình được |
| `X-Frame-Options` | `DENY` | cho trình duyệt cũ |
| `Content-Security-Policy` | `default-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'; sandbox` | hằng `pub const API_CSP` trong kernel; JSON không cần gì hơn, file người dùng tải lên không chạy được script |
| `Content-Security-Policy` ở `DOCS_PATH` (`/api/docs`) | `default-src 'none'; script-src https://cdn.jsdelivr.net; style-src 'unsafe-inline' https://cdn.jsdelivr.net; font-src https://cdn.jsdelivr.net data:; img-src 'self' data: https:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'` | trang Scalar của `utoipa-scalar` 0.4 nạp script từ jsdelivr; khối `<script type="application/json">` không bị chặn. **Mở trang trong trình duyệt thật để kiểm** — Scalar có thể cần thêm nguồn font; chỉ mở thêm đúng nguồn bị chặn (đọc console) |
| `Strict-Transport-Security` | không gửi | gửi `max-age=N` khi `AUTOXIDE_HTTP_HSTS_SECONDS > 0`, thêm `; includeSubDomains` khi bật biến tương ứng |

Không đặt `Cross-Origin-Resource-Policy` toàn cục (ảnh `media` được trang web ở origin khác nhúng).

Cấu hình (`config/http.rs`, thêm vào `HttpConfig`):

| Biến | Mặc định | |
|---|---|---|
| `AUTOXIDE_HTTP_SECURITY_HEADERS` | `true` | tắt khi reverse proxy đã đặt (vẫn khuyên để bật) |
| `AUTOXIDE_HTTP_REFERRER_POLICY` | `no-referrer` | kiểm thuộc tập giá trị chuẩn |
| `AUTOXIDE_HTTP_HSTS_SECONDS` | `0` | 0 = không gửi; 0–63072000 |
| `AUTOXIDE_HTTP_HSTS_INCLUDE_SUBDOMAINS` | `false` | |

`autoxide doctor`: cảnh báo khi `AUTOXIDE_HTTP_PUBLIC_URL` là `https://…` mà HSTS = 0 (gợi ý, không lỗi).

Hai endpoint đang tự đặt header — `modules/media/src/http/content.rs` (dòng `nosniff` + CSP
`default-src 'none'; sandbox`) và `modules/files/src/http.rs` (`nosniff`) — bỏ các dòng đó, dựa vào
lớp chung (CSP mặc định đã gồm `sandbox`, chặt hơn). Sửa test của chúng so với hằng `API_CSP` của
kernel thay vì chuỗi cũ. Endpoint không đổi hợp đồng (header chỉ chặt thêm) — nếu `docs/openapi.json`
có khai header CSP trong response của media thì sinh lại.

### P2 — MCP đầy đủ hơn (làm ngay)

Kiểu mới trong `autoxide_kernel::mcp` (phác thảo; tra `rmcp` 3.9 bằng `check-library-api`: `ServerHandler::
list_resources`, `read_resource`, `list_prompts`, `get_prompt`, `ServerCapabilities::builder()
.enable_resources().enable_prompts()`):

```rust
/// Một tài liệu chỉ đọc cho agent. Tên `McpResource` để không lẫn với `autoxide_kernel::resource`
/// (sổ đối tượng của D-123).
pub struct McpResource { /* uri, name, description, mime_type, permission, handler */ }
impl McpResource {
    /// `path` ngắn (vd `docs`, `schema/entry`); kernel ghép URI `autoxide://<module>/<path>`.
    pub fn new<F, Fut>(path: &'static str, name: &'static str, description: &'static str,
        mime_type: &'static str, read: F) -> Self
    where F: Fn(Principal) -> Fut + Send + Sync + 'static,
          Fut: Future<Output = Result<String, Error>> + Send + 'static;
    pub fn requires(self, permission: Permission) -> Self;
    /// Tài liệu tĩnh (không cần người gọi): `McpResource::text(path, name, description, mime, &'static str)`.
    pub fn text(...) -> Self;
    /// JSON Schema của kiểu `T: ToSchema` — "schema một loại bản ghi", sinh từ code.
    pub fn schema<T: utoipa::ToSchema>(path: &'static str, description: &'static str) -> Self;
}
pub struct Prompt { /* name, description, arguments, uses: Vec<String> (tool), render */ }
pub struct PromptArgument { pub name: &'static str, pub description: &'static str, pub required: bool }
impl Prompt {
    pub fn new(name: &'static str, description: &'static str,
        render: impl Fn(&BTreeMap<String, String>) -> Result<Vec<PromptMessage>, Error> + Send + Sync + 'static) -> Self;
    pub fn argument(self, argument: PromptArgument) -> Self;
    /// Tool mà prompt hướng dẫn gọi (tên ngắn trong module): prompt chỉ hiện với người gọi được
    /// MỌI tool đó — không lộ việc người gọi không làm được.
    pub fn uses(self, tool: &'static str) -> Self;
}
pub enum PromptMessage { User(String), Assistant(String) }
```

`Module` thêm hai hàm, mặc định rỗng (D-038): `fn mcp_resources(&self) -> Vec<McpResource>` và
`fn prompts(&self) -> Vec<Prompt>` (gọi sau `start`, như `tools`). Thêm `fn description(&self) ->
&'static str { "" }` — một câu tiếng Anh cho `/llms.txt` và sổ module. `Modules` gom và kiểm lúc khởi
động như `tools()`: tên dạng `[a-z0-9_/]`, không trùng, permission có người khai, `uses` trỏ tới tool có
thật của cùng module, mô tả không có chữ chưa dịch (`untranslated_text`). Kernel thêm tiền tố: prompt
`<module>.<tên>`, URI `autoxide://<module>/<path>`.

Endpoint `/mcp` (`http/mcp.rs`): `get_info` bật thêm resources + prompts; `list_resources` /
`list_prompts` lọc theo permission y như `tools/list`; `read_resource` / `get_prompt` với URI / tên không
có **hoặc** không được dùng trả cùng một lỗi (`invalid_params`, "Unknown resource … call
`resources/list`"). Kết quả resource là text; prompt trả `messages`. Cập nhật `INSTRUCTIONS` nhắc
resources và prompts. `module_catalog` thêm `resources` và `prompts` vào `ModuleInfo` (trường mới —
không phá vỡ).

Resource và prompt có sẵn:

| Module | Loại | Tên / URI | Permission | Nội dung |
|---|---|---|---|---|
| `kernel` | resource | `autoxide://kernel/llms.txt` | — | đúng nội dung `/llms.txt` |
| `kernel` | resource | `autoxide://kernel/openapi.json` | — | tài liệu OpenAPI (cùng `Bytes` đã dựng sẵn, D-158) |
| `kernel` | resource | `autoxide://kernel/modules` | `kernel.modules.read` | sổ module (JSON) |
| `content` | resource | `autoxide://content/schema/entry` | `content.read` | `McpResource::schema::<EntryResponse>` (tên kiểu thật: tra trong `modules/content/src/http`) |
| `content` | resource | `autoxide://content/guide` | `content.read` | hướng dẫn viết bài cho agent (trạng thái, bản nháp, xuất bản cần `content.publish`) — viết trong `modules/content/src/mcp_guide.md`, `include_str!` trong crate |
| `comments` | resource | `autoxide://comments/guide` | `comments.moderate` | quy tắc kiểm duyệt (trạng thái, khi nào cấm) |
| `content` | prompt | `content.draft_post` (args: `topic` bắt buộc, `locale`, `audience`) | uses `create_draft` | hướng dẫn agent viết nháp rồi gọi `content.create_draft`; **không** gọi `publish` |
| `comments` | prompt | `comments.moderate_queue` (args: `limit`) | uses `queue`, `moderate` | đọc `comments.queue`, quyết từng bình luận, gọi `comments.moderate` kèm lý do |

Đối số người dùng chèn vào prompt nằm trong khối có ranh giới rõ (`<topic>…</topic>`) và được nói là
dữ liệu, không phải chỉ dẫn (tinh thần D-109: luật ở chỉ dẫn, dữ liệu người dùng tách riêng).

`/llms.txt` (`http/llms.rs`, route gốc như `health_routes`, có `#[utoipa::path]`, tag `health`,
`text/plain; charset=utf-8`, công khai, `Cache-Control: public, max-age=300`): sinh lúc dựng router từ
code — tên + phiên bản, một đoạn mô tả, link tuyệt đối (ghép `AUTOXIDE_HTTP_PUBLIC_URL`, không có thì
đường dẫn tương đối) tới `/api/v1/openapi.json`, `/mcp`, `/api/docs` (chỉ khi bật), cách xác thực
(`Authorization: Bearer <API key>`, tạo key ở `POST /api/v1/auth/api-keys`), quy ước (envelope `data` /
`pagination`, lỗi RFC 9457 có `code`, `Idempotency-Key`, `limit` / `cursor` / `order`), danh sách module
(`name` — `description()`), danh sách prompt và resource công khai. Theo định dạng llmstxt.org (H1, câu
tóm tắt trong `>`, các mục `##` với danh sách link). Sau bước 35, khi có ngữ cảnh khách, chỉ liệt kê
module khách đã bật — ghi TODO có điều kiện, không làm trước khi có `Entitlements`.

### P3 — Đăng nhập OIDC / OAuth 2.0 (sau bước 34)

Thuộc module `auth` (thư mục `modules/auth/src/oidc/`). Luồng cho core chỉ có API (trình duyệt do
giao diện riêng điều khiển):

1. `GET /api/v1/auth/oidc/providers` (công khai, theo khách của request) → `[{id, slug, kind, name}]`.
2. `POST /api/v1/auth/oidc/{slug}/start` `{redirect_uri}` → `{authorization_url,
   expires_in}` (liên kết khi đã đăng nhập đi qua `/me/identities/{slug}/start`, không qua cờ `link` — D-380). Máy chủ sinh `state` (32 byte), `nonce`, PKCE `code_verifier` (S256); lưu bằng
   `OneTimeTokens` của kernel (bảng `auth_tokens` sẵn có, `purpose = "oidc_state"`, `subject` =
   provider id, `data` = JSON `{nonce, verifier, redirect_uri, link_user_id}` **mã hoá** bằng
   AEAD của bước 33 — khách nằm trong dữ liệu đi kèm của AEAD và trong `state` dạng `link_token`), hạn 10 phút. `redirect_uri` phải nằm trong `redirect_uris` của provider.
3. Giao diện chuyển trình duyệt tới `authorization_url`; nhà cung cấp quay về `redirect_uri` của giao
   diện với `code` + `state`.
4. `POST /api/v1/auth/oidc/{slug}/callback` `{code, state}` → **cùng hình dạng** với
   `POST /api/v1/auth/login` (cặp token, hoặc `mfa_required` + `mfa_token` → `/login/mfa`). Tiêu thụ
   `state` một lần (`consume`), đổi `code` lấy token (gửi `code_verifier`), kiểm `id_token`.

Kiểm `id_token` (tự viết như D-045, ~150 dòng, `modules/auth/src/oidc/jwt.rs`): chỉ nhận `alg` trong
`RS256` / `ES256` theo JWKS của issuer (không bao giờ `none`, không HS*); chọn khoá theo `kid`; kiểm
`iss` (Microsoft đa tenant: `iss` chứa tenant id thật — so với mẫu
`https://login.microsoftonline.com/{tid}/v2.0` với `tid` lấy từ claim), `aud` chứa `client_id` (có
`azp` thì phải bằng `client_id`), `exp` / `iat` / `nbf` với lệch giờ ±60 s, `nonce` khớp. RS256 kiểm
bằng `ring::signature::RsaPublicKeyComponents { n, e }.verify(&RSA_PKCS1_2048_8192_SHA256, …)` (ring đã
có trong cây qua rustls — **khuyến nghị**, không thêm `rsa` / `jsonwebtoken` / `openidconnect`, tránh
mở rộng ngoại lệ RUSTSEC của D-077); ES256 bằng `p256` (đã có). Discovery
`/.well-known/openid-configuration` và JWKS cache trong bộ nhớ theo `Cache-Control` (tối thiểu 5 phút,
tối đa 24 giờ); gặp `kid` lạ thì tải lại JWKS, tối đa một lần mỗi phút mỗi issuer.

Bốn kiểu (`kind`): `google` (issuer `https://accounts.google.com`), `microsoft` (issuer theo tenant hay
`organizations`), `github` (OAuth 2.0 thuần, không có `id_token`: đọc `GET https://api.github.com/user`
và `/user/emails` lấy email `primary && verified`; subject = `id` số của GitHub), `oidc` (issuer bất kỳ,
discovery bắt buộc). Gọi ra ngoài bằng `reqwest` (đã có) qua bộ phân giải chỉ cho IP public — dời
`modules/webhooks/src/network.rs` thành `autoxide_kernel::net` (`is_public`, `PublicOnly`), webhooks
dùng lại (một cách làm); quản trị khách khai issuer được, nên **phải chặn SSRF**. Test cần
`AUTOXIDE_AUTH_OIDC_ALLOW_PRIVATE_NETWORKS=true`.

Liên kết tài khoản (theo câu của ROADMAP):
- Đã có `auth_identities` cho `(provider, subject)` → đăng nhập tài khoản đó.
- Chưa có, email nhà cung cấp **đã xác minh** (Google `email_verified = true`; GitHub `verified`;
  Microsoft chỉ khi claim `xms_edov = true` — claim `email` của Entra đổi được và không được xác minh;
  `oidc` theo `email_verified`) **và** khớp tài khoản có email đã xác minh trong Autoxide → tạo liên
  kết, ghi event, đăng nhập.
- Khớp tài khoản mà email trong Autoxide **chưa** xác minh, hoặc email nhà cung cấp chưa xác minh →
  KHÔNG gộp, lỗi `auth.oidc_link_required` (đăng nhập bằng cách cũ rồi liên kết ở
  `POST /api/v1/auth/me/identities/{slug}/start`) — chặn "chiếm tài khoản dựng sẵn".
- Không khớp ai → tạo tài khoản mới chỉ khi provider bật `allow_signup` **và** đăng ký mở
  (`AUTOXIDE_AUTH_SELF_REGISTRATION` cho nhân sự, `auth.customer_registration` cho người mua) — hoặc provider có
  `default_role_id` (quản trị khách chủ động mở cửa, D-380); provider có `default_org_id` +
  `default_role_id` thì gán thành viên nút đó (đăng nhập theo tổ chức: chuỗi dùng Entra của mình cho
  nhân viên). Không thì `auth.oidc_signup_closed`.
- MFA của Autoxide vẫn áp dụng: TOTP bật thì trả `mfa_required` qua `challenge::open` y như mật khẩu.
  Phiên ghi `method = "oidc"`. Tài khoản bị khoá → `auth.user_disabled` như cũ.
- `allowed_email_domains` (tuỳ chọn) — email ngoài danh sách bị từ chối.

Endpoint thêm: `GET /api/v1/auth/me/identities`, `POST /api/v1/auth/me/identities/{slug}/start` +
`/callback` (liên kết khi đã đăng nhập; đòi mật khẩu hiện tại hay passkey gần đây như đổi email),
`DELETE /api/v1/auth/me/identities/{id}` (không cho gỡ cách đăng nhập cuối cùng:
`auth.identity_last_method`). Quản trị: `GET/POST /api/v1/auth/oidc-providers`,
`GET/PATCH/DELETE /api/v1/auth/oidc-providers/{id}` — permission mới `auth.oidc.manage`; `client_secret`
chỉ ghi, không bao giờ trả lại (`Cache-Control: no-store` không cần vì không trả; trả `has_secret`).
Tool MCP: `auth.list_oidc_providers` (read-only), `auth.create_oidc_provider`, `auth.update_oidc_provider`,
`auth.delete_oidc_provider` (destructive), `auth.list_my_identities`. Luồng đăng nhập không có tool MCP
(cần trình duyệt; MCP xác thực bằng API key).

Bảng (migration mới của `auth`, phân loại theo bước 34 — provider của khách là bảng cấp khách):

`auth_oidc_provider` (tên số ít theo quy ước bảng của `auth`, D-380): `id` uuid pk, `tenant_id` uuid, `slug` string(64), `kind` string(16), `name`
string(100), `issuer` string(255) null (github: null), `client_id` string(255), `client_secret` text
(mã hoá AEAD + số phiên bản khoá của bước 33), `scopes` string(255) (mặc định `openid email profile`),
`redirect_uris` text (JSON mảng chuỗi), `allowed_email_domains` text null, `allow_signup` bool, `realm` string(16) (`staff` | `customer`, D-309 — sàn cho
người mua đăng nhập Google),
`default_org_id` uuid null, `default_role_id` uuid null, `enabled` bool, `created_at`, `updated_at`.
Unique (`tenant_id`, `slug`).

`auth_identity`: `id` uuid pk, `tenant_id` uuid, `user_id` uuid, `provider_id` uuid, `subject`
string(255), `email` string(320) null, `email_verified` bool, `created_at`, `last_login_at` null.
Unique (`tenant_id`, `provider_id`, `subject`); index (`tenant_id`, `user_id`). Không foreign key (bảng
cấp khách khoá `(tenant_id, id)`): xoá tài khoản / nhà cung cấp xoá liên kết trong cùng transaction. Dữ liệu cá nhân: thêm vào bộ xuất
(`PersonalData`) và xoá cùng tài khoản (`auth.user_deleted`).

Cấu hình: `AUTOXIDE_AUTH_OIDC` (`false` mặc định — tắt thì mọi endpoint OIDC trả `auth.oidc_disabled`
404, provider vẫn quản trị được); `AUTOXIDE_AUTH_OIDC_ALLOW_PRIVATE_NETWORKS` (`false`).
Event: `auth.identity_linked`, `auth.identity_unlinked`, `auth.oidc_provider_created`,
`auth.oidc_provider_updated`, `auth.oidc_provider_deleted`; `auth.signed_in` thêm `after.method = "oidc"`.

### P4 — Tải thẳng lên S3 (sau bước 34)

Trait — mở rộng `Storage` bằng hàm có mặc định (D-038), cài cho `ObjectStorage` của `files`:

```rust
pub struct UploadConstraints { pub size: u64, pub content_type: String, pub sha256: [u8; 32] }
pub struct PresignedUpload { pub method: String /* "PUT" */, pub url: String,
    pub headers: Vec<(String, String)>, pub expires_at: DateTime<Utc> }
pub struct ObjectInfo { pub size: u64 }
#[async_trait::async_trait]
pub trait Storage {   // thêm
    /// `None`: nơi lưu không cấp được link (ổ đĩa) — client tải qua máy chủ như cũ.
    async fn presign_upload(&self, key: &str, c: &UploadConstraints, ttl: Duration)
        -> Result<Option<PresignedUpload>, Error> { Ok(None) }
    async fn head(&self, key: &str) -> Result<Option<ObjectInfo>, Error> { /* mặc định qua get */ }
}
```

S3: `object_store::signer::Signer::signed_url_opts` (object_store 0.14.2 có, `AmazonS3` cài nó) với
`SignedUrlOptions::with_signed_header` cho `content-length`, `content-type`, `x-amz-checksum-sha256`
(base64 của SHA-256) — S3 tự từ chối file sai cỡ, sai loại khai, sai hash. Khoá gồm
`AUTOXIDE_FILES_S3_PREFIX` và tiền tố khách (bước 33); khách `dedicated` ký bằng bucket của khách.

Dùng chung cho `files` và `media`: `autoxide_kernel::upload::DirectUploads` trên bảng
`<module>_uploads` sinh bằng `uploads_migration(name, module)` (như `jobs_migration`):
`id` uuid pk (cũng là ID của bản ghi sẽ tạo — khoá lưu cuối cùng, không phải chép), `tenant_id`,
`org_id` null, `owner_id` uuid, `storage_key` string(512), `name` string(255), `content_type`
string(255), `size` bigint, `sha256` string(64), `status` string(16) (`pending`/`completed`),
`created_at`, `expires_at`, `completed_at` null. Unique (`tenant_id`, `id`); index (`status`,
`expires_at`) cho dọn. Hàm: `start(&C, NewUpload) -> PendingUpload`, `take_for_completion(&C, tenant,
id) -> Option<PendingUpload>` (khoá dòng bằng `rows::lock_one`), `mark_completed`, `expired(limit)`.

Endpoint (mỗi module, dưới tiền tố của nó):
- `POST /api/v1/files/uploads` (`files.upload`) `{name, content_type, size, sha256}` → 201
  `Created` `{id, method, url, headers, expires_at}`; nơi lưu là ổ đĩa → 409
  `files.direct_upload_unavailable` (message: dùng `POST /api/v1/files`). Kiểm cỡ ≤
  `AUTOXIDE_FILES_DIRECT_MAX_BYTES`, loại thuộc `AUTOXIDE_FILES_ALLOWED_TYPES`, tên hợp lệ — gom lỗi
  một lần (D-083). Giữ chỗ hạn mức dung lượng của khách (`Entitlements::reserve`, mã tham chiếu = id).
- `POST /api/v1/files/uploads/{id}/complete` → `head` (cỡ đúng), đọc `get_range(0..HEAD_BYTES)` kiểm
  loại thật (cách của `media/src/library/sniff.rs`), nếu nơi lưu không xác nhận checksum thì đọc luồng băm
  SHA-256; đạt → tạo bản ghi + event `files.file_uploaded` trong một transaction, xác nhận hạn mức;
  không đạt → xoá object, `files.upload_mismatch` (422, nói rõ cỡ / loại / hash nào sai). Gọi lại sau
  khi đã xong → 200 trả cùng bản ghi (agent hay retry; có `Idempotency-Key` càng tốt).
- `media`: `POST /api/v1/media/uploads` (`media.write`, thêm `folder_id`, `tags`) và
  `/uploads/{id}/complete` → tạo item, xếp job bản nhỏ như tải qua máy chủ; loại thật nhận từ nội dung.
- Tool MCP: `files.start_upload`, `files.complete_upload`, `media.start_upload`,
  `media.complete_upload` (agent tự PUT lên URL).
- Job dọn: `files.purge_expired_uploads`, `media.purge_expired_uploads` (lịch 1 giờ): `pending` quá
  `expires_at` + 1 giờ → xoá object, xoá dòng, trả hạn mức. Chạy lại được.
- Cấu hình: `AUTOXIDE_FILES_DIRECT_MAX_BYTES` (mặc định 5368709120 = 5 GiB, trần của một PUT S3),
  `AUTOXIDE_FILES_DIRECT_UPLOAD_SECONDS` (900, 60–3600), `AUTOXIDE_MEDIA_DIRECT_MAX_BYTES`.
- Mã lỗi: `files.direct_upload_unavailable`, `files.upload_not_found`, `files.upload_expired`,
  `files.upload_mismatch`, `files.upload_incomplete` (object chưa có trên S3), và bản `media.*` tương ứng.
- DEPLOY (EN + VI): CORS của bucket cho phép `PUT` từ origin giao diện với ba header đã ký.

### P5 — Event thời gian thực bằng SSE (sau bước 34)

Khác "realtime theo bảng": chỉ stream **event module chủ động khai là stream được**, cùng nội dung và
quyền như đọc qua REST; không đẩy thay đổi dòng. Ghi quyết định nêu khác biệt (skill `record-decision`).

- `EventInfo` thêm `pub const fn streamable(self, permission: Permission) -> Self` (mặc định không
  stream). Module khai, vd `content.entry_published` với `content.read`, `comments.comment_created` với
  `comments.moderate`. Người gọi thấy event khi `principal.has(permission)` **và** event cùng khách
  (`tenant_id`) **và** phạm vi của permission bao event: `Tenant` → mọi; `Subtree` / `Unit` → `org_id`
  của event thuộc nhánh / nút (công cụ lọc nhánh của bước 34); `Own` → `actor_user_id` là người gọi.
  Giả định: bảng `<module>_events` có `tenant_id` (bước 33) và `org_id` null (bước 34; nếu chưa có thì
  bước này thêm `outbox_org_migration` của kernel và `Outbox::record` nhận `org_id`).
- Endpoint `GET /api/v1/kernel/events/stream?names=content.entry_published,comments.*` (cần đăng
  nhập; `names` bắt buộc, `*` chỉ ở sau dấu chấm của tên module; tên không có hay không stream được →
  `kernel.stream_unknown_event` 422). `Last-Event-ID` (header, hay `last_event_id` trong query cho
  client không đặt được header) = id event cuối đã nhận. Response `text/event-stream`: mỗi event
  `id: <uuid>`, `event: <name>`, `data: {"id","name","occurred_at","subject","actor_kind","payload"}`;
  `: ping` mỗi 15 s; `retry: 3000`.
- Đọc từ bảng event (đúng khi nhiều máy chủ): mỗi tiến trình một `StreamHub` hỏi mọi bảng
  `<module>_events` có event stream được, mỗi `AUTOXIDE_EVENTS_STREAM_POLL_MS`, theo
  `(occurred_at, id)` **lùi lại một khoảng `grace`** (5 s) rồi khử trùng bằng tập id đã phát trong
  khoảng đó — vì UUIDv7 / `occurred_at` gán lúc ghi, transaction commit sau có thể mang thời điểm sớm
  hơn event đã phát. Hub phát cho các kết nối qua `tokio::sync::broadcast`; kết nối tụt hậu
  (`Lagged`) hay nối lại với `Last-Event-ID` cũ hơn bộ đệm thì đọc bù thẳng từ database. Event đã
  bị dọn (giữ 7 ngày sau khi giao — `KEEP_DELIVERED`) → gửi `event: reset` để client tải lại trạng thái.
  Giao **ít nhất một lần**: client khử trùng theo `id` (ghi trong mô tả OpenAPI).
- Giới hạn: `AUTOXIDE_EVENTS_STREAM_MAX_CONNECTIONS` (mỗi tiến trình, 1000), `…_PER_USER` (5) — vượt
  → 429 `kernel.stream_too_many`. Bật bằng `AUTOXIDE_EVENTS_STREAM` (`false` mặc định).
- Route stream **không** đi qua lớp timeout (30 s), idempotency, nén (bước 39); khi graceful shutdown
  phải đóng mọi stream để tiến trình thoát trong `AUTOXIDE_HTTP_SHUTDOWN_SECONDS`.
- OpenAPI: response 200 `content_type = "text/event-stream"`, mô tả định dạng; tool MCP tương ứng
  `kernel.list_events` (read-only, `{names, after_id?, limit}`) trả `Page` cùng bộ lọc — agent hỏi
  vòng thay vì giữ stream.

### P6 — Rate limit đúng khi nhiều máy chủ (sau bước 34)

`AUTOXIDE_HTTP_RATE_LIMIT_STORE` = `memory` (mặc định) | `database`. `memory` giữ GCRA hiện có (đã có
khách trong khoá từ bước 33) — đúng hoàn toàn với SQLite vì SQLite chỉ cho một tiến trình máy chủ.
`database` đếm chung qua bảng `kernel_throttle` (`throttle_migration(…, "kernel")`), chạy và có test
trên **cả hai** database, dành cho nhiều máy chủ Postgres.

Để không thêm một câu ghi cho mỗi request: **giữ phần theo lô** — API mới `Throttle::take(key, n) ->
Result<u32, Error>` (một câu `UPDATE … SET count = count + n WHERE … AND count + n <= limit`, hay phần
còn lại nếu ít hơn) — mỗi tiến trình lấy một lô `k = max(1, burst / 4)` lượt cho một khoá rồi tiêu dần
trong bộ nhớ; hết lô mới hỏi lại. Sai số tối đa `k × số máy chủ` mỗi cửa sổ — ghi trong mô tả biến.
Khoá: `api_key:<tenant_id>:<api_key_id>` (giới hạn theo API key, như `ApiKeyLimited`), tuỳ chọn
`ip:<tenant_id>:<nhóm IP>`; giá trị theo khách lấy từ cấu hình theo khách của bước 33 (ghi đè mặc
định toàn máy chủ) — gói (bước 35) đặt được giá trị đó. Dọn dòng cũ bằng `Throttle::purge` trong job
dọn sẵn có của kernel.

Đo: `scripts/loadtest.sh` bài `GET /api/v1/auth/me` bằng API key, `memory` vs `database`, cả hai
database, ghi `docs/BENCHMARKS.md`; giữ `memory` làm mặc định nếu `database` làm p95 tăng quá 10 %
hay request/s giảm quá 10 % — ghi quyết định với số đo.

### Permission · Mã lỗi (bản dịch `locales/vi.json` của crate tương ứng)

Permission mới: `auth.oidc.manage` ("Add, change and remove sign-in providers of the tenant") và `auth.oidc.read`
(xem danh sách, cho tool read-only, D-380).

| `code` | HTTP | Tiếng Việt |
|---|---|---|
| `auth.oidc_disabled` | 404 | "Đăng nhập qua nhà cung cấp bên ngoài chưa được bật trên máy chủ này." |
| `auth.oidc_provider_not_found` | 404 | "Không có nhà cung cấp đăng nhập `{slug}`. Xem danh sách ở `GET /api/v1/auth/oidc/providers`." |
| `auth.oidc_redirect_not_allowed` | 422 | "Địa chỉ quay về `{redirect_uri}` không có trong danh sách của nhà cung cấp này." |
| `auth.oidc_state_invalid` | 400 | "Lượt đăng nhập đã hết hạn hoặc đã dùng. Hãy bắt đầu lại." |
| `auth.oidc_token_invalid` | 401 | "Nhà cung cấp trả về thông tin đăng nhập không hợp lệ ({reason}). Hãy thử lại." |
| `auth.oidc_provider_unavailable` | 503 (D-380: `ErrorKind` không có 502) | "Không liên lạc được với nhà cung cấp đăng nhập. Hãy thử lại sau." |
| `auth.oidc_email_unverified` | 403 | "Nhà cung cấp chưa xác minh email của bạn, nên không dùng được để đăng nhập." |
| `auth.oidc_email_domain_not_allowed` | 403 | "Email thuộc tên miền `{domain}` không được đăng nhập bằng nhà cung cấp này." |
| `auth.oidc_link_required` | 409 | "Đã có tài khoản với email này. Hãy đăng nhập bằng cách cũ rồi liên kết nhà cung cấp trong phần tài khoản." |
| `auth.oidc_signup_closed` | 403 | "Chưa có tài khoản cho danh tính này và không được tự đăng ký." |
| `auth.oidc_slug_taken` | 409 | "Đã có nhà cung cấp mang tên `{slug}`." |
| `auth.identity_not_found` | 404 | "Bạn không có liên kết đăng nhập mang id `{id}`." |
| `auth.identity_taken` | 409 | "Danh tính đăng nhập này đã liên kết với tài khoản khác. Hãy gỡ ở tài khoản đó trước." |
| `auth.identity_last_method` | 409 | "Không gỡ được: đây là cách đăng nhập cuối cùng của bạn. Hãy đặt mật khẩu hoặc thêm passkey trước." |
| `files.direct_upload_unavailable` | 409 | "Nơi lưu hiện tại không nhận tải thẳng. Hãy tải lên qua `POST /api/v1/files`." |
| `files.upload_not_found` | 404 | "Không có lượt tải lên mang id `{id}`." |
| `files.upload_expired` | 410 | "Lượt tải lên đã hết hạn. Hãy bắt đầu lượt mới." |
| `files.upload_incomplete` | 409 | "Chưa thấy file trên nơi lưu. Hãy PUT file lên link đã cấp rồi gọi lại." |
| `files.upload_mismatch` | 422 | "File đã tải lên không khớp khai báo: {detail}. File đã bị xoá." |
| (`media.*` tương ứng) | | cùng câu, thay `files` bằng `media` |
| `kernel.stream_unknown_event` | 422 | "Event `{name}` không có hoặc không stream được. Xem danh sách event ở sổ module." |
| `kernel.stream_too_many` | 429 | "Bạn đang mở quá nhiều kết nối stream ({limit}). Hãy đóng bớt." |
| `kernel.stream_disabled` | 404 | "Stream event chưa được bật trên máy chủ này." |

## Thứ tự làm

Commit theo thứ tự; `./gate.sh --quick` trước mỗi commit. Mục ROADMAP → phần: (1) OIDC → P3,
(2) tải thẳng → P4, (3) header → P1, (4) SSE → P5, (5) MCP → P2, (6) rate limit → P6.

**Làm ngay (hai PR, song song được với 35–39):**
1. P1: `security_headers.rs` + cấu hình + doctor; `.env.example`; test.
2. P1: bỏ header tự đặt ở `media` / `files`, test so `API_CSP`; kiểm trang `/api/docs` trong trình
   duyệt; DEPLOY (EN + VI) mục header / HSTS.
3. P2: `McpResource`, `Prompt`, `Module::{mcp_resources, prompts, description}`, kiểm khai báo trong
   `Modules`, sổ module.
4. P2: `/mcp` resources + prompts; resource của kernel; resource + prompt của `content`, `comments`;
   `description()` cho 13 module.
5. P2: `/llms.txt` + resource `autoxide://kernel/llms.txt`; README / MODULE_GUIDE (EN + VI); sinh lại
   `docs/openapi.json`.

**Sau bước 34:**
6. Dời `webhooks/src/network.rs` → `autoxide_kernel::net`; webhooks dùng lại (test sẵn có xanh).
7. P3: bảng, provider CRUD + tool MCP + permission; 8. P3: luồng start / callback, JWT, JWKS, GitHub;
   9. P3: liên kết / gỡ, xuất / xoá dữ liệu cá nhân, event; DEPLOY + README (EN + VI) cách khai Google,
   Microsoft, GitHub.
10. P4: `Storage::{presign_upload, head}` cho S3; `DirectUploads` + `uploads_migration`; 11. P4:
    endpoint + tool + job dọn của `files`; 12. P4: của `media`; DEPLOY: CORS bucket.
13. P5: `EventInfo::streamable`, `StreamHub`, endpoint, tool `kernel.list_events`, khai stream cho
    event của `content`, `comments`, `media`; ghi quyết định SSE vs realtime theo bảng.
14. P6: `Throttle::take`, store `database`, đo trước / sau, quyết định.
15. CHANGELOG, đánh dấu ROADMAP, `./gate.sh` đầy đủ, phát hành theo `docs/RELEASING.md`.

## Test bắt buộc

Mọi test chạm database qua `TestDatabase::run` (SQLite và Postgres), qua REST / MCP bằng `TestApp`.

- P1: `moi_response_co_header_bao_mat` (200 JSON, 404 router, 429, OPTIONS preflight, `/health/live`,
  `/mcp` 401); `handler_dat_csp_rieng_thi_giu`; `trang_tai_lieu_co_csp_rieng`;
  `hsts_chi_khi_cau_hinh`; `tat_header_bang_cau_hinh`; `cau_hinh_referrer_policy_sai_bi_tu_choi`.
- P2: `resources_list_loc_theo_permission`, `doc_resource_khong_duoc_phep_giong_khong_ton_tai`,
  `prompt_an_khi_thieu_permission_cua_tool`, `prompt_chen_doi_so_trong_ranh_gioi`,
  `khai_bao_prompt_tro_toi_tool_khong_co_bi_tu_choi_luc_khoi_dong`, `schema_resource_khop_openapi`
  (JSON Schema của resource bằng schema cùng tên trong `openapi.json`), `llms_txt_co_moi_module_va_link`,
  `moi_link_trong_llms_txt_khong_404` (gọi từng đường dẫn: 200 / 401, không 404).
- P3 (máy chủ OIDC giả chạy trong test bằng axum trên `127.0.0.1`: discovery, JWKS, token endpoint, API
  kiểu GitHub; ký `id_token` bằng ES256 khoá sinh lúc chạy — **không** để khoá riêng nào trong repo, gitleaks
  sẽ chặn; RS256 kiểm bằng ví dụ công khai RFC 7515 phụ lục A.2 / RFC 7517, chỉ khoá công khai):
  `dang_nhap_lan_dau_tao_tai_khoan_khi_cho_phep`, `lien_ket_theo_email_da_xac_minh`,
  `khong_gop_khi_email_chua_xac_minh` (cả hai phía), `microsoft_khong_tin_claim_email_khi_thieu_xms_edov`,
  `state_dung_mot_lan` (hai callback đồng thời: đúng một thành công, cả hai database),
  `nonce_sai_bi_tu_choi`, `alg_none_va_hs256_bi_tu_choi`, `aud_iss_exp_sai_bi_tu_choi`, `kid_la_thi_tai_lai_jwks_mot_lan`,
  `mfa_van_ap_dung`, `redirect_uri_ngoai_danh_sach`, `issuer_mang_noi_bo_bi_chan`,
  `provider_cua_khach_a_khong_dung_duoc_o_khach_b` (rò rỉ giữa khách), `go_cach_dang_nhap_cuoi_bi_chan`,
  `xuat_va_xoa_identity_cung_tai_khoan`.
- P4 (chạy trên S3 giả của gate qua `testkit::s3_settings`; không có `TEST_S3_ENDPOINT` thì chỉ chạy
  phần ổ đĩa): `o_dia_khong_cap_link`, `tai_thang_roi_xac_nhan_tao_ban_ghi`, `sai_co_hay_hash_bi_s3_tu_choi`
  (nếu moto không kiểm checksum thì bài này kiểm bước xác nhận băm lại), `loai_that_khac_khai_bao_bi_xoa`,
  `xac_nhan_goi_lai_tra_cung_ban_ghi`, `xac_nhan_dong_thoi_chi_mot_ban_ghi` (cả hai database),
  `luot_het_han_duoc_don_va_tra_han_muc`, `khoa_luu_co_tien_to_khach`, `khach_b_khong_xac_nhan_duoc_luot_cua_khach_a`.
- P5: `stream_nhan_event_duoc_phep`, `khong_nhan_event_khong_co_quyen`, `khong_nhan_event_cua_khach_khac`,
  `khong_nhan_event_cua_nhanh_anh_em` (phạm vi `Subtree`), `noi_lai_bang_last_event_id_khong_mat_event`,
  `event_commit_muon_van_toi` (transaction mở trước, commit sau event khác — vẫn nhận), `hai_may_chu_chung_database`
  (ghi qua `TestApp` A, đọc stream ở `TestApp` B), `qua_gioi_han_ket_noi_429`, `stream_khong_bi_timeout_30s`
  (đặt timeout thử ngắn), `tool_list_events_cung_bo_loc`.
- P6: `hai_may_chu_chung_han_muc_api_key` (hai `TestApp` cùng database, store `database`: tổng request
  vượt `limit + k × 2` thì 429), `take_dong_thoi_khong_vuot_limit` (test đồng thời, cả hai database),
  `khoa_co_khach_khong_tinh_chung_giua_khach`, `memory_van_la_mac_dinh`.

## Tiêu chí xong

- [ ] Sáu mục `- [ ]` của bước 40 trong ROADMAP xong (ánh xạ ở "Thứ tự làm").
- [ ] Mọi response có header bảo mật; trang tài liệu chạy được với CSP (đã mở trong trình duyệt).
- [ ] MCP liệt kê / đọc resources, prompts theo permission; `/llms.txt` sinh từ code, test giữ khớp.
- [ ] OIDC: bốn kiểu nhà cung cấp, PKCE, `state` / `nonce`, chữ ký qua JWKS, liên kết an toàn, MFA, tắt
      mặc định; SSRF chặn; dữ liệu cá nhân xuất / xoá được.
- [ ] Tải thẳng S3 với cỡ / loại / hash ràng buộc, xác nhận, dọn bằng job; ổ đĩa đi qua máy chủ như cũ.
- [ ] SSE: lọc theo permission, khách, nhánh; nối lại không mất event trong khoảng giữ; nhiều máy chủ đúng.
- [ ] Rate limit `database` đúng khi nhiều máy chủ, số đo trước / sau trong BENCHMARKS, quyết định ghi.
- [ ] Mọi endpoint mới có `///` summary, `description` cho mỗi response; `docs/openapi.json`,
      `.env.example` sinh lại; mọi `code` mới có bản dịch (xtask chặn).
- [ ] Quyết định ghi: header (giá trị CSP), MCP resources / prompts / llms.txt, JWT OIDC tự viết bằng
      `ring` + `p256`, thiết kế tải thẳng, SSE khác realtime theo bảng, rate limit theo lô. `STACK.md`
      nếu thêm thư viện hay feature (vd `ring` thành phụ thuộc trực tiếp).
- [ ] README, DEPLOY, MODULE_GUIDE: bản EN và VI cùng nội dung, sửa trong cùng commit.
- [ ] CHANGELOG; ROADMAP đánh dấu; `./gate.sh` đầy đủ xanh; phát hành theo `docs/RELEASING.md`.

## Chỗ dễ sai

- **Thứ tự lớp**: lớp header đặt trong cùng (gần handler) thì 404 của router, 429 của rate limit và
  lỗi CORS không có header. Đặt ngoài `log`, trong `request_id`.
- **Ghi đè CSP của handler**: dùng "chỉ đặt khi chưa có" — trang tài liệu và `media` dựa vào đó.
- **Scalar trên CDN không ghim phiên bản** (`@scalar/api-reference` không có số) — CSP chỉ giới hạn
  nguồn, không giới hạn phiên bản; ghi điều này trong DEPLOY ("bật `/api/docs` chỉ khi cần").
- **MCP `$ref`**: JSON Schema của resource schema có thể lồng `$ref` — resource là tài liệu nên
  được, nhưng nhúng định nghĩa phụ thuộc (gom `components`) để agent đọc được một mình.
- **Lộ sự tồn tại**: resource / prompt không được phép phải trả lỗi giống hệt cái không có.
- **OIDC**: so `state` bằng tra băm (OneTimeTokens), không so chuỗi thường; `code_verifier` không bao
  giờ ra khỏi máy chủ; `redirect_uri` so khớp đúng từng ký tự (không so tiền tố); JWKS tải lại vô hạn
  khi kẻ xấu gửi `kid` ngẫu nhiên → giới hạn mỗi phút; lệch giờ; Microsoft `common` / `organizations`
  có `iss` khác nhau mỗi tenant; GitHub không có `id_token` và email chính có thể chưa xác minh; gọi
  nhà cung cấp **không** nằm trong transaction ghi (luật dữ liệu) — đổi code lấy token xong mới mở
  transaction tạo liên kết.
- **Chiếm tài khoản dựng sẵn**: không bao giờ gộp khi một trong hai phía chưa xác minh email.
- **Tải thẳng**: presigned PUT không giới hạn cỡ nếu không ký `content-length` — phải ký; S3 một PUT tối
  đa 5 GiB; trình duyệt cần CORS của bucket; object lên S3 mà không gọi `complete` → job dọn; hai lần
  `complete` đồng thời → khoá dòng `lock_one`, không tạo hai bản ghi; hash S3 là base64 còn cột là hex.
- **SSE**: lớp timeout 30 s cắt stream; graceful shutdown chờ stream mãi; proxy (Nginx) đệm SSE — đặt
  `X-Accel-Buffering: no` và `Cache-Control: no-cache`; khoảng `grace` quá ngắn thì mất event commit
  muộn, quá dài thì trễ — đo; mỗi kết nối KHÔNG tự hỏi database (dùng hub chung), không thì N kết nối
  là N câu mỗi giây; payload event có thể chứa dữ liệu cá nhân — chỉ event module khai `streamable`.
- **Rate limit `database` trên SQLite**: mỗi lô là một lệnh ghi trên kết nối ghi duy nhất — vẫn đúng,
  nhưng không có lý do bật (một tiến trình); `doctor` cảnh báo khi SQLite + `database`.
- **`tenant_id` trong khoá đếm** (bước 33): quên thì khách A dùng hết lượt của khách B.

## Căn cứ pháp lý

- Luật BVDLCN 91/2025/QH15 + NĐ 356/2025 (hiệu lực 01/01/2026): danh tính từ nhà cung cấp (email, tên,
  subject) là dữ liệu cá nhân — chỉ lấy scope cần (`openid email profile`), xuất được và xoá cùng tài
  khoản; payload event qua SSE chịu cùng quyền như REST. Cần đối chiếu văn bản gốc về sổ đồng ý khi
  liên kết tài khoản (bước 37 có sổ đồng ý).
- NĐ 147/2024/NĐ-CP (xác thực tài khoản đăng nội dung bằng số điện thoại Việt Nam): đăng nhập qua
  Google / GitHub **không** thay xác thực đó — module cần xác thực (vd `comments` cho người dùng mạng
  xã hội) vẫn đòi bước của bước 37. Cần đối chiếu phạm vi áp dụng (D-186).

## Agent làm bước này tự quyết

- Giá trị CSP cuối cùng của `/api/docs` (theo kiểm trong trình duyệt) — ghi DECISIONS.
- `ring` hay `rsa` để kiểm RS256 (khuyến nghị `ring`: đã có, không mở rộng ngoại lệ RUSTSEC) — tra
  `check-library-api`, ghi DECISIONS và `STACK.md`.
- Tên kiểu schema thật cho `autoxide://content/schema/entry` và danh sách event khai `streamable` ban đầu.
- `k` của lô rate limit, ngưỡng giữ `memory` mặc định, `grace` và chu kỳ hỏi của SSE — theo số đo.
- Có thêm `/llms-full.txt` (gộp hướng dẫn các module) hay không — khuyến nghị chưa, tới khi có module
  khai tài liệu riêng cho agent.

## Bổ sung theo D-220

### P7 — Bộ máy tìm kiếm thay được (sau bước 33)

`autoxide_kernel::search` hiện là "chứa chuỗi" trên cột khoá tìm (`LIKE`), tài liệu ghi chỉ đủ cho vài trăm
nghìn dòng. Thêm trait để sàn có hàng triệu sản phẩm cắm bộ tìm chuyên dụng mà module không phải sửa:

```rust
#[async_trait] pub trait SearchBackend: Send + Sync {          // autoxide_kernel::search (D-381)
    async fn index(&self, tenant: &TenantContext, docs: &[SearchDoc]) -> Result<(), Error>;   // upsert
    async fn remove(&self, tenant: &TenantContext, kind: &str, ids: &[Uuid]) -> Result<(), Error>;
    async fn clear(&self, tenant: &TenantContext, kind: &str) -> Result<(), Error>;            // trước khi dựng lại
    async fn query(&self, tenant: &TenantContext, kind: &SearchKind, request: &SearchRequest)
        -> Result<SearchHits, Error>;                                                           // Page<SearchHit>
}
pub struct SearchDoc { pub kind: String, pub id: Uuid, pub org_id: OrgId, pub locale: String,
    pub fields: BTreeMap<String, String>, pub filters: BTreeMap<String, Value> }
pub struct SearchRequest { pub kind: String, pub text: String, pub filters: BTreeMap<String, String>,
    pub locale: Option<String>, pub scope: Option<OrgScope>, pub page: PageRequest }
```

Tên `SearchRequest` (không `SearchQuery`: tên đó đã là truy vấn đã phân tích của tìm xuyên phân hệ bước 51,
`autoxide_kernel::find`). Một truy vấn một loại; `OrgScope` (`subtrees`, `units`, `fence`) do `SearchIndex::search`
dựng từ grant của người gọi cho permission của loại (D-381).

- **Mặc định `DatabaseSearch`** (trong kernel): bảng `kernel_search_docs` (một dòng mỗi tài liệu, khoá
  `(tenant_id, kind, id)`, cột `primary_exact` / `primary_fold` cho trường trọng số cao nhất và `exact_key` /
  `fold_key` cho mọi trường như D-202) và `kernel_search_filters` (một dòng mỗi bộ lọc, index `(tenant_id, kind,
  name, value)` — bộ lọc là cột có index mà không đổi schema theo module); bốn tầng (`fetch_page_tiers`): tiêu đề
  đúng dấu, nội dung đúng dấu, tiêu đề không dấu, nội dung không dấu — chạy như nhau trên hai database.
- **Bộ cắm ngoài** (Meilisearch, Typesense, OpenSearch) là module riêng làm khi có nhu cầu đo được, đăng ký
  bằng `context.search_backend(name, …)`; chọn bằng `AUTOXIDE_SEARCH_BACKEND` (`database` mặc định). Gọi mạng
  ngoài transaction: trong transaction của thay đổi, module gọi `SearchIndex::enqueue` (xếp job
  `kernel.search_sync` mang loại + ID, cùng transaction như outbox); job đọc lại tài liệu qua
  `SearchSource::load` — còn thì `index`, mất thì `remove` — chịu nhận lại.
- Module khai loại tài liệu tìm được bằng `context.search_documents(SearchKind, Arc<dyn SearchSource>)` (loại,
  trường + trọng số, bộ lọc, permission đọc; nguồn `load` / `scan` từ bảng của module) — như `finder_source`, vì
  nguồn cần trạng thái có sau `start` (D-381, thay `Module::search_documents`). Tìm xuyên phân hệ bước 51 và ô tìm
  của module giữ nguyên trên bảng của module (không phá API); module chuyển sang chỉ mục khi bảng lớn (catalog
  bước 41). Kết quả luôn lọc theo khách và phạm vi nút **trong** backend (bộ lọc `org_id`), không lọc sau.
- `autoxide task kernel.reindex [kind=…] [tenant=…]` dựng lại chỉ mục. Đo trên bộ dữ liệu lớn của bước 39 trước
  khi khuyên dùng backend ngoài — khi module đầu tiên dùng chỉ mục (D-381); ghi số đo vào `docs/BENCHMARKS.md`.
- Test (cả hai database): `tim_khong_dau_dung_dau_xep_truoc`, `ket_qua_khong_lo_sang_khach_khac`,
  `ket_qua_loc_theo_pham_vi_nut`, `xoa_ban_ghi_thi_bien_khoi_ket_qua`, `dong_bo_chiu_nhan_lai_event`,
  `backend_gia_thay_duoc_khong_sua_module`.
