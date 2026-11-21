## D-20261121-own-migrations · Kernel tự chạy migration, bỏ `sea-orm-migration` — 2026-11-21 — agent quyết

Chủ đề: Database, dữ liệu, job, event

Quyết định: `autoxide_kernel::migration` tự giữ phần nền mà `sea-orm-migration` 2.0 từng cung cấp
(`crates/kernel/src/data/migration/{manager,schema}.rs`): trait `MigrationName` / `MigrationTrait`, `SchemaManager`
(bọc `sea_orm::DatabaseExecutor`: `create_table`, `create_index`, `alter_table`, `drop_table`, `drop_index`,
`create_foreign_key`, `drop_foreign_key`, `rename_table`, `truncate_table`, `has_table`, `execute` / `exec_stmt`,
`get_connection`, `get_database_backend`), các hàm cột `schema::*` (chép đúng định nghĩa cũ), và bảng lịch sử
`<m>_migrations`. Prelude `autoxide_kernel::migration::prelude` giữ nguyên các tên module đang dùng, nên migration của
module không phải sửa. Bộ chạy (`Migrator::status` / `up` / `down`) như trước: mỗi migration schema chạy cùng dòng lịch
sử trong MỘT transaction trên cả hai database; upgrade dữ liệu (D-257) chạy ngoài transaction rồi ghi lịch sử; tên có
trong database mà không có trong code thì báo lỗi ("Migration file of version … is missing"); đọc trạng thái không tạo
bảng nào. Bảng lịch sử dựng từ cùng entity (`version` chuỗi làm khoá chính, `applied_at` giây Unix), nên database cũ
đọc tiếp được, không migration dữ liệu.

Lý do: `sea-orm-migration` 2.0.4 (mới nhất) khai `sea-orm` không tắt feature mặc định, nên luôn bật `with-chrono`,
`with-time`, `with-rust_decimal`, `schema-sync`: còn dùng nó thì `chrono` (RUSTSEC-2026-0543) không ra khỏi cây
(D-20261121-time-library). Phần Autoxide dùng nhỏ (vài trăm dòng) và ổn định. Gỡ nó cũng bỏ `time`, `sea-schema` khỏi
cây phụ thuộc.

Đã cân nhắc: chờ upstream tắt feature mặc định (không có mốc); fork / `[patch]` (`deny.toml` chỉ nhận crates.io); viết
bộ chạy mới khác cách cũ (bảng lịch sử khác dạng thì cần migration dữ liệu và bản cũ không đọc được).

