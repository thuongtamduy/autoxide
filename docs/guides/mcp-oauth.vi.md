# Kết nối client MCP bằng đăng nhập OAuth

Tiếng Anh: [`mcp-oauth.md`](mcp-oauth.md).

Khi bật đăng nhập OAuth, người dùng thêm máy chủ của bạn vào Claude, ChatGPT, IDE hay bất kỳ client MCP nào chỉ bằng địa chỉ. Client mở trình duyệt, người dùng đăng nhập vào **giao diện của bạn** và chọn client được làm gì, rồi client nhận token không bao giờ mang nhiều quyền hơn thế. Không ai phải chép hay dán key. Không bật OAuth thì client vẫn kết nối bằng API key ([Nối AI agent qua MCP](mcp.vi.md)).

Autoxide theo [đặc tả uỷ quyền MCP](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization) (2026-07-28): OAuth 2.1 với PKCE, Protected Resource Metadata (RFC 9728), Authorization Server Metadata (RFC 8414), resource indicator (RFC 8707), tài liệu metadata của client và, cho client cũ, đăng ký client động (RFC 7591).

## 1. Bật

Đăng nhập OAuth bật khi đặt cả hai biến:

| Biến | Giá trị |
|---|---|
| `AUTOXIDE_HTTP_PUBLIC_URL` | Địa chỉ client dùng để tới máy chủ, **không có path**, vd `https://api.example.com`. Đây là issuer của OAuth. Có path (`https://example.com/api`) thì OAuth vẫn tắt và log nói lý do. |
| `AUTOXIDE_AUTH_OAUTH_CONSENT_URL` | Trang trong giao diện của bạn nơi người đã đăng nhập duyệt client, có `{token}` ở chỗ đặt token của yêu cầu, vd `https://app.example.com/connect?request={token}`. |

Tuỳ chọn:

| Biến | Mặc định | Nghĩa |
|---|---|---|
| `AUTOXIDE_AUTH_OAUTH_DYNAMIC_REGISTRATION` | `true` | Cho client tự đăng ký ở `POST /api/v1/auth/oauth/register`. MCP 2026-07-28 đã khai deprecated cách này, thay bằng tài liệu metadata của client; giữ bật khi client bạn cần vẫn dùng nó. |
| `AUTOXIDE_AUTH_OAUTH_GRANT_DAYS` | `90` | Một lần duyệt sống tối đa bao lâu (1–365 ngày). Người duyệt có thể chọn ít hơn. Hết hạn thì client xin lại. |
| `AUTOXIDE_AUTH_OAUTH_ALLOW_PRIVATE_NETWORKS` | `false` | Tải tài liệu metadata của client từ địa chỉ mạng riêng và qua `http://`. **Chỉ cho test**: ai cũng đặt được tên client, nên ở production biến này cho họ khiến máy chủ gọi vào mạng nội bộ của bạn. |

Kiểm bằng `autoxide doctor` (dòng `mcp_oauth` nêu issuer, hoặc lý do OAuth tắt), rồi:

```bash
curl -sS https://api.example.com/.well-known/oauth-protected-resource/mcp
curl -sS https://api.example.com/.well-known/oauth-authorization-server
```

Lệnh đầu cho biết resource (`https://api.example.com/mcp`) và authorization server của nó; lệnh sau liệt kê các endpoint (`/api/v1/auth/oauth/authorize`, `/token`, `/revoke`, `/register`). Khi OAuth tắt, cả hai trả `404` với mã `oauth_unavailable`.

Client chạy trên cloud của bên khác (Claude và ChatGPT bản web) tới máy chủ của bạn qua internet: `AUTOXIDE_HTTP_PUBLIC_URL` phải là địa chỉ `https://` họ mở được. Client trên cùng máy (IDE, Claude Code) dùng được địa chỉ cục bộ.

## 2. Chuyện gì xảy ra khi client kết nối

1. Client gọi `/mcp` không kèm token, nhận `401` với `WWW-Authenticate: Bearer resource_metadata="…/.well-known/oauth-protected-resource/mcp"`. Từ đó nó tìm ra authorization server.
2. Client tự giới thiệu: bằng **tài liệu metadata của client** (`client_id` là một URL `https://` tới tài liệu JSON mà máy chủ tải về), hoặc đăng ký ở `/register`.
3. Client mở trình duyệt tới `/api/v1/auth/oauth/authorize` với PKCE (`S256`). Máy chủ kiểm mọi tham số rồi chuyển trình duyệt tới trang consent của bạn kèm token của yêu cầu. Tham số sai thì trả `400` ngay tại chỗ, không bao giờ chuyển hướng về client.
4. Ở trang consent, người dùng đăng nhập (nếu cần), thấy ai đang xin và chọn permission. Trang duyệt hay từ chối qua API, và trình duyệt quay về client kèm mã.
5. Client đổi mã lấy token ở `/token` rồi gọi `/mcp` với `Authorization: Bearer axo1_…`.

Token:

- Access token (`axo1_…`) sống ngắn; refresh token (`axr1_…`) đổi được một cặp mới đúng một lần rồi hết dùng. Client phải làm mới **tuần tự**: gửi lại refresh token cũ (hay hai lượt làm mới cùng lúc) bị coi là bị đánh cắp và lần duyệt bị thu hồi.
- Client xin cho `/mcp` dùng được cả `/mcp/<module>`. REST ở `/api/v1` là resource riêng, client phải xin riêng (`resource=https://api.example.com/api/v1`).
- Client làm thay người dùng, loại người gọi `agent`, không bao giờ vượt quá permission đã duyệt **và** quyền hiện tại của người đó. Người dùng mất một permission thì client cũng mất.
- `tools/list` chỉ hiện tool mà lần duyệt cho phép. Gọi thứ người dùng có quyền nhưng chưa duyệt cho client thì nhận `403` với `WWW-Authenticate: Bearer error="insufficient_scope", scope="…"`, để client xin thêm.

Chỉ tài khoản nhân sự được duyệt client (khách mua hàng thì không). Lần duyệt thuộc về khách (tenant) mà người dùng đang đăng nhập ở trang consent.

## 3. Trang consent của giao diện

Autoxide không có trang web riêng: trang consent nằm trong giao diện của bạn, ở địa chỉ `AUTOXIDE_AUTH_OAUTH_CONSENT_URL`. Trang cần **phiên đăng nhập của chính người dùng** (mật khẩu, passkey, đăng nhập một lần…). API key hay token OAuth không đọc hay quyết được yêu cầu (`403 auth.session_required`), nên client không bao giờ tự duyệt cho mình được.

1. **Giữ token của yêu cầu.** Đọc `request` trên địa chỉ. Người dùng chưa đăng nhập thì cho đăng nhập trước rồi quay lại kèm token. Yêu cầu chờ 10 phút.
2. **Hiện yêu cầu**: `GET /api/v1/auth/oauth/requests/{token}`.

   ```json
   {
     "data": {
       "client_id": "https://client.example.com/oauth/metadata.json",
       "client_host": "client.example.com",
       "client_name": "Example assistant",
       "registration": "cimd",
       "redirect_uri": "https://client.example.com/oauth/callback",
       "redirect_host": "client.example.com",
       "localhost_redirect": false,
       "resource": "https://api.example.com/mcp",
       "requested_scope": ["content.*"],
       "permissions": [
         {"name": "content.read", "description": "…", "risk": "read", "requested": true},
         {"name": "content.write_own", "description": "…", "risk": "write", "requested": true}
       ],
       "current_permissions": null,
       "expires_at": "2026-11-27T10:10:00Z"
     }
   }
   ```

   - Hiện rõ `client_host` và `redirect_host`. `client_name` do client tự đặt, chưa được kiểm: hiện cạnh host, không bao giờ hiện một mình.
   - `registration` là `cimd` (tài liệu metadata: host đó thật sự công bố nó) hay `dynamic` (tự đăng ký: ai cũng chọn được tên đó).
   - `localhost_redirect` là `true` thì cảnh báo: mã quay về một chương trình trên chính thiết bị này, và chương trình nào ở đó cũng giả làm client này được.
   - `permissions` là các permission người dùng đang có, thu hẹp theo thứ client xin. Chọn sẵn những cái `requested: true`; cho người dùng bỏ chọn tuỳ ý. Mẫu như `content.*` chỉ là lời xin: lần duyệt lưu đúng danh sách tên đã chọn, nên permission thêm vào máy chủ sau này không tự lọt vào lần duyệt cũ.
   - `current_permissions` là lần duyệt hiện có của người dùng cho client này với resource này, nếu có.
3. **Duyệt**: `POST /api/v1/auth/oauth/requests/{token}/approve` với

   ```json
   {"permissions": ["content.read", "content.write_own"], "days": 30}
   ```

   `days` không bắt buộc (mặc định và tối đa: `AUTOXIDE_AUTH_OAUTH_GRANT_DAYS`). Permission có `risk` là `destructive`, `sensitive_data` hay `money` cần thêm `"reauth_token"` từ `POST /api/v1/auth/me/reauth` (người dùng xác nhận bằng mật khẩu, passkey hay mã); thiếu thì trả `422` và yêu cầu vẫn chờ.
4. **Hoặc từ chối**: `POST /api/v1/auth/oauth/requests/{token}/deny`.
5. **Chuyển trình duyệt đi tiếp**: cả hai trả `{"data": {"redirect_to": "…"}}`. Đặt `window.location` bằng giá trị đó: client nhận mã (hay `error=access_denied`) cùng `state` của nó và issuer `iss`.

## 4. Kết nối các client

Địa chỉ đưa cho mọi client là `https://api.example.com/mcp` (hay `/mcp/<module>` để chỉ có tool của một module). Tên menu thay đổi theo phiên bản; tài liệu của chính client là chuẩn.

- **Claude** (claude.ai, ứng dụng máy tính và điện thoại): thêm custom connector bằng địa chỉ máy chủ. Claude mở trang consent của bạn trong trình duyệt; duyệt xong, tool hiện trong các cuộc trò chuyện bật connector đó. Claude kết nối qua internet, nên địa chỉ phải là `https://` công khai.
- **Claude Code**: `claude mcp add --transport http autoxide https://api.example.com/mcp`, rồi chạy `/mcp` trong một phiên và chọn xác thực; trình duyệt mở trên máy này.
- **ChatGPT**: trong phần cài đặt connector / app (bật developer mode nếu cần), tạo connector với địa chỉ máy chủ và xác thực OAuth. ChatGPT cũng kết nối qua internet.
- **Client khác** (IDE, framework agent, [SDK MCP](https://modelcontextprotocol.io/docs/sdk)): client nào cài đặt uỷ quyền MCP cũng làm y như vậy: đưa địa chỉ và để nó mở trình duyệt. Client giữ redirect trên thiết bị dùng `http://127.0.0.1:<cổng bất kỳ>/…`; máy chủ chấp nhận mọi cổng với `127.0.0.1` và `[::1]`, nhưng không với tên `localhost` (RFC 8252).

Client không làm được OAuth vẫn kết nối bằng API key trong header `Authorization` ([Nối AI agent qua MCP](mcp.vi.md)).

## 5. Xem và thu hồi lần duyệt

- Người dùng: `GET /api/v1/auth/me/oauth-grants` liệt kê các lần duyệt của mình (mới nhất trước, kể cả đã thu hồi và hết hạn); `DELETE /api/v1/auth/me/oauth-grants/{id}` thu hồi một lần. Token ngừng chạy từ request kế tiếp.
- Quản trị có `auth.oauth_grants.manage`: `GET /api/v1/auth/oauth-grants` và `DELETE /api/v1/auth/oauth-grants/{id}` cho mọi người trong khách. Thu hồi (cả hai endpoint) cần phiên đăng nhập của chính người đó, làm trực tiếp trong khách của mình: API key hay token OAuth nhận `403 auth.session_required`.
- Chính client: `POST /api/v1/auth/oauth/revoke` (RFC 7009) với access token hay refresh token thu hồi cả lần duyệt.
- Tự động: khoá, vô hiệu hay xoá tài khoản, lấy lại tài khoản, đổi mật khẩu kèm đăng xuất các phiên khác, và "đăng xuất khắp nơi" thu hồi mọi lần duyệt của người đó. Luật tự động tạo qua một client tạm dừng khi lần duyệt của client đó bị thu hồi.

Mỗi lần duyệt và thu hồi là một event trong audit log (`auth.oauth_granted`, `auth.oauth_revoked`), và mọi thay đổi làm qua client ghi lại client nào làm thay ai.

## 6. Lỗi thường gặp

| Hiện tượng | Nguyên nhân và cách sửa |
|---|---|
| Client báo máy chủ không có uỷ quyền / `/.well-known/…` trả `404 oauth_unavailable` | OAuth tắt: đặt `AUTOXIDE_HTTP_PUBLIC_URL` (không path) và `AUTOXIDE_AUTH_OAUTH_CONSENT_URL`, khởi động lại, kiểm bằng `autoxide doctor`. Endpoint của API trả `auth.oauth_disabled`. |
| Trình duyệt hiện `400` với `auth.oauth_client_invalid` | Không tải được tài liệu metadata của client hoặc tài liệu không hợp lệ (phải là `https`, cổng 443, địa chỉ công khai, tối đa 5 KB, `client_id` trùng URL của chính nó, không có client secret), hoặc đăng ký động đang tắt mà client cần nó. Thông báo nêu chi tiết. |
| `400 auth.oauth_redirect_invalid` | `redirect_uri` của client không nằm trong danh sách nó đã đăng ký. Chỉ `127.0.0.1` / `[::1]` được linh hoạt cổng. |
| `400 auth.oauth_request_invalid` | Thiếu hay sai tham số (`code_challenge_method` phải là `S256`, `resource` phải là `…/mcp`, `…/mcp/<module>` hay `…/api/v1` của máy chủ này). |
| Trang consent nhận `404 auth.oauth_request_not_found` | Đã quá 10 phút, hoặc yêu cầu đã được quyết. Kết nối lại từ client. |
| Trang consent nhận `403 auth.session_required` / `auth.oauth_staff_only` | Gọi bằng phiên đăng nhập của chính người dùng, không bằng API key; chỉ tài khoản nhân sự được duyệt client. |
| `422` khi duyệt | Một permission không nằm trong danh sách được mời, `days` nằm ngoài khoảng 1 tới `AUTOXIDE_AUTH_OAUTH_GRANT_DAYS`, hoặc permission nhạy cảm cần `reauth_token`. Yêu cầu vẫn chờ. |
| Client bị đăng xuất sau một lúc, `/token` trả `invalid_grant` | Lần duyệt hết hạn hay bị thu hồi, người dùng không còn đăng nhập được, hoặc client làm mới hai lần cùng lúc (refresh token dùng lại làm thu hồi lần duyệt). Kết nối lại. |
| Tool trả `403 insufficient_scope` | Người dùng có permission nhưng chưa duyệt cho client này. Kết nối lại và duyệt thêm. |
| `auth.oauth_rate_limited` | Quá nhiều yêu cầu uỷ quyền từ một địa chỉ; đợi một phút. |
