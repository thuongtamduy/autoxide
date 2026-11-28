## D-20261128-tenant-runtime-guard · Cách ly khách lúc chạy: chốt chặn câu SQL trên handle của khách, hạn mức chỉ nhận `SignedInTenant` — 2026-11-28 — agent quyết

Chủ đề: Đa khách, tổ chức, gói

Quyết định: Bổ sung lớp 1 của D-20261126-tenant-isolation (không thay; RLS vẫn sau 1.0) bằng hai việc, cả SQLite và Postgres:

1. **Chốt chặn lúc chạy** (`crates/kernel/src/data/db/guard.rs`). Mọi câu mà handle của khách (`tenant.db()`) gửi xuống
   database đi qua `guard::check` ngay trước khi chạy — câu của bộ dựng `db`, SQL viết tay `handle.sql(…)`, câu SeaORM module
   tự dựng chạy qua cầu `ConnectionTrait` chuyển tiếp, và câu của kernel nhận `&impl Executor` trên handle đó. Luật: **mỗi lần**
   một bảng cấp khách được dùng làm bảng (sau FROM / JOIN / UPDATE / USING, trong subquery, ở mỗi vế UNION, kể cả nối bảng với
   chính nó) phải có vế lọc **riêng** `<tên gắn>.tenant_id = <tham số>` (hay đảo hai vế; tên gắn = alias, không có alias thì
   tên bảng) mà tham số đó là **đúng khách của handle**; vế không ghi bảng `tenant_id = <tham số>` dùng được cho đúng một lần.
   Hai vế lọc trên cùng một alias không thay cho bảng thứ hai. Câu `INSERT` phải ghi cột `tenant_id` và gắn giá trị của khách;
   `ON CONFLICT … DO UPDATE` còn phải lọc khách ở nhánh cập nhật (bộ dựng `db` vốn thêm `WHERE <bảng>.tenant_id = …`). Câu
   không tham số (`execute_unprepared`) không gắn được khách nên bị từ chối trên handle của khách. Bảng cấp khách = (a) bảng của
   mọi record `scope = tenant` mà module khai (`Manifest::records`), ghi lúc dựng `Modules`, và (b) **mọi bảng có cột
   `tenant_id` trong schema thật** trừ bảng control-plane module khai, đọc ở `Modules::check_records` sau migration — nên chốt
   phủ cả bảng của module chưa chuyển sang record và bảng cấp khách của kernel khi câu chạm chúng đi qua handle của khách.
   Câu vi phạm không tới database: trả lỗi `internal_error` (lỗi lập trình, client không cần biết chi tiết),
   log `error` nêu tên bảng, đếm `kernel_tenant_guard_blocked_total`. Handle control-plane và `all_tenants("lý do")` không
   kiểm (không có khách để so).
2. **Khách chịu hạn mức là một kiểu riêng**: `tenancy::SignedInTenant`, chỉ dựng được từ `Principal` đã xác thực
   (`SignedInTenant::of`, loại nhân sự nền tảng vào khách khác và quyền hỗ trợ — luật cũ của `budget_tenant`).
   `EntitlementsHandle::count_request`, `request_quota_wait` và giới hạn request theo khách nhận kiểu đó thay `TenantId`, nên
   khách đọc từ gợi ý chưa kiểm (`Autoxide-Tenant`, `Host`) không truyền nhầm vào được. Hàm giải khách của request chưa đăng
   nhập đổi tên `unverified_tenant` (trước là `by_hint`), tài liệu nói rõ chỉ dùng chọn dữ liệu công khai. Đổi chữ ký hai
   hàm công khai của `EntitlementsHandle` trước 1.0 theo D-20261125-api-break-before-1-0 (chỉ kernel và một test e2e gọi).

Cách chốt đọc câu: tách token trên chữ SQL (bỏ chuỗi `'…'`, comment `--` và `/* */`, bỏ ép kiểu `::kiểu` của Postgres;
tên trong ngoặc kép giữ nguyên, tên trần đổi chữ thường; tham số `?` trên SQLite, `$n` trên Postgres — đúng dạng sea-query và
`db.sql` sinh), rồi so tên với sổ bảng và giá trị được gắn với khách. Không dùng thư viện phân tích SQL (vài chục dòng đủ, luật
"không thêm thư viện thừa"). Chốt **bắt câu quên lọc** (lớp lỗi mà lint `tenant-scope` tìm) ở lúc chạy, dù câu dựng bằng
đường nào; nó **không** cam kết chống câu cố tình vô hiệu bộ lọc (`… OR 1 = 1`) — code module chạy chung tiến trình và được tin.
Rào chữ `:tenant` cũ của `db.sql` giữ nguyên (báo lỗi rõ hơn, sớm hơn); chốt mới bắt cả trường hợp rào cũ cho qua (`:tenant`
chỉ nằm trong comment — có test). Giới hạn đã biết, ghi để không ai tưởng chốt làm được hơn: comment lồng nhau của Postgres
và chuỗi `$$…$$` lừa được bộ tách token (chỉ đáng lo với câu cố tình phá, không phải câu quên lọc); một số dạng lọc đúng
nghĩa bị chặn nhầm vì chốt không đọc chúng — `tenant_id IN (:tenant)`, `IS NOT DISTINCT FROM`, lọc nằm trong `WITH … INSERT`
— nên tài liệu của `sql` bảo viết đúng dạng `tenant_id = :tenant`. `SignedInTenant::of` nhận mọi `Principal`, kể cả tự dựng
(test cần): kiểu này chống truyền nhầm, không chống code cố ý. Lọc lặp trên cùng một tên gắn
(cùng alias, hay cùng tên bảng khi bảng dùng nhiều lần không alias) đếm thành nhiều vế, nên có thể bù cho một lần dùng
không lọc mang cùng tên gắn — chỉ xảy ra khi người viết cố tình lặp vế lọc. Phép gán `tenant_id = …` trong `SET` (UPDATE,
`DO UPDATE`) không được tính là lọc (kiểm độc lập lần 3: `ActiveModel::update` đủ trường của SeaORM gán `tenant_id` rồi
chọn dòng theo id, kéo dòng khách khác về); `INSERT … VALUES` phải có giá trị của khách đúng ở cột `tenant_id` của mọi
dòng. Tên bảng so không phân biệt hoa thường (SQLite coi `"NOTES_NOTE"` là `notes_note`). Lệnh chạy với
`Migrations::Ignore` (sinh `openapi`, `mcp_manifest`, một nhánh `backup`) không gọi `check_records`, nên sổ chỉ có bảng từ
record — các lệnh đó không phục vụ request của khách.

Phạm vi lúc này và đường nâng cấp: chốt chỉ chạy trên câu đi qua **handle** của khách. Module còn đi đường cũ
(`TenantContext::database()`, đếm ở baseline `raw-database` — đa số truy vấn của 38 module hôm nay) không qua handle nên chưa
được chốt — vẫn do lint `tenant-scope` canh; sổ bảng đã phủ bảng của chúng, nên câu nào của chúng chuyển sang handle (kể cả
qua cầu SeaORM) là được chốt ngay. Mỗi cụm chuyển sang `db` (D-20261128-library-layer-plan mục 6)
tự vào phạm vi chốt mà không phải làm gì thêm: record của cụm vào sổ, mọi câu của nó đi qua handle. Khi baseline rỗng và
`database()` thành nội bộ, mọi truy vấn của module đi qua chốt, lint `tenant-scope` gỡ (D-20261126-tenant-isolation mục 3).
RLS trên Postgres vẫn là tuỳ chọn sau 1.0 với bốn điều kiện đã ghi.

Khi sổ bảng mở rộng theo schema, chốt bắt ngay ba câu thật trên handle của khách: `orders` cập nhật `orders_payments` chỉ
theo khoá chính (`ActiveModel::update`, sửa thành `Entity::update(…).scoped(tenant)`), và hai test của `compliance`,
`conversations` đọc bảng của khách không lọc (sửa test). Kiểm độc lập lần 1 (PR #624) dò ra bản đầu chỉ đếm số bảng khác
nhau nên lọt subquery cùng bảng, self-join lọc một phía, vế UNION không lọc, lọc hai lần trên cùng alias; luật "mỗi lần dùng
một vế lọc riêng" ở trên và test cho cả bốn câu là bản sửa.

Module viết ngoài repo: không đổi API của `db` (handle, bộ dựng, `sql`); câu của họ được chốt như module trong repo khi khai
record trong `Manifest::records` (đã bắt buộc). Họ chỉ bị ảnh hưởng nếu đang chạy câu SeaORM không lọc khách trên handle của
khách qua cầu chuyển tiếp — chính lối vượt mà cầu đó sẽ bị gỡ (mục 5 của kế hoạch) — hay gọi hai hàm hạn mức ở trên.

Chi phí, đo được: một lần quét chữ của câu (O(độ dài câu)) và tra sổ dưới một khoá đọc, không thêm truy vấn. Đo bằng vòng lặp
100.000 lần trên câu `SELECT` nối 2 bảng dạng bộ dựng sinh (284 ký tự): ~2,5 µs mỗi câu ở bản release (~14 µs bản debug) —
cỡ một phần mười một truy vấn SQLite nhỏ, một phần trăm một vòng Postgres qua mạng; chưa đáng tối ưu thêm hay cache. Câu không
chạm bảng cấp khách vẫn quét một lần (không biết trước nó chạm bảng nào). Test cũ của lớp `db` (join, subquery, upsert `DO UPDATE`, keyset, `adjust`) chạy xanh với
chốt bật trên cả hai database, tức câu của bộ dựng đều đạt luật.

Lý do: lớp 1 lọc khách khi dựng câu, nhưng ba đường vẫn chạy câu không qua bộ lọc: SQL viết tay (rào chữ dễ lừa), cầu
`ConnectionTrait` (câu SeaORM tự dựng), và câu của kernel qua `Executor`. Mục lộ trình 71 đòi "chặn truy vấn thiếu
`tenant_id`" lúc chạy, không chỉ lint đọc mã — và phải chạy trên cả SQLite (luật số một), nên không dựa vào RLS. Kiểu
`SignedInTenant` biến luật "hạn mức chỉ tính người đã xác thực" (D-20261126-tenant-rate-limit, D-20261126-api-quota-signed-in)
từ quy ước thành thứ trình biên dịch kiểm.

Đã cân nhắc: chặn ở tầng kết nối cho cả đường cũ (bọc `DatabaseConnection` của SeaORM hay dùng backend proxy của nó) —
phải đổi kiểu trả về của `Database::read()` cho 38 module đang được chuyển theo cụm, đụng mọi file, và chuyển kết quả qua
proxy làm mất kiểu dữ liệu; cụm chuyển module làm việc đó đúng chỗ. Callback đo của SeaORM — chạy sau khi câu đã chạy, chỉ
phát hiện được, không chặn. Thư viện phân tích SQL đầy đủ — thêm phụ thuộc lớn cho việc vài chục dòng làm đủ. Sổ bảng chỉ từ
record (bản đầu) — kiểm độc lập chỉ ra nó chỉ phủ 7 bảng và để lọt mọi bảng chưa có record; đổi sang dò schema sống (cột
`tenant_id`, trừ bảng control-plane), lo ngại chặn nhầm câu của kernel trên bảng dùng chung không xảy ra vì câu đó chạy trên
kết nối control-plane, không qua handle của khách (test của 9 module xanh với sổ mới).
