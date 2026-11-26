# Đăng nhập bằng Google, Microsoft, GitHub hay OpenID Connect

English: [`oidc.md`](oidc.md). Ví dụ dùng `curl` với `API` là địa chỉ máy chủ của bạn (`export API=https://api.example.com`). Các ví dụ chạy như test ở mỗi bản build, nên khớp với máy chủ của phiên bản này.

Người dùng có thể đăng nhập bằng một nhà cung cấp danh tính bên ngoài thay cho mật khẩu: Google, Microsoft (Entra ID), GitHub hay một issuer OpenID Connect bất kỳ. Mỗi khách tự khai nhà cung cấp của mình. Autoxide chỉ là *bên dùng* của nhà cung cấp; giao diện của bạn điều khiển trình duyệt.

Tính năng **tắt mặc định**. Người vận hành máy chủ bật bằng `AUTOXIDE_AUTH_OIDC=true`. Lúc tắt vẫn quản lý được nhà cung cấp; đăng nhập thì trả `404 auth.oidc_disabled`.

## 1. Đăng ký ứng dụng ở nhà cung cấp

Đăng ký một ứng dụng web ở nhà cung cấp, ghi lại client id và client secret. Khai trang callback của giao diện làm địa chỉ quay về (redirect), vd `https://app.example.com/sign-in/callback`.

- **Google** — Google Cloud console, *APIs & Services → Credentials → OAuth client ID* (loại *Web application*). Scope `openid email profile`.
- **Microsoft** — Microsoft Entra admin center, *App registrations → New registration*, nền tảng *Web*. Tạo client secret ở *Certificates & secrets*. Muốn tin email của tài khoản công ty thì thêm optional claim `xms_edov` vào ID token (*Token configuration*): thiếu claim đó, Autoxide không bao giờ liên kết theo email với Microsoft, vì Entra cho đổi email mà không xác minh.
- **GitHub** — *Settings → Developer settings → OAuth Apps → New OAuth App*. GitHub không có OpenID Connect: Autoxide đọc id và email chính đã xác minh của người dùng qua API của GitHub.
- **Issuer OpenID Connect bất kỳ** (Keycloak, Auth0, Okta, Zitadel…) — issuer phải công bố `/.well-known/openid-configuration` và ký ID token bằng `RS256` hay `ES256`.

## 2. Thêm nhà cung cấp cho khách

Cần permission `auth.oidc.manage` (`auth.oidc.read` nếu chỉ xem danh sách) và, để thêm hay sửa nhà cung cấp, một phiên đăng nhập: API key không làm được (`auth.session_required`). Client secret lưu mã hoá và không bao giờ hiện lại (`has_secret` cho biết đã lưu chưa).

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
```

<!-- example: add-provider -->
```bash
curl -sS "$API/api/v1/auth/oidc-providers" -H "Authorization: Bearer $TOKEN" \
  --json '{"slug":"google","kind":"google","name":"Google","client_id":"1234.apps.googleusercontent.com","client_secret":"from-google","redirect_uris":["https://app.example.com/sign-in/callback"]}'
# expect: 201
```

- `kind` `oidc` cần `issuer`. `microsoft` mặc định nhận mọi tài khoản công ty / trường học (`https://login.microsoftonline.com/organizations/v2.0`); khai `https://login.microsoftonline.com/<tenant id>/v2.0` để chỉ nhận công ty của bạn. `github` có `issuer` là GitHub Enterprise Server.
- `redirect_uris`: đúng các trang nhà cung cấp được đưa trình duyệt về, so khớp từng ký tự.
- `allowed_email_domains`: chỉ email thuộc các tên miền này được đăng nhập.
- `allow_signup`: người chưa có tài khoản được tạo tài khoản ở lần đăng nhập đầu — chỉ khi đăng ký cũng đang mở (`AUTOXIDE_AUTH_SELF_REGISTRATION` cho nhân sự, cài đặt `auth.customer_registration` cho `realm` `customer`), **hoặc** nhà cung cấp gán role cho tài khoản mới (`default_role_id`, tại `default_org_id`) — cách một công ty cho nhân viên vào bằng Entra ID của mình. Ai lưu nhà cung cấp có gán role thì phải tự trao được role đó, ở mọi lần lưu. Ở mỗi lần tạo tài khoản, máy chủ kiểm lại người lưu nhà cung cấp gần nhất còn hoạt động, còn `auth.oidc.manage` và còn trao được role đó; không thì từ chối tạo tài khoản (`auth.oidc_default_role_unavailable`) cho tới khi quản trị viên lưu lại nhà cung cấp.
- Địa chỉ nhà cung cấp phải là địa chỉ public: địa chỉ trong mạng riêng bị từ chối trừ khi người vận hành đặt `AUTOXIDE_AUTH_OIDC_ALLOW_PRIVATE_NETWORKS=true` (chỉ để thử).

Sửa bằng `PATCH /api/v1/auth/oidc-providers/{id}`, xoá bằng `DELETE` (mọi liên kết tài khoản tới nó bị xoá theo).

## 3. Đăng nhập

Giao diện dựng các nút từ `GET /api/v1/auth/oidc/providers` (công khai). Máy chủ ví dụ này đang tắt đăng nhập ngoài:

<!-- example: list-providers -->
```bash
curl -sS "$API/api/v1/auth/oidc/providers"
# expect: 404
```

Khi đã bật:

1. `POST /api/v1/auth/oidc/{slug}/start` với `{"redirect_uri": "https://app.example.com/sign-in/callback"}` trả `authorization_url`. Mở nó trong trình duyệt. Máy chủ giữ PKCE verifier, `state` và `nonce` trong 10 phút; chúng không bao giờ rời máy chủ.
2. Nhà cung cấp đưa trình duyệt về trang của bạn với `code` và `state` trên query.
3. `POST /api/v1/auth/oidc/{slug}/callback` với `{"code": "…", "state": "…"}` trả lời y như `POST /api/v1/auth/login`: cặp token, hoặc `202` kèm `mfa_token` khi tài khoản bật đăng nhập hai bước (tiếp ở `POST /api/v1/auth/login/mfa`). `state` chỉ dùng được một lần.

Tài khoản nào được đăng nhập:

- tài khoản đã liên kết với danh tính này ở nhà cung cấp;
- nếu chưa, tài khoản có cùng email — **chỉ khi cả nhà cung cấp lẫn Autoxide đã xác minh email đó**; một bên chưa thì trả `409 auth.oidc_link_required`: đăng nhập bằng cách cũ rồi liên kết nhà cung cấp (bên dưới). Cách này chặn việc chiếm tài khoản dựng sẵn bằng email của người khác;
- nếu không, một tài khoản mới khi được phép (xem `allow_signup`), không thì `403 auth.oidc_signup_closed`. Email chưa xác minh không bao giờ tạo tài khoản (`403 auth.oidc_email_unverified`).

Lỗi: `400 auth.oidc_state_invalid` (bắt đầu lại), `401 auth.oidc_token_invalid` kèm `reason` (`signature`, `issuer`, `audience`, `expired`, `nonce`, `unknown_key`, `algorithm`…), `403 auth.oidc_email_domain_not_allowed`, `503 auth.oidc_provider_unavailable`.

## 4. Liên kết và gỡ trong phần tài khoản

- `GET /api/v1/auth/me/identities` liệt kê nhà cung cấp đã liên kết.
- `POST /api/v1/auth/me/identities/{slug}/start` với `redirect_uri` và `current_password` (hay `reauth_token` lấy ở `POST /api/v1/auth/me/reauth`), rồi `POST /api/v1/auth/me/identities/{slug}/callback` với `code` và `state`, vẫn trong phiên đăng nhập đó.
- `DELETE /api/v1/auth/me/identities/{id}` gỡ một liên kết, nhưng không gỡ được cách đăng nhập cuối cùng (`409 auth.identity_last_method`): đặt mật khẩu hay thêm passkey trước.

Danh tính đã liên kết (subject và email ở nhà cung cấp) có trong bản xuất dữ liệu cá nhân và bị xoá cùng tài khoản.

Đăng nhập bằng Google hay GitHub không thay cho việc xác thực bằng số điện thoại mà một số module đòi khi đăng nội dung (Nghị định 147/2024/NĐ-CP).
