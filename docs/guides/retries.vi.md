# Gọi lại an toàn

Tiếng Anh: [`retries.md`](retries.md). Ví dụ dùng `curl` với biến `API` là địa chỉ máy chủ của bạn (`export API=https://api.example.com`), và SDK TypeScript (`sdk/typescript`). Chúng chạy như test với mỗi bản build, nên khớp máy chủ của phiên bản này.

Lỗi mạng sau khi gửi một lệnh ghi khiến bạn không chắc nó đã chạy chưa. Gửi header `Idempotency-Key` (một UUID ngẫu nhiên cho mỗi thao tác) với mọi lệnh ghi: gửi lại cùng key trong 24 giờ trả **kết quả đầu tiên**, kèm `Idempotent-Replayed: true`, thay vì làm hai lần.

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
# save: REFRESH=.data.refresh_token
```

<!-- example: first-try -->
```bash
curl -sS "$API/api/v1/content/entries" -H "Authorization: Bearer $TOKEN" \
  -H 'Idempotency-Key: 0193e3a4-7b1c-7f00-9a51-6f2b8c0d4e11' \
  --json '{"kind":"post","title":"Written once"}'
# expect: 201
# save: ID=.data.id
```

<!-- example: retry-gets-the-first-result -->
```bash
curl -sS "$API/api/v1/content/entries" -H "Authorization: Bearer $TOKEN" \
  -H 'Idempotency-Key: 0193e3a4-7b1c-7f00-9a51-6f2b8c0d4e11' \
  --json '{"kind":"post","title":"Written once"}'
# expect: 201
# expect-header: idempotent-replayed: true
```

Dùng cùng key cho một request **khác** là lỗi, máy chủ từ chối (`idempotency_key_reused`, 422): tạo key mới cho mỗi thao tác khác nhau.

<!-- example: same-key-other-request -->
```bash
curl -sS "$API/api/v1/content/entries" -H "Authorization: Bearer $TOKEN" \
  -H 'Idempotency-Key: 0193e3a4-7b1c-7f00-9a51-6f2b8c0d4e11' \
  --json '{"kind":"post","title":"Something else"}'
# expect: 422
```

Lưu ý:

- Request còn đang chạy trả `409 idempotency_key_in_progress` kèm `Retry-After`; gọi lại sau chừng ấy giây.
- Kết quả chứa secret chỉ hiện một lần (API key mới) không được giữ: gửi lại trả `409 idempotency_result_not_kept`. Xem danh sách key để biết nó đã được tạo chưa.
- Lệnh nghiệp vụ chuyển hàng hay tiền còn nhận mã tham chiếu của riêng bạn (`reference`), duy nhất cho mỗi thao tác, để cả lần gọi lại sau 24 giờ cũng không chạy hai lần.

## Với SDK

```ts
import { AutoxideError, createClient } from "@autoxide/sdk";

const api = createClient({ baseUrl: "https://api.example.com", token: "axk_…" /* read it from your secret store */ });

// One key per logical operation, reused on every retry of it.
const key = crypto.randomUUID();
for (let attempt = 1; ; attempt++) {
  try {
    const entry = await api.post("/api/v1/content/entries", {
      body: { kind: "post", title: "Written once" },
      idempotencyKey: key,
    });
    console.log(entry.id);
    break;
  } catch (error) {
    const retryable = error instanceof TypeError || (error instanceof AutoxideError && error.status >= 500);
    if (!retryable || attempt === 3) throw error;
    await new Promise((resolve) => setTimeout(resolve, attempt * 1000));
  }
}
```
