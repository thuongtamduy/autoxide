# Bước 69 — Nền cho ứng dụng đa nền tảng (di động, ngoại tuyến, thời gian thực hai chiều)

> Lộ trình: `docs/ROADMAP.md` bước 69 · Quyết định: D-20261119-step69-multiplatform
> Đặc tả liên quan: 31 (hợp đồng API, SDK TypeScript), 33–34 (khách, tổ chức), 35 (gói, thuê bao), 36 (tiền),
> 37 (thông báo đẩy di động), 40 P5 (SSE), 61 (chat)

## Mục tiêu

Core chỉ có API (luật kiến trúc 9), nên mọi loại giao diện đều là "bên gọi API". Web app đã dùng được. Bước này lấp
bốn chỗ còn thiếu để app điện thoại, app máy tính và máy bán hàng tại quầy (POS) dùng Autoxide trọn vẹn như web,
kèm một hướng dẫn cho người làm app:

1. SDK có sẵn cho Swift (iOS), Kotlin (Android) và Dart (Flutter), sinh từ `docs/openapi.json`.
2. Đồng bộ khi mất mạng: app ghi tạm trên máy, có mạng lại thì gửi lên; máy chủ báo xung đột rõ ràng.
3. Kênh thời gian thực hai chiều (WebSocket) bên cạnh SSE của bước 40.
4. Mua trong ứng dụng qua App Store và Google Play, nối vào gói và thuê bao của bước 35.

Mọi thứ trung lập với nghiệp vụ: kernel cho khung chung, module tự khai phần của mình (như `facts()`,
`relocation()`), không có khái niệm của một ngành nào trong core.

## Bắt đầu khi

- SDK và hướng dẫn: ngay (bước 31 đã có OpenAPI đầy đủ và SDK TypeScript).
- Đồng bộ ngoại tuyến: sau 33–34 (đã xong). Module thương mại khai phần của mình ở bước của nó.
- WebSocket: sau 40 P5 (đã xong). Bước 61 (chat) dùng được ngay khi có.
- Mua trong ứng dụng: sau 35 và 36 (đã xong).

Song song được với 41–49. Ngoài phạm vi: IoT gửi dữ liệu dày (MQTT, cơ sở dữ liệu chuỗi thời gian) — khác mục tiêu
của core; GraphQL (bảng "Chưa làm, đã cân nhắc" của `STACK.md`).

## Thiết kế

### P1 — SDK Swift, Kotlin, Dart

- Sinh từ `docs/openapi.json` bằng công cụ chuẩn, ghim phiên bản: Swift dùng `swift-openapi-generator` (của Apple),
  Kotlin và Dart dùng `openapi-generator` (generator `kotlin` và `dart-dio`). Tra đúng phiên bản bằng skill
  `check-library-api` trước khi chọn.
- Đặt trong `sdk/swift`, `sdk/kotlin`, `sdk/dart`; file sinh tự động đánh dấu `linguist-generated` trong
  `.gitattributes` như `sdk/typescript/src/schema.ts`.
- `cargo xtask sdk` sinh lại cả bốn SDK. Gate không cần JDK hay Swift: gate chỉ kiểm cấu hình sinh và sự khớp với
  OpenAPI (như `sdk --check` hiện có); CI có job riêng biên dịch từng SDK.
- Đăng SDK lên kho gói (Swift Package Index, Maven Central, pub.dev) cần tài khoản và secret của chủ dự án: dừng
  hỏi, không tự làm.

### P2 — Đồng bộ khi mất mạng

Theo cách làm chuẩn của các hệ đồng bộ (luồng thay đổi theo con trỏ, ghi có điều kiện):

- **Luồng thay đổi:** module khai loại bản ghi đồng bộ được qua hook mới `Module::sync_resources()` (cuối trait, có
  mặc định rỗng). Kernel cho bảng `<module>_changes` (ghi trong CÙNG transaction với thay đổi, như outbox) và
  endpoint `GET /api/v1/<module>/<resource>/changes?cursor=…` trả các bản ghi đã đổi, kể cả dấu xoá (tombstone),
  theo `Page<T>`. Lọc theo khách, theo nút và theo permission đọc — giống REST thường.
- **Ghi lại khi có mạng:** dùng `Idempotency-Key` (đã có) để gửi lại không trùng, và `If-Match` với phiên bản bản ghi
  (`ETag`) để phát hiện xung đột. Xung đột trả `412` kèm mã lỗi ổn định và bản hiện tại trên máy chủ, để app tự gộp
  hay hỏi người dùng. Không tự gộp ở máy chủ.
- **Lô ghi:** `POST /api/v1/kernel/sync/batch` nhận nhiều lệnh ghi đã xếp hàng trên máy, chạy từng lệnh qua đúng
  service như REST (luật kiến trúc 8), trả kết quả từng lệnh; một lệnh hỏng không chặn lệnh khác.
- **Giữ thay đổi:** có thời hạn là cấu hình; con trỏ quá cũ thì trả `reset` để app tải lại từ đầu (như SSE).
- Chạy giống nhau trên SQLite và Postgres; con trỏ là UUIDv7 của dòng thay đổi.

### P3 — WebSocket hai chiều

- `GET /api/v1/kernel/events/ws`, dùng chung `StreamHub` và bộ lọc quyền của SSE (D-378): cùng event, cùng
  `Last-Event-ID` / con trỏ để nối lại, cùng giới hạn kết nối. Nhiều máy chủ vẫn đúng vì hub đọc từ bảng event.
- Máy khách gửi lên chỉ các lệnh điều khiển: đăng ký / huỷ đăng ký tên event, báo "đang gõ" và "đang online"
  (không lưu), xác nhận đã nhận. **Ghi dữ liệu vẫn qua REST** — mỗi việc chỉ một cách làm.
- Xác thực như REST (token hay API key) lúc bắt tay; token hết hạn thì máy chủ đóng kết nối với mã rõ ràng.
- SSE vẫn là cách mặc định cho trình duyệt; WebSocket cho app cần hai chiều (chat, vị trí shipper).

### P4 — Mua trong ứng dụng (App Store, Google Play)

- Trait `StorePurchases` của kernel: kiểm giao dịch với máy chủ của cửa hàng ứng dụng (App Store Server API, Google
  Play Developer API) và nhận thông báo từ họ (App Store Server Notifications v2, Google Real-time Developer
  Notifications) qua webhook có kiểm chữ ký. Bộ giả cho test.
- `plans` nối giao dịch đã kiểm vào thuê bao và quyền dùng của bước 35: mua, gia hạn, hết hạn, hoàn tiền, huỷ.
  Gọi lại nhiều lần không cấp trùng.
- Tiền do Apple / Google thu và giữ (đối tác có giấy phép) — đúng D-187, core không giữ tiền.
- Bộ nối thật cần tài khoản nhà phát triển và key của chủ dự án: dừng hỏi, không tự làm.
- Căn cứ pháp lý: thuế với hàng hoá, dịch vụ số bán qua nền tảng nước ngoài (Luật Quản lý thuế 38/2019/QH14 và văn
  bản hướng dẫn về nhà cung cấp nước ngoài) — **cần đối chiếu văn bản gốc / người có chuyên môn pháp lý**; ghi vào
  quyết định của bước.

### P5 — Hướng dẫn cho người làm app

`docs/guides/mobile.md` + `mobile.vi.md`: đăng nhập và làm mới token, đăng nhập Google / Microsoft / GitHub trên
điện thoại (PKCE với redirect của app), thông báo đẩy, tải ảnh thẳng lên S3, nhận event (SSE / WebSocket), đồng bộ
khi mất mạng, mua trong ứng dụng, dùng SDK. Ví dụ code chạy được như test (như các hướng dẫn hiện có).

## Test bắt buộc

- P1: `cargo xtask sdk --check` báo khi SDK lệch OpenAPI; job CI biên dịch ba SDK.
- P2: luồng thay đổi đúng thứ tự và đủ dấu xoá; con trỏ cũ trả `reset`; ghi với `If-Match` cũ trả `412` kèm bản
  hiện tại; lô ghi gửi lại hai lần không ghi trùng; không thấy bản ghi của khách hay nhánh khác; hai máy cùng ghi một
  bản ghi đồng thời — một bên thắng, bên kia nhận `412`. Cả SQLite và Postgres.
- P3: nhận đúng event được phép; nối lại không mất event; hai máy chủ dùng chung database; token hết hạn thì đóng;
  không ghi được dữ liệu qua WebSocket.
- P4: giao dịch giả hợp lệ cấp quyền dùng; thông báo gửi lại hai lần không cấp trùng; hoàn tiền thu hồi quyền; chữ ký
  sai bị từ chối. Cả SQLite và Postgres.

## Agent làm bước này tự quyết

Chọn đúng phiên bản công cụ sinh SDK, tên endpoint, cấu trúc payload, thời hạn giữ thay đổi; ghi quyết định cho
mỗi phần. Chỉ dừng hỏi khi cần tài khoản, key hay secret thật (kho gói, App Store, Google Play).
