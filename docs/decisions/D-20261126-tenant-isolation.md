## D-20261126-tenant-isolation · Cách ly khách lúc chạy: lớp kết nối gắn khách trên cả hai database, RLS Postgres tuỳ chọn, hạn mức không tin gợi ý khách — 2026-11-26 — agent quyết

Chủ đề: Đa khách, tổ chức, gói

Quyết định: (1) Lớp chính, cả SQLite và Postgres: handle theo khách của `db` (D-20261126-kernel-data-layer) — record cấp
khách chỉ truy vấn qua `TenantDb`, quên khách là lỗi biên dịch, quét mọi khách bằng `all_tenants("lý do")`, SQL viết tay
trên `TenantDb` phải có `:tenant`, `insert` kiểm `tenant_id`; mỗi thao tác có test "khách A không chạm được dòng của khách
B" trên cả hai database. (2) Phòng thủ thêm, chỉ Postgres, tuỳ chọn: `AUTOXIDE_POSTGRES_ROW_SECURITY=off|on` (mặc định
`off` tới khi đo chi phí); `on` thì migration tạo policy `FORCE ROW LEVEL SECURITY` cho mọi bảng `TenantTable` theo
`current_setting('autoxide.tenant', true)`, `TenantDb` đặt `set_config(…, true)` trong transaction; nhánh SQLite không làm
gì, giấu sau trait `RowSecurity`, test cả hai nhánh. (3) Lint `tenant-scope` gỡ khi module cuối chuyển sang `db`. (4) Hạn
mức của request chưa xác thực chỉ lấy khách từ thứ đã kiểm (tên miền đã xác minh, `Site` đã phân giải), không từ gợi ý của
client; không xác định được thì tính theo IP.

Lý do: `tenant-scope` là lint đọc text, có 64 chỗ miễn, không bắt được truy vấn dựng động. Lớp kết nối chạy trên cả hai
database nên đáp ứng luật số một; RLS chặn thêm SQL viết tay sai trên Postgres — nơi chạy nhiều khách chung một database —
nhưng không thể là cam kết duy nhất vì SQLite không có. Gợi ý khách chưa kiểm cho phép kẻ lạ tiêu hạn mức của khách khác.

Đã cân nhắc: chỉ RLS (không có trên SQLite, vi phạm luật số một); schema riêng mỗi khách trên Postgres (không có trên SQLite,
khó migrate hàng nghìn khách); giữ lint (không kiểm được lúc chạy). Thiết kế đầy đủ, API mẫu và thứ tự PR: `docs/research/kernel-contract-1-0.md`.
