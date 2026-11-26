## D-20261126-kernel-data-layer · Lớp dữ liệu `autoxide_kernel::db`: `Record` có cột mang kiểu, handle theo khách, migration là kế hoạch khai báo — 2026-11-26 — agent quyết

Chủ đề: Database, dữ liệu, job, event

Quyết định: Module truy vấn qua `autoxide_kernel::db` thay SeaORM/sea-query. (1) `#[derive(Record)]` (crate
`autoxide-macros`, kernel xuất lại) thay entity SeaORM; `scope = tenant|global` bắt buộc; cột là hằng có kiểu
`Note::TITLE: Col<Note, String>`, cột `tenant_id` kiểu `TenantKey` không `set` được; không có `ActiveModel`. (2) Thao tác: `find`/`filter`/`order_by`/`one`/`all`/`count`/`exists`, `page*` (thay
`fetch_page*`), `select`+`group_by`+hàm gộp, `join` chỉ giữa record cùng module (kiểm lúc dựng câu), `for_update`
(SQLite bỏ qua: transaction ghi đã khoá), `for_share`, `insert`/`insert_many`/`on_conflict` (DO UPDATE tự thêm điều kiện
khách), `update().set`/`.adjust` có cận, `delete`, `lock_key`, `sub` + `in_sub`, `Cond::any/all`, hàm gộp ép `BIGINT`, `Expr`
tập nhỏ; `Outbox::record`, `SyncLog`, kiểm quyền ghi nhận `&TenantTx`. Đã chạy trên SQLite và Postgres ở spike (a). Lỗi là `Error` của kernel; unique đọc bằng `error.conflict()`. (3) Handle quyết định khách: `tenant.db()` tự lọc
record `tenant`; `database.db()` không nhận record `tenant` (lỗi biên dịch); quét mọi khách bằng
`all_tenants("lý do")` (có metric); `Db`/`AllTenants` chỉ lấy từ `StartContext`. (4) SQL viết tay: `db.sql(…)` với `#[derive(FromRow)]`; trên `TenantDb` bắt buộc có
tham số `:tenant`. (5) Migration: trait `Migration` mới với `up/down(&self, plan: &mut Plan)` đồng bộ, ghi bước vào
`Plan` (bảng, index, `plan.sql`, `plan.data`), kernel dựng SQL cho từng database. (6) Bên trong dựng trên sea-query +
kết nối SeaORM; không kiểu nào của chúng ở chữ ký công khai. `db` nằm ngay ở đường ổn định `autoxide_kernel::db` (không qua `unstable::`, vì nâng hạng sau này là dời đường dẫn); cam kết SemVer từ 1.0, và 1.0 đòi mọi module đã chuyển. `#[derive(Record)]` kiểm lúc biên dịch tiền tố bảng = tên module (từ `CARGO_PKG_NAME`, hay `[package.metadata.autoxide] module` cho crate ngoài) — luật kiến trúc 1 thành lỗi biên dịch cho mọi thao tác. Kiểm `:tenant` của `db.sql` là rào phụ (dò chuỗi), số chỗ dùng theo baseline chỉ giảm. Đường cũ (`Database::read/write/begin_write` trả kiểu SeaORM,
`fetch_page*`, `ScopeExt`, `TenantScoped`, `MigrationTrait`, `migration::prelude`, `database::statement`) giữ
`#[deprecated]` tới khi mọi module chuyển, gỡ trước 1.0.

Lý do: module gọi SeaORM ~1.970 chỗ và migration dùng DSL sea-query; bản lớn của hai thư viện đó hiện buộc mọi module sửa
(D-20261126-library-isolation). Cột có kiểu bắt thêm lỗi lúc biên dịch; handle theo khách biến lint đọc text (`tenant-scope`,
64 chỗ miễn) thành kiểm của trình biên dịch trên cả hai database; kế hoạch migration là dữ liệu nên luật an toàn migration và
lệnh lùi phiên bản của G3 đọc được trực tiếp. Tên mới (`db()`) thay vì đổi kiểu trả về của `read()` để không phá 29 module
cùng lúc.

Đã cân nhắc: xuất lại SeaORM qua kernel (module vẫn viết theo API thư viện — đã bị loại ở D-20261126-library-isolation); tự
viết ORM đầy đủ cả phần chạy (lớn, không cần: bên trong đổi được sau); bọc kiểu nhưng giữ `ActiveModel` (kéo theo toàn bộ
mô hình của SeaORM). Thiết kế đầy đủ, API mẫu và thứ tự PR: `docs/research/kernel-contract-1-0.md`.

Thay: phần "SeaORM là mặc định" của luật dữ liệu khi module cuối cùng đã chuyển (CLAUDE.md sửa ở PR đó); D-20261121-raw-statement khi `db.sql` có
