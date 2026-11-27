# Đăng nhập và giữ phiên đăng nhập

Tiếng Anh: [`authentication.md`](authentication.md). Ví dụ dùng `curl` với biến `API` là địa chỉ máy chủ của bạn (`export API=https://api.example.com`), và SDK TypeScript (`sdk/typescript`). Chúng chạy như test với mỗi bản build, nên khớp máy chủ của phiên bản này.

## Đăng nhập

Gửi email và mật khẩu. Câu trả lời có **access token** sống ngắn (gửi kèm mọi request) và **refresh token** (để lấy cặp kế tiếp). Tài khoản bật xác thực hai bước thì trả `202` kèm `challenge_id`; hoàn tất bằng `POST /api/v1/auth/login/mfa` với mã.

Trên máy chủ phục vụ nhiều khách, đăng nhập trên tên miền của khách hoặc kèm header `Autoxide-Tenant`. Token nhận được ghi sẵn khách của nó, nên các request sau — bằng access token, `/refresh`, `/logout`, `/login/mfa`, và link trong mail đặt lại mật khẩu hay xác minh email — không cần header hay tên miền nữa. Gửi token kèm header (hay tên miền) của khách khác thì bị từ chối.

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

## Đăng nhập bằng link

Khi máy chủ bật (`AUTOXIDE_AUTH_LOGIN_LINK=true`), `POST /api/v1/auth/login/link` với `{"email": …}` gửi một link đăng nhập dùng một lần (sống `AUTOXIDE_AUTH_LOGIN_LINK_MINUTES`, mặc định 15 phút). Endpoint luôn trả 202 như nhau dù email có tài khoản hay không. Hoàn tất bằng `POST /api/v1/auth/login/link/confirm` với `token` trong mail; kết quả như `/login` (token, hay `mfa_token` khi bật xác thực hai bước). Gửi `"bind_device": true` để nhận `device_secret` rồi gửi lại nó cùng token: link khi đó chỉ dùng được trên thiết bị này.

## Người mua: đăng nhập bằng số điện thoại

Người mua đăng nhập, hay tạo tài khoản, bằng mã gửi qua SMS. Máy chủ cần có nhà cung cấp SMS; không có
thì các endpoint này trả 503 `auth.phone_sign_in_unavailable`.

1. `POST /api/v1/auth/phone/codes` với `{"phone": "0912345678"}` trả 202 kèm `challenge_id`. Câu trả
   lời giống nhau dù số đã có tài khoản hay chưa.
2. `POST /api/v1/auth/phone/login` với `{"challenge_id": "…", "code": "123456", "name": "Lan"}`. Số đã
   có tài khoản người mua thì đăng nhập vào đó (200). Số mới thì tạo tài khoản người mua không email,
   không mật khẩu (201) khi cửa hàng cho người mua tự tạo tài khoản; `name` chỉ dùng lúc đó. Cả hai trả
   cùng token như `/login`, hoặc 202 khi tài khoản bật đăng nhập hai bước.

Tài khoản như vậy có `email` là `""` cho tới khi người mua thêm email. Để xác nhận một thay đổi nhạy cảm
(`POST /api/v1/auth/me/reauth`), dùng `"method": "phone_code"` với `phone_challenge_id` và `phone_code`
mới gửi tới chính số của mình; sau đó thêm được passkey và đăng nhập bằng passkey.

**Giới hạn chống đoán mã** (NIST SP 800-63B-4). Mã sống tối đa 10 phút
(`AUTOXIDE_COMPLIANCE_OTP_TTL_SECONDS`, 60 tới 600) và chỉ cho nhập sai vài lần. Theo từng số:

- số mã mỗi giờ từ một client (một địa chỉ IPv4, hay một khối IPv6 /64) và từ mọi client cộng lại, và số mã mỗi
  ngày — mã đăng nhập và mã xác minh số đếm riêng, nên xin mã đăng nhập không bao giờ làm chủ số hết đường xác minh
  lại số (`429 compliance.otp_throttled`);
- số mã sai mỗi ngày (`429 compliance.otp_daily_limit`): với mã đăng nhập, từ một client và từ mọi client cộng
  lại; với mã xác minh số, chỉ trên mã của chính người dùng với số đó (không có trần "mọi client", nên lần sai của
  người khác không chặn được);
- số mã sai liên tiếp, không theo cửa sổ thời gian: từ 90 lần thì số bị khoá với client chưa từng đăng nhập bằng số,
  từ 100 lần thì khoá với mọi client (`403 compliance.otp_locked`).

Client đã đăng nhập bằng số trong 90 ngày không bị tính vào các giới hạn "mọi client" và giữ 10 lượt cuối của 100,
nên người lạ không khoá được chủ số ở mạng họ hay dùng. Nhập đúng thì bộ đếm về 0. Số bị khoá được mở khi chủ số đăng
nhập bằng cách khác và xác thực lại số trong tài khoản, hay khi cửa hàng mở khoá — việc này cũng tính lại từ đầu số mã
sai mỗi ngày, còn giới hạn gửi mã vẫn áp như thường
(`POST /api/v1/compliance/phone-locks/unlock`, permission `compliance.phone_locks.manage`; tool MCP
`compliance.unlock_phone`; `autoxide task compliance.unlock_phone phone=…`) — đường lấy lại tài khoản cho người chỉ
đăng nhập bằng số. **Rủi ro còn lại:** mạng di động sau NAT của nhà mạng đổi địa chỉ thường xuyên; ở mạng chủ số chưa
từng đăng nhập, họ có thể bị khoá khi người lạ chạm các giới hạn đó. Hãy cho người mua thêm một cách đăng nhập (mật khẩu
hay passkey) — NIST đòi điều này với mã SMS.

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

Kết thúc một phiên bằng refresh token của nó. Access token của phiên thôi dùng được ngay (mã `auth.session_ended`), không chờ hết hạn; cũng vậy khi bạn kết thúc một phiên trong danh sách bên dưới, đổi mật khẩu, hay quản trị viên đăng xuất hoặc khoá tài khoản của bạn.

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

<!-- example: signed-out-token -->
```bash
curl -sS "$API/api/v1/auth/me" -H "Authorization: Bearer $TOKEN"
# expect: 401
```

## Các phiên của bạn

`GET /api/v1/auth/me/sessions` liệt kê các thiết bị đang đăng nhập tài khoản: `user_agent`, `sign_in_method` (`password`, `password_and_mfa`, `passkey`, `email_link`, `password_change`) và, khi reverse proxy của máy chủ cho biết (`AUTOXIDE_AUTH_COUNTRY_HEADER`, vd `CF-IPCountry`), quốc gia gần đúng `country`. Địa chỉ IP không bao giờ được lưu. Phiên lạ thì kết thúc bằng `DELETE /api/v1/auth/me/sessions/{id}`.

## Tài khoản của bạn

`PATCH /api/v1/auth/me` đổi tên hiển thị (`name`), ngôn ngữ ưa thích (`locale`: `en` hay `vi`) và múi giờ (`time_zone`, theo IANA, vd `Asia/Ho_Chi_Minh`); `null` bỏ đặt ngôn ngữ hay múi giờ. Mail gửi sau (đặt lại mật khẩu, báo đổi email) dùng ngôn ngữ ưa thích của bạn.

Thay đổi quan trọng — tạo API key, đổi email, bật xác thực hai bước, thêm passkey, xoá tài khoản — cần chứng minh đó là bạn. Gửi `current_password`, hoặc trước đó lấy token xác nhận ngắn hạn bằng `POST /api/v1/auth/me/reauth` (`{"method": "password" | "totp" | "passkey" | "email_link", …}`) rồi gửi nó trong `reauth_token`. Mỗi token dùng một lần, trong `AUTOXIDE_AUTH_REAUTH_MINUTES` (mặc định 10 phút). Tài khoản không có mật khẩu dùng `passkey`, `totp` hay `email_link`.

Một role có thể bắt buộc xác thực hai bước (`requires_mfa`, cả với role `admin` dựng sẵn). Khi tài khoản chưa bật ứng dụng xác thực hay thêm passkey, permission của role đó bị bỏ ra và `GET /api/v1/auth/me` trả `mfa_required: true`.

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
