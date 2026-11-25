## D-20261125-tenant-cors · CORS theo tên miền đã xác minh của khách, gắn với đúng khách — 2026-11-25 — agent quyết

Chủ đề: Đa khách, tổ chức, gói

Quyết định: Ở chế độ nhiều khách, một origin được CORS khi nằm trong `AUTOXIDE_HTTP_CORS_ORIGINS` (giữ
nguyên, kể cả `*` và khi không đặt) **hoặc** đúng bằng `https://<tên miền>` của một tên miền `verified` trong
`kernel_tenant_domains`. Lớp `http::cors::classify` quyết một lần mỗi request và để `OriginGrant` (`Listed` |
`Tenant(id)`) trong extensions; header CORS (`tower-http`, predicate chỉ đọc kết quả đó), kiểm `Origin` của MCP
và kiểm khách (`tenancy::request::ensure_origin`) cùng đọc nó. Request thật mang origin của khách A mà khách giải
được (credential, `Autoxide-Tenant`, `Host`) là B → `403 tenant_origin_mismatch`, trước khi handler chạy.
Không thêm tenant setting cho origin phụ.

Mô hình đe doạ và cách chặn:

- Preflight không có credential nên chỉ biết "origin thuộc một khách đã xác minh"; quyền thật gắn với khách ở
  request thật (lỗi ở extractor, trước handler, nên không có tác dụng phụ). Token là bearer trong header, không
  có cookie, không bao giờ gửi `Access-Control-Allow-Credentials` — giữ nguyên mức cũ.
- Không phản chiếu origin tuỳ ý: origin của khách phải đúng dạng trình duyệt gửi (`https`, chữ thường,
  ASCII/punycode, không port, không dấu chấm cuối, không đường dẫn); `null`, `http`, port, chữ hoa, tên miền
  con, hậu tố/tiền tố đều bị từ chối. Danh sách chung so khớp nguyên byte như trước. `Vary: origin` luôn có.
- Chỉ tên miền đã chứng minh sở hữu bằng TXT (`verified`) mới được; `pending` / `failed` / `revoked` thì không.
  Một tên miền chỉ `verified` cho một khách (unique index), nên khách không tự cấp CORS cho tên miền của khách
  khác; dù có thì quyền chỉ dùng được cho chính khách đó.
- MCP giữ chống DNS rebinding: có `Origin` mà không có `OriginGrant` → 403; origin của khách gắn với khách.

Cache và độ trễ: tái dùng cache tên miền của `DatabaseRegistry` (`domain_site`, cache cả kết quả "không có"),
TTL `AUTOXIDE_TENANCY_CACHE_SECONDS` (mặc định 10 giây). Gỡ / xác minh / hỏng / dời khách (`move_domains`) xoá
mục cache ngay trên máy làm việc đó; máy khác mất quyền chậm nhất sau TTL. Cache này nay giới hạn 10 000 mục
(đầy thì bỏ mục hết hạn, vẫn đầy thì xoá sạch) — trước đó không giới hạn và `Host` lạ cũng làm nó phình. Origin
chưa có trong cache tốn một lần đọc theo index của database điều khiển, tối đa 50 lần mỗi giây mỗi tiến trình
(`cors::LOOKUPS_PER_SECOND`); vượt thì từ chối origin đó lúc này (fail-closed, trình duyệt thử lại sau), nên
dội origin ngẫu nhiên (preflight không qua rate limiting) không dội được database. Chế độ một khách: không truy
vấn thêm nào.

Lý do: SaaS nhiều khách, mỗi khách một giao diện ở tên miền riêng; bắt người vận hành sửa `.env` và khởi động lại
cho mỗi khách không dùng được. Tên miền đã xác minh là bằng chứng sở hữu sẵn có, đã có cache và đã có chỗ xoá
cache khi đổi.

Đã cân nhắc:

- Tenant setting "origin thêm" do admin khách đặt: preflight không biết khách nên phải tra ngược origin → khách
  trên setting nằm ở database riêng của từng khách (nhiều placement) — không có index chung, không cache được rẻ;
  và cho khách tự khai origin chưa chứng minh sở hữu. Tên miền phụ (`admin.shop.example`) thì xác minh thành
  một tên miền riêng; `http://localhost:5173` lúc phát triển thì người vận hành đặt ở danh sách chung.
- `AllowOrigin::async_predicate` tra database trong lớp CORS: phải tra lần nữa cho MCP và kiểm khách; một lớp
  phân loại để lại kết quả trong extensions thì chỉ tra một lần.
- Suy khách từ `Origin` khi request không nêu khách: `Origin` do máy gửi tuỳ ý, thêm một đường chọn khách mà
  không thêm gì cho an toàn; giao diện vẫn gửi token hay `Autoxide-Tenant` như trước.
