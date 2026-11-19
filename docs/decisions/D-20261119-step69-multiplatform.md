## D-20261119-step69-multiplatform · Bước 69: nền cho ứng dụng đa nền tảng (SDK di động, đồng bộ ngoại tuyến, WebSocket, mua trong ứng dụng) — 2026-11-19 — chủ dự án quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: thêm bước 69 vào lộ trình (đặc tả `docs/steps/69-multiplatform.md`), gồm bốn phần chủ dự án duyệt ngày
19/11/2026 sau khi rà backend có hợp với mảng khác ngoài web app không, cộng một hướng dẫn:

1. SDK Swift, Kotlin, Dart sinh từ OpenAPI bằng công cụ chuẩn, ghim phiên bản; gate kiểm khớp OpenAPI, CI biên dịch.
2. Đồng bộ khi mất mạng theo luồng thay đổi có con trỏ + ghi có điều kiện (`If-Match`, `412`) + `Idempotency-Key`;
   module khai qua hook mới `Module::sync_resources()` (cuối trait, có mặc định).
3. WebSocket hai chiều dùng chung `StreamHub` và bộ lọc quyền của SSE (D-378); chỉ nhận lệnh điều khiển, ghi dữ liệu
   vẫn qua REST.
4. Mua trong ứng dụng qua trait `StorePurchases` của kernel, `plans` nối vào thuê bao; tiền do Apple / Google giữ
   (D-187).
5. Hướng dẫn `docs/guides/mobile.md` + `.vi.md`.

Lý do: core chỉ có API nên app di động, desktop, POS dùng được ngay phần lớn; bốn chỗ trên là phần còn thiếu để dùng
trọn vẹn. Giữ trung lập với nghiệp vụ (khung ở kernel, module tự khai), chạy trên cả hai database, mỗi việc một cách
làm (SSE mặc định cho trình duyệt, WebSocket cho app hai chiều; ghi luôn qua REST). Không làm IoT dữ liệu dày (MQTT,
chuỗi thời gian) và GraphQL: khác mục tiêu của core.

Phụ thuộc: các bước nền (31, 33–37, 40) đã xong, nên bắt đầu được ngay, song song với 41–49. Bộ nối thật (kho gói
SDK, App Store, Google Play) cần tài khoản và secret của chủ dự án: dừng hỏi.
