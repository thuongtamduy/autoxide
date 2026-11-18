## D-20261118-step57-analytics · Phân tích truy cập không cookie: hợp đồng khai báo, khoá phiên, gộp, đồng ý — 2026-11-18 — agent quyết

Chủ đề: Module nội dung, file, liên lạc

Quyết định (bước 57, đặc tả `docs/steps/57-analytics.md`, D-220):

1. **Hợp đồng khai báo ở kernel** (`autoxide_kernel::analytics`): `Module::analytics_events` → `AnalyticsEvent`
   (`browser` = website gửi; `server` = rút từ event `<module>.…` của chính module bằng hàm thuần, như `Module::facts`
   của bước 38) và `Module::funnels` → `Funnel`; cả hai mặc định rỗng (D-038). Kernel gom thành `AnalyticsCatalog`
   (đọc qua `StartContext::analytics_catalog`, gồm cả module đăng ký sau), kiểm lúc khởi động: tên `<module>.<việc>`,
   không trùng, sự kiện phía máy chủ chỉ nghe event đã khai của chính module, bước của phễu là loại đã khai.
   "Schema" của `props` là danh sách thuộc tính có kiểu (`text` ≤ 100 ký tự / `number` / `boolean`, tối đa 10) thay
   cho JSON Schema: đủ để chặn rác mà không thêm thư viện. Loại sự kiện của chính module: `analytics.page_view`
   (`object` tuỳ chọn). Tên loại luôn có tiền tố module (website gửi `catalog.product_viewed`).
2. **Khoá phiên**: khoá của ngày là chuỗi ngẫu nhiên lưu ở `analytics_day_keys` (theo khách, theo ngày UTC) và bị job
   dọn xoá khi ngày qua — mạnh hơn `SecretKey::derive("analytics.day.<ngày>")` của đặc tả, vì khoá dẫn xuất tính lại
   được bất kỳ lúc nào từ khoá gốc. Khoá phiên = HMAC-SHA256(khoá ngày, website | khối mạng /24 hay /64 | nhãn user
   agent `trình duyệt|hệ điều hành|loại máy` không phiên bản), 16 byte, base64url. Không lưu IP, không lưu user agent.
3. **Danh sách bot**: user agent không bắt đầu bằng `Mozilla/` / `Opera/`, hay chứa một mẩu đã biết (bot, crawl,
   spider, headless, curl, python-requests…, `client.rs`); không có user agent cũng là bot. Không chặn trình duyệt
   trong ứng dụng (Zalo, Facebook) vì đó là người thật.
4. **"Tên miền khớp website"**: trang gửi (`Origin`, không có thì `Referer`) phải là tên miền của request, tên miền đã
   xác minh của chính khách, hay có trong `AUTOXIDE_ANALYTICS_ALLOWED_HOSTS` (cấu hình mới, mặc định rỗng — cho chế độ
   một khách, nơi không có tên miền xác minh). Không có cả hai header → bỏ.
5. **Thân request**: giữ `ValidJson` (`application/json`). `navigator.sendBeacon` với Blob JSON không chạy ổn định
   trên mọi trình duyệt; tài liệu hướng dẫn `fetch(…, {keepalive: true})` — cùng tác dụng (gửi được khi trang đóng).
6. **Gộp**: job `analytics.rollup` mỗi giờ tính lại (xoá rồi ghi, một transaction) hôm nay, hai ngày trước và mọi ngày
   còn sự kiện thô chưa từng gộp; cột `day` (`YYYY-MM-DD`) có sẵn trên sự kiện thô để gộp / dọn bằng so sánh chuỗi,
   giống nhau trên hai database; `SUM` ép `BIGINT` (Postgres trả `numeric`). Thêm cột `bounces` vào `analytics_daily`
   và chiều `total`. Không dùng `reporting` (bước 38) để gộp: số liệu truy cập nằm trong database của khách, báo cáo đọc
   tại chỗ.
7. **Tỉ lệ thoát** = phiên chỉ xem đúng một trang / khách (phiên trình duyệt) của ngày, cộng các ngày. **Khách** của một
   khoảng = tổng khách của từng ngày (không cookie thì không nối được hai ngày). **Phễu**: mỗi bước đếm số phiên có
   sự kiện của bước trong ngày, độc lập — phiên ẩn danh không nối được với sự kiện phía máy chủ (đơn đã trả tiền);
   API ghi rõ bước sau có thể lớn hơn bước trước.
8. **Danh sách xếp hạng** (trang, nguồn, chiến dịch, đối tượng, loại) trả `Data<[…]>` với `limit` 1–100, không phân
   trang bằng cursor: là bảng xếp hạng top-N từ tổng của nhiều ngày, không phải danh sách bản ghi.
9. **Đồng ý**: đọc sổ đồng ý của `compliance` qua `dyn ConsentLedger` (mục đích `analytics.personal`, mọi phạm vi, mọi
   kênh) ở MỖI lần nhận và mỗi lần đọc "vừa xem" — rút là ngừng ngay. Event `compliance.consent_withdrawn` không nói
   của ai, nên bên nhận hỏi lại sổ cho mọi người còn gắn sự kiện (giới hạn bởi hạn lưu sự kiện thô) rồi gỡ `user_id`
   của người không còn đồng ý. Số gộp không chứa người, giữ nguyên. `auth.user_deleted` gỡ `user_id` của người đó.
10. **Permission**: chỉ `analytics.read` (theo nút). Bỏ `analytics.manage` của đặc tả: phễu do module khai bằng code,
    tên miền nhận do cấu hình — không còn việc gì cho nó. Không phát event `analytics.daily_rolled_up` (đặc tả ghi "nếu
    cần"; chưa ai cần). Tính năng gói: module `analytics` là module của gói (không `always_enabled`).

Lý do: bám đặc tả, giữ "không cookie, không truy ngược" thật sự (khoá ngẫu nhiên bị xoá), chạy giống nhau trên SQLite và
Postgres, không thêm thư viện.

Đã cân nhắc: khoá ngày dẫn xuất từ `SecretKey` (tính lại được, không thật sự "xoá"); JSON Schema cho `props` (thêm thư
viện, quá sức cho vài thuộc tính); phễu tuần tự theo phiên (không nối được phiên với đơn phía máy chủ); nhận `text/plain`
cho `sendBeacon` (phá luật `ValidJson`); gộp bằng `reporting` (kho của cây khách, không phải số truy cập theo website).

Căn cứ pháp lý (cần đối chiếu văn bản gốc / người có chuyên môn pháp lý): Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15 và
NĐ 356/2025 — gắn hành vi với người cần đồng ý theo mục đích, tối thiểu hoá; số gộp ẩn danh có là dữ liệu cá nhân không,
có cần banner khi không cookie không: chưa thống nhất.
