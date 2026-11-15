# Đăng nhập và giữ phiên đăng nhập

Tiếng Anh: [`authentication.md`](authentication.md). Ví dụ dùng `curl` với biến `API` là địa chỉ máy chủ của bạn (`export API=https://api.example.com`), và SDK TypeScript (`sdk/typescript`). Chúng chạy như test với mỗi bản build, nên khớp máy chủ của phiên bản này.

## Đăng nhập

Gửi email và mật khẩu. Câu trả lời có **access token** sống ngắn (gửi kèm mọi request) và **refresh token** (để lấy cặp kế tiếp). Tài khoản bật xác thực hai bước thì trả `202` kèm `challenge_id`; hoàn tất bằng `POST /api/v1/auth/login/mfa` với mã.

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
# save: REFRESH=.data.refresh_token
```

<!-- example: who-am-i -->
```bash
curl -sS "$API/api/v1/auth/me" -H "Authorization: Bearer $TOKEN"
```

## Làm mới trước khi access token hết hạn

`expires_in` cho biết access token sống bao nhiêu giây. Trước lúc đó một chút, đổi refresh token lấy cặp mới. Mỗi refresh token chỉ dùng được **một lần**: giữ lại cái mới.

<!-- example: refresh -->
```bash
curl -sS "$API/api/v1/auth/refresh" --json '{"refresh_token":"'"$REFRESH"'"}'
# save: TOKEN=.data.access_token
# save: NEW_REFRESH=.data.refresh_token
```

Gửi lại refresh token cũ trông như token bị đánh cắp: máy chủ từ chối (`auth.refresh_token_reused`) **và kết thúc cả phiên**, nên refresh token mới nhất cũng thôi dùng được. Đăng nhập lại.

<!-- example: refresh-token-reused -->
```bash
curl -sS "$API/api/v1/auth/refresh" --json '{"refresh_token":"'"$REFRESH"'"}'
# expect: 401
```

<!-- example: session-ended -->
```bash
curl -sS "$API/api/v1/auth/refresh" --json '{"refresh_token":"'"$NEW_REFRESH"'"}'
# expect: 401
```

## Đăng xuất

Kết thúc một phiên bằng refresh token của nó; access token thôi dùng được khi hết hạn.

<!-- example: sign-in-again -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
# save: REFRESH=.data.refresh_token
```

<!-- example: sign-out -->
```bash
curl -sS "$API/api/v1/auth/logout" --json '{"refresh_token":"'"$REFRESH"'"}'
# expect: 204
```

## Các phiên của bạn

`GET /api/v1/auth/me/sessions` liệt kê các thiết bị đang đăng nhập tài khoản: `user_agent`, `sign_in_method` (`password`, `password_and_mfa`, `passkey`, `password_change`) và, khi reverse proxy của máy chủ cho biết (`AUTOXIDE_AUTH_COUNTRY_HEADER`, vd `CF-IPCountry`), quốc gia gần đúng `country`. Địa chỉ IP không bao giờ được lưu. Phiên lạ thì kết thúc bằng `DELETE /api/v1/auth/me/sessions/{id}`.

## Với SDK

Đưa client một hàm trả token, để mỗi request đọc token hiện tại:

```ts
import { createClient } from "@autoxide/sdk";

let tokens = { access: "", refresh: "" };
const api = createClient({ baseUrl: "https://api.example.com", token: () => tokens.access });

const signedIn = await api.post("/api/v1/auth/login", {
  body: { email: "admin@example.com", password: "correct horse battery staple" },
});
if ("access_token" in signedIn) {
  tokens = { access: signedIn.access_token, refresh: signedIn.refresh_token };
}

const me = await api.get("/api/v1/auth/me");
console.log(me.email, me.permissions);

const next = await api.post("/api/v1/auth/refresh", { body: { refresh_token: tokens.refresh } });
tokens = { access: next.access_token, refresh: next.refresh_token };
```
