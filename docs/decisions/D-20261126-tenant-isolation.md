## D-20261126-tenant-isolation · Cách ly khách lúc chạy: lớp kết nối gắn khách trên cả hai database, RLS Postgres tuỳ chọn, hạn mức không tin gợi ý khách — 2026-11-26 — agent quyết

Chủ đề: Đa khách, tổ chức, gói

Quyết định: (1) Lớp chính, cả SQLite và Postgres: handle theo khách của `db` (D-20261126-kernel-data-layer) — record cấp
khách chỉ truy vấn qua `TenantDb`, quên khách là lỗi biên dịch, quét mọi khách bằng `all_tenants("lý do")`, SQL viết tay
trên `TenantDb` phải có `:tenant`, `insert` kiểm `tenant_id`; mỗi thao tác có test "khách A không chạm được dòng của khách
B" trên cả hai database. Đóng các lối vượt: `scope = tenant|global` bắt buộc ở
`#[derive(Record)]` (có trường `tenant_id` mà khai `global` là lỗi biên dịch; lúc khởi động đối chiếu với `TenantTable`); `Db`,
`AllTenants`, `Db::sql` chỉ lấy từ `StartContext` (handle control-plane), `TenantContext::database()` thô bị gỡ; `DO UPDATE`
trên `TenantDb` tự thêm `WHERE tenant_id = <khách>` và va dòng của khách khác là lỗi; `tenant_id` không `set` được; `join`/
`sub`/`exists` lọc khách trên mọi record tham gia. (2) Row-level security trên Postgres: **sau 1.0** (lớp 1 đủ cho mục lộ
trình "lớp kết nối gắn khách hay RLS tuỳ chọn"); khi làm: `current_setting` rỗng là chặn, `doctor` báo role superuser/
`BYPASSRLS`, worker quét nhiều khách tự đặt cờ bỏ qua, policy dựng bằng bước đối chiếu idempotent lúc khởi động. (3) Lint `tenant-scope` gỡ khi module cuối chuyển sang `db`. (4) Hạn
mức của request chưa xác thực chỉ lấy khách từ thứ đã kiểm (tên miền đã xác minh, `Site` đã phân giải), không từ gợi ý của
client; không xác định được thì tính theo IP.

Lý do: `tenant-scope` là lint đọc text, có 64 chỗ miễn, không bắt được truy vấn dựng động. Lớp kết nối chạy trên cả hai
database nên đáp ứng luật số một; RLS chặn thêm SQL viết tay sai trên Postgres nhưng không thể là cam kết duy nhất vì SQLite không có; kiểm độc lập lần 1
(26/11/2026) đề nghị dời nó sau 1.0 để tập trung đóng các lối vượt của lớp 1. Gợi ý khách chưa kiểm cho phép kẻ lạ tiêu hạn mức của khách khác.

Đã cân nhắc: chỉ RLS (không có trên SQLite, vi phạm luật số một); schema riêng mỗi khách trên Postgres (không có trên SQLite,
khó migrate hàng nghìn khách); giữ lint (không kiểm được lúc chạy). Thiết kế đầy đủ, API mẫu và thứ tự PR: `docs/research/kernel-contract-1-0.md`.
