## D-20261125-tenant-cors · CORS theo tên miền đã xác minh của khách, gắn với đúng khách — 2026-11-25 — agent quyết

Chủ đề: Đa khách, tổ chức, gói

Quyết định: Ở chế độ nhiều khách, một origin được CORS khi nằm trong `AUTOXIDE_HTTP_CORS_ORIGINS` (giữ
nguyên, kể cả `*` và khi không đặt) **hoặc** đúng bằng `https://<tên miền>` của một tên miền `verified` trong
`kernel_tenant_domains`. Lớp `http::cors::classify` quyết một lần mỗi request và để `OriginGrant` (`Listed` |
`Tenant(id)`) trong extensions; header CORS (`tower-http`, predicate chỉ đọc kết quả đó), kiểm `Origin` của MCP
và kiểm khách (`tenancy::request::ensure_origin`) cùng đọc nó. Không thêm tenant setting cho origin phụ.

Gắn với khách:

- Preflight không có credential nên chỉ biết "origin thuộc một khách đã xác minh". Request thật mang origin của
  khách A mà khách giải được (credential, `Autoxide-Tenant`, `Host`) là B → `403 tenant_origin_mismatch`, trong
  extractor `TenantContext` / `Principal` / `OptionalTenant`, trước khi handler chạy.
- Request không nêu khách nào (không `Autoxide-Tenant`, `Host` không thuộc khách nào — thường là `api.<nền tảng>`)
  mà mang origin của A: khách của request là A (`by_hint`), và `TenantContext` được đánh dấu "đến từ trang của
  khách" (`reached_from_page`). Nhờ vậy endpoint lấy khách từ token trong thân hay query (`OptionalTenant` +
  `resolve_link_token`: `/auth/refresh`, `/auth/login/mfa`, link trong mail, lời mời, đặt lại mật khẩu, endpoint
  công khai của comments / newsletter / notifications / inventory) cũng gắn với A: `resolve_link_token` trả "không
  có" cho token của khách khác **chỉ khi** khách của request được đánh dấu như vậy. Khách đến từ credential, header
  hay `Host` thì token vẫn thắng như trước — người đang đăng nhập ở khách X (hay giữ `Autoxide-Tenant: X` trên sàn)
  mở link huỷ nhận thư của khách Y vẫn được. Tải bản xuất bằng link (`/kernel/exports/download`) kiểm `OriginGrant`
  sau khi giải token.
- Tài khoản nền tảng (người vận hành có `kernel.tenants.operate`) gọi từ origin của khách A: chỉ làm được cho A
  (vào bằng `Autoxide-Tenant: A`); làm cho khách nền tảng hay vào B từ trang đó → `tenant_origin_mismatch`.

Mức độ của ràng buộc: đây là **phòng thủ thêm ở trình duyệt, không phải ranh giới bảo mật**. Token là bearer
trong header, không có cookie, không bao giờ gửi `Access-Control-Allow-Credentials`: trang của A chỉ gửi được
bí mật nó đã có, và máy gọi máy (không trình duyệt) đặt `Origin` tuỳ ý hay bỏ hẳn. Ràng buộc chặn trường hợp trang
của A cầm sẵn bí mật của B (token rò, link dán nhầm) dùng nó và đọc được kết quả trong trình duyệt. Không bị ràng
buộc: request không có `Origin` (máy gọi máy, trang cùng origin với API), origin trong danh sách chung, SCIM (máy
gọi máy, khách lấy từ token, không có trình duyệt), endpoint không giải khách (`/health`, `openapi.json`,
`/kernel/domains/allowed`), WebSocket (`http/ws.rs`, không kiểm `Origin`; xác thực bằng header `Authorization`
mà trình duyệt không gửi được khi mở WebSocket — ghi việc ở P16).

So khớp: origin của khách phải đúng dạng trình duyệt gửi (`https`, chữ thường, ASCII/punycode, không port, không
dấu chấm cuối, không đường dẫn); `null`, `http`, port (kể cả `:443`), chữ hoa, tên miền con, hậu tố/tiền tố đều
bị từ chối, không bao giờ phản chiếu origin tuỳ ý. Danh sách chung so khớp nguyên byte như trước. `Vary: origin`
luôn có. Chỉ tên miền đã chứng minh sở hữu bằng TXT (`verified`) mới được; `pending` / `failed` / `revoked` thì
không; một tên miền chỉ `verified` cho một khách (unique index).

Cache và độ trễ (`tenancy::verified_domains`): ảnh chụp **toàn bộ** tên miền `verified` (tên miền → khách) trong
bộ nhớ — bảng tên miền chỉ nằm ở database điều khiển nên một câu đọc là đủ. Origin lạ chỉ là tra bản đồ, không chạm
database (bản đầu dùng ngân sách 50 lần tra/giây, bị bỏ vì dội origin ngẫu nhiên làm cạn ngân sách và origin hợp lệ
bị từ chối theo).

- Đọc lại lười, tối đa một lần mỗi `AUTOXIDE_TENANCY_CACHE_SECONDS` (mặc định 10 giây), một bên đọc, các bên
  chờ dùng kết quả (test: 32 lượt tra đồng thời gây một lần đọc).
- Gỡ / xác minh / hỏng / dời khách (`move_domains`) gọi `invalidate_domain` → xoá ảnh chụp ngay trên máy đó và
  tăng số thế hệ: lượt đọc bắt đầu trước thay đổi không lưu kết quả cũ. Sửa khách (`invalidate(tenant)`) không
  đụng ảnh chụp. Máy khác mất quyền chậm nhất sau TTL.
- Không đọc được database, hay lượt đọc quá 3 giây (`READ_TIMEOUT`, database treo): từ chối origin của khách
  (fail-closed); lỗi được nhớ 1 giây để các request đang chờ
  trả ngay thay vì lần lượt hỏi lại database đang hỏng (lớp phân loại đứng ngoài rate limiting).
- Ngưỡng 100 000 tên miền (`MAX_VERIFIED_DOMAINS`, khoảng vài MB); vượt thì tên miền mới nhất không có CORS và log
  lỗi — phương án dự phòng khi tới gần ngưỡng: tra từng origin qua cache `domain_site` có giới hạn tần suất theo
  IP nguồn.
- Chế độ một khách: không truy vấn thêm nào. Cache `domain_site` (theo `Host`) nay giới hạn 10 000 mục (đầy thì
  bỏ mục hết hạn, vẫn đầy thì xoá sạch); trước đó không giới hạn.

Lý do: SaaS nhiều khách, mỗi khách một giao diện ở tên miền riêng; bắt người vận hành sửa `.env` và khởi động lại
cho mỗi khách không dùng được. Tên miền đã xác minh là bằng chứng sở hữu sẵn có, đã có chỗ xoá cache khi đổi.

Đã cân nhắc:

- Tenant setting "origin thêm" do admin khách đặt: preflight không biết khách nên phải tra ngược origin → khách
  trên setting nằm ở database riêng của từng khách (nhiều placement) — không có index chung, không cache được rẻ;
  và cho khách tự khai origin chưa chứng minh sở hữu. Tên miền phụ (`admin.shop.example`) thì xác minh thành
  một tên miền riêng; `http://localhost:5173` lúc phát triển thì người vận hành đặt ở danh sách chung.
- `AllowOrigin::async_predicate` tra database trong lớp CORS: phải tra lần nữa cho MCP và kiểm khách; một lớp
  phân loại để lại kết quả trong extensions thì chỉ tra một lần.
- Không suy khách từ `Origin` (bản đầu): để endpoint lấy khách từ token không gắn với khách của trang. Suy khách
  chỉ khi request không nêu khách nào; `Origin` do máy gửi tuỳ ý cũng chỉ chọn được khách như header
  `Autoxide-Tenant` vốn đã cho.
