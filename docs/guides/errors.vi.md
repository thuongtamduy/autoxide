# Đọc lỗi

Tiếng Anh: [`errors.md`](errors.md). Ví dụ dùng `curl` với biến `API` là địa chỉ máy chủ của bạn (`export API=https://api.example.com`), và SDK TypeScript (`sdk/typescript`). Chúng chạy như test với mỗi bản build, nên khớp máy chủ của phiên bản này.

Mọi lỗi theo RFC 9457 Problem Details (`application/problem+json`):

- `code` — ổn định, cho chương trình: **rẽ nhánh theo nó**, không theo câu chữ. Mã của module bắt đầu bằng tên module (`content.not_found`).
- `detail` — câu cho người đọc, theo ngôn ngữ của `Accept-Language` (mặc định `en`, hoặc `vi`).
- `errors` — với `422`, mỗi trường sai một mục: `field` (vd `items[0].url`), `code`, `detail`. Mọi lỗi của request được báo một lần.
- header `X-Request-Id` — ghi kèm khi báo lỗi.
- `Retry-After` (header) — với `429` (quá nhiều request) và `503` (máy chủ đang khởi động lại), số giây phải chờ.

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
# save: REFRESH=.data.refresh_token
```

Chưa đăng nhập:

<!-- example: not-signed-in -->
```bash
curl -sS "$API/api/v1/content/entries"
# expect: 401
```

Đầu vào sai, chi tiết bằng tiếng Việt:

<!-- example: invalid-input -->
```bash
curl -sS "$API/api/v1/content/entries" -H "Authorization: Bearer $TOKEN" -H 'Accept-Language: vi' \
  --json '{"kind":"essay","title":"x"}'
# expect: 422
```

Trường gõ sai tên hay không có trong thân request bị từ chối bằng `422` (mã `invalid_body`, `detail` nêu tên trường và các trường được nhận) thay vì bị lặng lẽ bỏ qua:

<!-- example: unknown-field -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","pasword":"correct horse battery staple"}'
# expect: 422
```

Bản ghi không tồn tại:

<!-- example: not-found -->
```bash
curl -sS "$API/api/v1/content/entries/0193e3a4-7b1c-7f00-9a51-6f2b8c0d4e11" -H "Authorization: Bearer $TOKEN"
# expect: 404
```

## Sửa một bản ghi: `If-Match`, `428` và `412`

Đọc một bản ghi trả phiên bản của nó trong header `ETag`. Endpoint sửa bản ghi nhận lại giá trị đó trong `If-Match`, để hai người không ghi đè lên nhau. Endpoint có mô tả ghi `If-Match` là bắt buộc trả `428` (mã `precondition_required`) khi thiếu; phiên bản không còn mới nhất thì nhận `412` (mã `precondition_failed`): đọc lại bản ghi, áp thay đổi của bạn lên bản vừa đọc rồi gửi `ETag` mới.

<!-- example: create-entry -->
```bash
curl -sS "$API/api/v1/content/entries" -H "Authorization: Bearer $TOKEN" \
  --json '{"kind":"post","title":"First title"}'
# expect: 201
# save: ENTRY=.data.id
```

<!-- example: read-entry -->
```bash
curl -sS "$API/api/v1/content/entries/$ENTRY" -H "Authorization: Bearer $TOKEN"
# save: ENTRY_ETAG=header:etag
```

<!-- example: change-without-version -->
```bash
curl -sS -X PATCH "$API/api/v1/content/entries/$ENTRY" -H "Authorization: Bearer $TOKEN" \
  --json '{"title":"Second title"}'
# expect: 428
```

<!-- example: change-with-version -->
```bash
curl -sS -X PATCH "$API/api/v1/content/entries/$ENTRY" -H "Authorization: Bearer $TOKEN" \
  -H "If-Match: $ENTRY_ETAG" --json '{"title":"Second title"}'
# expect: 200
```

<!-- example: change-with-old-version -->
```bash
curl -sS -X PATCH "$API/api/v1/content/entries/$ENTRY" -H "Authorization: Bearer $TOKEN" \
  -H "If-Match: $ENTRY_ETAG" --json '{"title":"Third title"}'
# expect: 412
```

## Ứng dụng nên làm gì

Các mã ứng dụng web hay di động hay gặp nhất, và cách xử lý từng mã. Rẽ nhánh theo `code`; hiện `detail` cho
người dùng (đã đúng ngôn ngữ của họ). Mã của module bắt đầu bằng tên module (`content.not_found`,
`auth.email_taken`): không có cách xử lý riêng thì xử lý theo status.

| Status | `code` | Nghĩa là | Ứng dụng làm gì |
| ------ | ------ | -------- | --------------- |
| 401 | `auth.token_expired` | Access token đã hết hạn | Refresh một lần (`POST /api/v1/auth/refresh`) rồi gửi lại request; SDK tự làm khi có tuỳ chọn `refresh` |
| 401 | `auth.missing_token`, `auth.invalid_token`, `auth.session_ended` | Không có token, token hỏng, hoặc phiên đã kết thúc (đăng xuất, đổi mật khẩu, bị thu hồi) | Quên token, hiện trang đăng nhập |
| 401 | `auth.invalid_refresh_token`, `auth.refresh_token_reused` | Từ `/auth/refresh`: refresh token không có, hết hạn, hoặc đã dùng rồi (phiên giờ đã kết thúc) | Đăng nhập lại. Không bao giờ refresh hai lần cùng lúc |
| 401 | `auth.invalid_credentials` | Từ `/auth/login`: sai email hoặc mật khẩu | Báo sai, không nói sai cái nào |
| 401 | `auth.invalid_mfa_code`, `auth.invalid_mfa_token` | Từ `/auth/login/mfa`: mã sai hoặc đã dùng; lượt đăng nhập hết hạn hay hết số lần thử | Hỏi mã kế tiếp; với `auth.invalid_mfa_token` thì bắt đầu lại từ mật khẩu |
| 401 | `step_up_required` | Thao tác nhạy cảm cần xác nhận lại là chính bạn | Hỏi mật khẩu hay mã, gọi `POST /api/v1/auth/me/reauth`, rồi gửi lại |
| 403 | `permission_denied` | Người dùng thiếu permission (`detail` nêu tên) | Ẩn thao tác; gợi ý xin quản trị viên |
| 403 | `auth.user_disabled` | Tài khoản bị khoá | Báo người dùng; không gửi lại |
| 404 | `not_found`, `<module>.…_not_found` | Đã mất, chưa từng có (hoặc thuộc khách khác) | Tải lại danh sách hoặc quay lại |
| 409 | `idempotency_key_in_progress` | Request đầu tiên với key này vẫn đang chạy | Chờ `Retry-After` (1 giây) rồi gửi lại đúng request với **cùng** key |
| 409 | `<module>.…_taken`, `conflict` | Giá trị duy nhất đã có người dùng, hoặc trạng thái không cho phép | Hiện `detail` cạnh trường |
| 412 | `precondition_failed` | Có người đã sửa bản ghi sau khi bạn đọc | Đọc lại, hiện bản mới, áp lại thay đổi |
| 413 | `payload_too_large` | Thân request hay file lớn hơn máy chủ nhận | Báo giới hạn; file lớn thì [tải thẳng lên nơi lưu](direct-uploads.vi.md) |
| 422 | `validation_failed` | Đầu vào sai; **mọi** trường sai nằm trong `errors[]` | Hiện từng `errors[].detail` cạnh `errors[].field` của nó |
| 422 | `invalid_body` | Trường lạ hay viết sai, sai kiểu JSON | Lỗi của ứng dụng: sửa request |
| 422 | `idempotency_key_reused` | Cùng key gửi với một request khác | Lỗi của ứng dụng: mỗi thao tác một key mới |
| 428 | `precondition_required` | Endpoint cần `If-Match` | Đọc bản ghi, gửi `ETag` của nó trong `If-Match` |
| 429 | `rate_limited` | Quá nhiều request từ địa chỉ client này (hoặc với API key này) | Chờ `Retry-After` giây, rồi gửi thưa hơn |
| 429 | `tenant_rate_limited` | Cả khách (mọi người dùng và key cộng lại) vượt giới hạn | Chờ `Retry-After` giây |
| 429 | `auth.login_throttled` | Quá nhiều lần đăng nhập sai với email này (từ địa chỉ này, hoặc từ mọi nơi) | **Không tự gửi lại.** Bảo người dùng chờ `Retry-After` giây (thường là vài phút); tới lúc đó đúng mật khẩu cũng bị từ chối |
| 503 | `request_timeout`, `shutting_down` | Request chạy quá lâu, hoặc máy chủ đang khởi động lại | Chờ `Retry-After` (hoặc vài giây) rồi gửi lại; lệnh ghi kèm `Idempotency-Key` |

Hai loại `429` hay gặp đếm riêng: `rate_limited` đếm **request** theo địa chỉ client (mọi endpoint,
`AUTOXIDE_HTTP_RATE_LIMIT_PER_MINUTE`), còn `auth.login_throttled` đếm **lần đăng nhập sai** theo email
(`AUTOXIDE_AUTH_LOGIN_MAX_FAILURES` trong `AUTOXIDE_AUTH_LOGIN_LOCK_MINUTES`). Form đăng nhập không bao giờ được
tự gửi lại.

## Với SDK

Lỗi ném ra `AutoxideError`:

```ts
import { AutoxideError, createClient } from "@autoxide/sdk";

const api = createClient({ baseUrl: "https://api.example.com", token: "axk1_…" /* read it from your secret store */, language: "vi" });

try {
  await api.post("/api/v1/content/entries", { body: { kind: "post", title: "" } });
} catch (error) {
  if (!(error instanceof AutoxideError)) throw error;
  switch (error.code) {
    case "validation_failed":
      for (const field of error.errors) console.log(field.field, field.detail);
      break;
    case "rate_limited":
      console.log(`try again in ${error.retryAfter} s`);
      break;
    default:
      console.log(error.status, error.detail, error.requestId);
  }
}
```
