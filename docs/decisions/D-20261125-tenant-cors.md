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

Cache và độ trễ: `DatabaseRegistry::verified_domain_owner` giữ ảnh chụp **toàn bộ** tên miền `verified` (tên
miền → khách) trong bộ nhớ — bảng tên miền chỉ nằm ở database điều khiển nên một câu đọc là đủ. Tra origin chỉ là
tra bản đồ: origin lạ không chạm database, không cần cache "không có" hay ngân sách truy vấn (bản đầu dùng ngân
sách 50 lần/giây, bị bỏ vì kẻ xấu dội origin ngẫu nhiên làm cạn ngân sách và origin hợp lệ bị từ chối theo).
Ảnh chụp đọc lại lười, tối đa một lần mỗi `AUTOXIDE_TENANCY_CACHE_SECONDS` (mặc định 10 giây), một bên đọc các bên
chờ; gỡ / xác minh / hỏng / dời khách (`move_domains`) / sửa khách trên máy này xoá ảnh chụp ngay
(`invalidate_domain`, `invalidate`), máy khác mất quyền chậm nhất sau TTL. Không đọc được database: từ chối
origin của khách (fail-closed). Ngưỡng: 100 000 tên miền (`MAX_VERIFIED_DOMAINS`, khoảng vài MB); vượt thì tên
miền mới nhất không có CORS và log lỗi — phương án dự phòng khi tới gần ngưỡng: chuyển sang tra từng origin qua
cache `domain_site` có giới hạn tần suất theo IP nguồn. Cache `domain_site` (theo `Host`) nay cũng giới hạn
10 000 mục (đầy thì bỏ mục hết hạn, vẫn đầy thì xoá sạch); trước đó không giới hạn. Chế độ một khách: không truy
vấn thêm nào.

Tài khoản nền tảng (người vận hành có `kernel.tenants.operate`) gọi từ origin của khách A: chỉ làm được cho A
(vào bằng `Autoxide-Tenant: A`); làm cho chính khách nền tảng hay vào khách B từ trang đó → `tenant_origin_mismatch`.
JavaScript trên tên miền do khách kiểm soát không được điều khiển quyền cấp nền tảng; giao diện của nền tảng đặt
origin ở danh sách chung.

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
