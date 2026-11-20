# Làm app điện thoại hay app máy tính

Tiếng Anh: [`mobile.md`](mobile.md). Ví dụ dùng `curl` với `API` là địa chỉ máy chủ của bạn (`export API=https://api.example.com`). Các ví dụ chạy như test ở mỗi bản build, nên luôn khớp với máy chủ của phiên bản này.

Autoxide chỉ có API: app điện thoại, app máy tính hay máy bán hàng tại quầy gọi đúng các endpoint mà website gọi. Hướng dẫn này đi qua những gì một app như vậy cần, theo thứ tự thường làm, và chỉ tới hướng dẫn chi tiết của từng phần.

## 1. Chọn SDK

Ba SDK sinh từ hợp đồng API (`docs/openapi.json`), nên luôn khớp với máy chủ cùng phiên bản:

| Nền tảng | Gói | Sinh bằng |
| --- | --- | --- |
| iOS, iPadOS, macOS (Swift) | [`sdk/swift`](../../sdk/swift/README.vi.md), Swift Package `Autoxide` | `swift-openapi-generator` của Apple, lúc build |
| Android, JVM trên máy tính (Kotlin) | `sdk/kotlin` | `openapi-generator`, generator `kotlin` (`jvm-okhttp4`, Moshi) |
| Flutter (Dart) | `sdk/dart` | `openapi-generator`, generator `dart-dio` (`built_value`) |
| Web, React Native, Node (TypeScript) | [`sdk/typescript`](../../sdk/typescript/) | `openapi-typescript` |

Mã Kotlin và Dart không lưu trong repo; sinh bằng đúng phiên bản generator và cấu hình đã ghim ở [`sdk/openapitools.json`](../../sdk/openapitools.json):

```bash
cd sdk
npx @openapitools/openapi-generator-cli generate --generator-key kotlin   # → sdk/kotlin/generated
npx @openapitools/openapi-generator-cli generate --generator-key dart     # → sdk/dart/generated
```

Các gói chưa đăng lên Swift Package Index, Maven Central hay pub.dev: thêm từ repo theo tag của phiên bản máy chủ. SDK nào cũng theo ba luật cho mọi lệnh gọi:

- Gửi access token trong `Authorization: Bearer …`.
- Lệnh ghi gửi header `Idempotency-Key` (một UUID ngẫu nhiên mới cho mỗi hành động, giữ nguyên khi gửi lại): mạng điện thoại chập chờn nên app hay gửi lại — xem [Gửi lại an toàn](retries.vi.md).
- Mọi lỗi là một `Problem` có `code` ổn định; hiện `detail` cho người dùng (gửi `Accept-Language: vi` để nhận tiếng Việt) và rẽ nhánh theo `code` — xem [Đọc lỗi](errors.vi.md).

## 2. Đăng nhập và giữ đăng nhập

Đăng nhập bằng email và mật khẩu (hoặc passkey, hoặc đường link đăng nhập — xem [Đăng nhập](authentication.vi.md)). Cất cả hai token trong kho bảo mật của hệ điều hành (Keychain trên thiết bị Apple, bộ lưu dựa trên Keystore trên Android), không bao giờ để trong phần cài đặt thường.

<!-- example: sign-in -->
```bash
curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}'
# save: TOKEN=.data.access_token
# save: REFRESH=.data.refresh_token
```

Access token sống `expires_in` giây. Ngay trước khi hết hạn, hoặc khi một lệnh gọi trả `401` với `auth.token_expired`, đổi refresh token lấy cặp token mới. Mỗi refresh token chỉ dùng được **một lần**: lưu token mới trước khi dùng access token mới. Chỉ làm mới token ở một chỗ trong app (mỗi lúc một lần làm mới); hai màn hình cùng làm mới bằng một token trông như token bị đánh cắp, và máy chủ kết thúc cả phiên đăng nhập.

<!-- example: refresh -->
```bash
curl -sS "$API/api/v1/auth/refresh" --json '{"refresh_token":"'"$REFRESH"'"}'
# save: TOKEN=.data.access_token
# save: REFRESH=.data.refresh_token
```

Làm mới mà nhận `401` (`auth.invalid_refresh_token`, `auth.refresh_token_reused`) thì xoá token và mở màn hình đăng nhập. Khi đăng xuất, gửi refresh token tới `POST /api/v1/auth/logout`, rồi xoá cả hai token và push token của máy (mục 4).

Chương trình chạy không có người (máy kiosk, dịch vụ đồng bộ) dùng API key — xem [API key](api-keys.vi.md).

## 3. Đăng nhập bằng Google, Microsoft hay GitHub (PKCE)

Máy chủ chạy luồng OAuth 2.0 authorization code có PKCE thay cho app, và tự giữ PKCE verifier, `state` và `nonce`: app không bao giờ giữ client secret. Quản trị viên thêm nhà cung cấp với địa chỉ quay về của app trong `redirect_uris` — một link HTTPS app đã nhận (Universal Link trên iOS, App Link trên Android, vd `https://app.example.com/sign-in/callback`) hoặc scheme riêng của app (`com.example.app:/sign-in/callback`). Xem [Đăng nhập bằng Google, Microsoft, GitHub hay OpenID Connect](oidc.vi.md).

1. App gọi `POST /api/v1/auth/oidc/{slug}/start` với `{"redirect_uri": "<địa chỉ quay về của app>"}`.
2. App mở `authorization_url` bằng trình duyệt đăng nhập của hệ điều hành — `ASWebAuthenticationSession` trên thiết bị Apple, Custom Tabs trên Android, `flutter_web_auth_2` trong Flutter — không mở trong web view nhúng, vì nhà cung cấp từ chối.
3. Nhà cung cấp đưa trình duyệt về địa chỉ quay về kèm `code` và `state`. App gửi nguyên cả hai tới `POST /api/v1/auth/oidc/{slug}/callback` và nhận câu trả lời như `/login`: token, hoặc `202` kèm `mfa_token` khi bật đăng nhập hai bước.

Danh sách nhà cung cấp cho màn hình đăng nhập (máy chủ này tắt đăng nhập bên ngoài nên trả `404`):

<!-- example: providers -->
```bash
curl -sS "$API/api/v1/auth/oidc/providers"
# expect: 404
```

## 4. Thông báo đẩy

Sau khi đăng nhập, và mỗi khi hệ điều hành cấp push token mới cho app, đăng ký máy. Đăng ký lại cùng token chỉ cập nhật nó, nên làm mỗi lần mở app.

<!-- example: register-device -->
```bash
curl -sS "$API/api/v1/notifications/me/device-tokens" -H "Authorization: Bearer $TOKEN" \
  --json '{"platform":"ios","token":"apns-device-token-from-the-os","app_id":"com.example.app","locale":"vi"}'
# expect: 201
# save: DEVICE=.data.id
```

- `platform` là `ios` (token của APNs) hoặc `android` (token của Firebase Cloud Messaging); `app_id` là bundle id hay package name; `locale` (`en`, `vi`) chọn ngôn ngữ của tin.
- `GET /api/v1/notifications/me/device-tokens` liệt kê các máy của người dùng; `DELETE /api/v1/notifications/me/device-tokens/{id}` ngừng đẩy tới một máy — gọi khi đăng xuất.
- Máy chủ bỏ token mà dịch vụ đẩy báo không còn hiệu lực; đăng ký lại khi hệ điều hành cấp token mới.
- Tin nào tới điện thoại theo cài đặt thông báo của người dùng (`/api/v1/notifications/me/preferences`); cũng các tin đó nằm trong hộp thư trong app (`/api/v1/notifications/me/inbox`). Gửi được cần máy chủ cài nhà cung cấp đẩy (FCM, APNs); chưa có thì máy vẫn đăng ký được.

## 5. Tải ảnh thẳng lên nơi lưu

Đừng gửi ảnh lớn qua máy chủ API. Xin link tải lên đã ký, gửi dữ liệu từ điện thoại thẳng lên S3, rồi hoàn tất — xem [Tải file lớn thẳng lên nơi lưu](direct-uploads.vi.md). Tính `size` và `sha256` của đúng dữ liệu sẽ gửi (sau khi thu nhỏ trên điện thoại). Máy chủ này lưu file trên đĩa của nó nên trả `409 media.direct_upload_unavailable`: khi đó app tải lên qua `POST /api/v1/media` (multipart).

<!-- example: start-upload -->
```bash
curl -sS "$API/api/v1/media/uploads" -H "Authorization: Bearer $TOKEN" \
  -H 'Idempotency-Key: 0193e3a4-7b1c-7f00-9a51-6f2b8c0d4e11' \
  --json '{"name":"shelf.jpg","content_type":"image/jpeg","size":482113,"sha256":"9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"}'
# expect: 409
```

Trên S3 câu trả lời là `201` kèm `url`, `method` và `headers`: tải lên với đúng các giá trị đó (tác vụ tải nền trên iOS, `WorkManager` trên Android, để không đứt khi app chạy nền), rồi gọi `POST /api/v1/media/uploads/{id}/complete`.

## 6. Làm việc khi mất mạng

App phải chạy được khi không có mạng (quầy bán hàng, app giao hàng) giữ bản sao bản ghi trên máy và gửi lệnh ghi sau. Máy chủ liệt kê các loại bản ghi đồng bộ được — mỗi module của máy chủ tự thêm loại của mình (module mẫu `notes` thêm `notes.notes`; máy chủ này chưa có loại nào):

<!-- example: sync-resources -->
```bash
curl -sS "$API/api/v1/kernel/sync/resources" -H "Authorization: Bearer $TOKEN"
```

Mỗi loại có endpoint danh sách (`path`) và luồng thay đổi (`changes`). Vòng làm việc:

1. **Lần đầu:** đọc endpoint danh sách (theo [pagination](pagination.vi.md)), rồi `GET <changes>` không kèm con trỏ; giữ `cursor` của thay đổi cuối đã áp.
2. **Bắt kịp:** `GET <changes>?cursor=<con trỏ cuối>`. Mỗi mục là `{id, cursor, deleted, version, data}`: bản ghi **như hiện tại** (áp lại không sao), hoặc `deleted: true` — xoá nó, kể cả khi người dùng mất quyền xem. Đọc các trang cho tới khi `pagination.next_cursor` là `null`.
3. **Tụt quá xa:** con trỏ cũ hơn thời gian máy chủ giữ thay đổi (`AUTOXIDE_SYNC_RETENTION_DAYS`, mặc định 30 ngày) trả `410 sync_cursor_expired`: bỏ bản sao trên máy và làm lại từ bước 1.
4. **Ghi khi mất mạng:** xếp từng lệnh ghi trên máy kèm `idempotency_key` riêng và `version` (`ETag`) của bản ghi nó sửa. Có mạng lại thì gửi hàng đợi bằng `POST /api/v1/kernel/sync/batch`:

```json
{
  "operations": [
    {
      "id": "w-1",
      "method": "PATCH",
      "path": "/api/v1/notes/0193f1c2-5a7e-7c3d-8e21-4b6f0a9d2c11",
      "idempotency_key": "w-1",
      "if_match": "\"3\"",
      "body": { "title": "Shelf B restocked" }
    }
  ]
}
```

Mỗi lệnh chạy đúng như endpoint REST của nó — cùng permission, cùng cách kiểm, cùng câu trả lời — và có kết quả riêng `{id, status, etag, location, body}`; một lệnh hỏng không chặn lệnh khác. Gửi lại cả lô (vì mất câu trả lời) không làm lại việc đã làm, nhờ `idempotency_key` đã giữ. Kết quả `412` nghĩa là có người đã sửa bản ghi: `body.current` là bản trên máy chủ. App tự gộp hoặc hỏi người dùng, rồi xếp lệnh ghi mới với phiên bản mới — máy chủ không bao giờ tự gộp.

## 7. Event thời gian thực và tín hiệu tức thời

Màn hình cần tự làm mới khi có thay đổi thì đăng ký nhận event. Máy chủ phải đặt `AUTOXIDE_EVENTS_STREAM=true` (máy chủ này không đặt, nên cả hai endpoint trả `404 kernel.stream_disabled`).

- **Server-Sent Events** — `GET /api/v1/kernel/events/stream`: một chiều, HTTP thường, cách đơn giản nhất. Xem [Nhận event thời gian thực](realtime-events.vi.md).
- **WebSocket** — `GET /api/v1/kernel/events/ws`: cùng event, cùng quyền, cùng cách nối lại, thêm tín hiệu tức thời app gửi lên ("đang gõ", "online") trong một chủ đề như một cuộc trò chuyện. Dùng cho chat hay màn hình vị trí người giao hàng.

<!-- example: websocket -->
```bash
curl -sS "$API/api/v1/kernel/events/ws?names=content.*" -H "Authorization: Bearer $TOKEN"
# expect: 404
```

Mở socket với header `Authorization` (client WebSocket gốc — `URLSessionWebSocketTask`, OkHttp, `web_socket_channel` — đặt được), `names` trong query và, khi nối lại, `last_event_id`. Mỗi thông điệp là một khung chữ JSON:

```text
→ {"type":"subscribe","names":"comments.*"}
← {"type":"subscribed","names":["comments.*","content.*"]}
← {"type":"event","event":{"id":"0193f1c2-…","name":"content.entry_published","occurred_at":"2026-11-18T09:30:00Z","subject":"content.entry:0193…","actor_kind":"user","payload":{…}}}
→ {"type":"ack","id":"0193f1c2-…"}
→ {"type":"join","topic":"conversation:0193f1d0-…"}
← {"type":"joined","topic":"conversation:0193f1d0-…","online":["0193e0aa-…"]}
→ {"type":"typing","topic":"conversation:0193f1d0-…"}
← {"type":"signal","topic":"conversation:0193f1d0-…","user_id":"0193e0aa-…","signal":"typing"}
```

- Socket chỉ nhận lệnh điều khiển: `subscribe`, `unsubscribe`, `ack`, `join`, `leave`, `typing`, `ping`. **Ghi dữ liệu qua REST** (hoặc lô đồng bộ); thứ khác trả `{"type":"error","code":"kernel.ws_unknown_command"}`.
- Giao ít nhất một lần: bỏ qua id event đã xử lý. Thông điệp `reset` nghĩa là phần bị lỡ không còn giữ — tải lại qua REST.
- Mã đóng: `4401` — thông tin đăng nhập không còn hiệu lực (access token hết hạn): làm mới token rồi nối lại; `4403` — người dùng không còn được nhận event nào đã đăng ký; `1012` — máy chủ khởi động lại: nối lại với `last_event_id`. Nối lại với thời gian chờ tăng dần (1 giây, 2 giây, 4 giây… tới một phút).
- Tín hiệu tức thời không được lưu. Chủ đề `<loại>:<id>` chỉ có khi một module mở nó (nếu không trả `kernel.ws_unknown_topic`), và chỉ vào được khi đọc được bản ghi đó.
- Trên điện thoại, đóng socket khi app chạy nền và dựa vào thông báo đẩy (mục 4); mở lại với `last_event_id` khi app quay lại.

## 8. Mua trong ứng dụng

Khi app bán thuê bao của khách qua App Store hay Google Play, cửa hàng thu và giữ tiền; máy chủ kiểm từng giao dịch với cửa hàng và đưa khách vào gói đã nối. Quản trị viên nối sản phẩm của cửa hàng với gói (`PUT /api/v1/plans/store/products/{store}/{product_id}`), và máy chủ cần thông tin truy cập của cửa hàng.

1. Mua sản phẩm bằng StoreKit 2 hay Google Play Billing.
2. Gửi nguyên thứ cửa hàng trả về — giao dịch đã ký (StoreKit 2) hay `purchaseToken` (Google Play) — với permission `plans.billing.manage`:

<!-- example: claim-purchase -->
```bash
curl -sS "$API/api/v1/plans/store/purchases" -H "Authorization: Bearer $TOKEN" \
  -H 'Idempotency-Key: 0193e3a4-7b1c-7f00-9a51-000000000002' \
  --json '{"store":"app_store","proof":"signed-transaction-from-storekit"}'
# expect: 422
```

Ở đây bạn đăng nhập vào khách nền tảng, không gói nào giới hạn, nên trả `422 plans.platform_exempt`; khách thường trên máy chủ chưa cài cửa hàng nhận `503 plans.store_unavailable`. Trên máy chủ đã cài, câu trả lời là thuê bao của khách. Gửi lại cùng giao dịch không đổi gì, nên gửi ở mỗi lần mở app cho tới khi máy chủ xác nhận (rồi mới kết thúc giao dịch trong StoreKit / xác nhận trong Google Play). Gia hạn, hết hạn và hoàn tiền do chính cửa hàng báo về máy chủ (thông báo máy chủ); app không phải báo. Người dùng đổi hay huỷ thuê bao trong cửa hàng, không qua API.

## Danh sách kiểm

- Token trong kho bảo mật; mỗi lúc một lần làm mới; làm mới mà `401` → màn hình đăng nhập.
- `Idempotency-Key` cho mọi lệnh ghi có thể gửi lại, giữ nguyên qua các lần gửi lại.
- Đăng ký push token mỗi lần mở app, xoá khi đăng xuất.
- File lớn tải thẳng lên nơi lưu.
- Mất mạng: luồng thay đổi với con trỏ đã giữ, lệnh ghi xếp hàng kèm `if_match`, app tự xử lý `412`.
- WebSocket hay SSE chỉ khi app đang mở; nối lại với `last_event_id`.
- Gửi giao dịch mua lên máy chủ cho tới khi được xác nhận.
