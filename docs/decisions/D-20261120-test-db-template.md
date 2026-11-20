## D-20261120-test-db-template · Test chép database mẫu đã migrate thay vì migrate mọi module ở mỗi bài — 2026-11-20 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. `TestApp::build` (testkit) và các helper dựng chương trình thật trong test của `autoxide` gọi
   `autoxide_testkit::prepare_from_template(&config.database, &migrator)` trước khi mở database: migration chạy MỘT
   lần vào một database mẫu cho mỗi khoá, rồi mỗi bài test chép mẫu vào database của nó. Sau đó `migrator.up` vẫn chạy
   như cũ (không còn gì để làm khi đã chép; làm hết việc khi không chép).
   - SQLite: file mẫu `<tạm>/autoxide-templates/<khoá>.sqlite`; chép ra file tạm cạnh đích rồi đổi tên.
   - Postgres: database `autoxide_tpl_<khoá>_<UUIDv7>` đánh dấu `IS_TEMPLATE true`, `ALLOW_CONNECTIONS false`; bài test
     `DROP DATABASE` database trống của mình rồi `CREATE DATABASE <cùng tên> TEMPLATE <mẫu>` (URL không đổi).
2. Khoá của mẫu: loại database, `Migrator::names()` (mọi migration và upgrade theo thứ tự chạy, hàm mới của kernel),
   chương trình test đang chạy (đường dẫn, kích thước, thời điểm sửa — build lại thì khoá đổi, nên sửa một migration
   chưa commit cũng không dùng nhầm mẫu cũ) và `Migrator::uses_configured_key()` (không đặt `AUTOXIDE_SECRET_KEY` thì
   upgrade dữ liệu sinh khoá gốc vào database đang migrate). Dựng mẫu dưới lock: lock file (`File::lock`, SQLite) hay
   `pg_advisory_lock` (Postgres); mẫu chỉ được nhận khi đã dựng xong (đổi tên file / đánh dấu `IS_TEMPLATE` sau cùng).
3. Chỉ chép khi chắc database của bài test còn nguyên: SQLite thì file chưa tồn tại (SQLite tạo file ở kết nối đầu),
   Postgres thì không có bảng / sequence / view nào và `DROP DATABASE` (không `FORCE`) thành công — tức không ai đang
   kết nối. Ngoài ra, hay lỗi bất kỳ, thì không chép và bài test migrate như trước. Test cần chính lần migrate trên
   database trống: `TestAppBuilder::empty_database()`; test tự mở database và tự migrate (vd `e2e::migrations`, test
   của kernel) không đi qua mẫu nên không đổi. `TEST_DATABASE_TEMPLATE=off` tắt mẫu cho mọi test (đo, loại trừ).
4. Dọn: mẫu cũ hơn một giờ (`STALE_AFTER`) bị xoá cùng lúc dọn database test sót; bài test đang dùng mẫu vừa bị xoá thì
   dựng lại.

Lý do: test đầu-cuối của `autoxide` (132 test, mỗi test dựng mọi module) là phần chậm nhất của gate trên SQLite
(D-20261119-ci-faster). Đo trên máy 4 CPU, `cargo test -p autoxide --bin autoxide` (chương trình đã build sẵn):

| Database                       | Không mẫu (`TEST_DATABASE_TEMPLATE=off`) | Có mẫu |
| ------------------------------ | ----------------------------------------- | ------ |
| SQLite (4 thread)              | 849 s                                     | 406 s  |
| Postgres 16 (`--test-threads=4`, như `test-all`) | 263 s                   | 192 s  |

Đã cân nhắc:
- Migrate một lần mỗi tiến trình rồi dùng chung một database cho mọi test: test không còn độc lập (dữ liệu lẫn nhau).
- Postgres `CREATE DATABASE … STRATEGY FILE_COPY`: cần checkpoint mỗi lần; mặc định `WAL_LOG` đủ nhanh với mẫu nhỏ.
- SQLite `VACUUM INTO` / backup API: sqlx không mở backup API, `unsafe` bị cấm; chép file sau khi đóng hết kết nối
  (WAL đã gộp) là đủ và nhanh nhất.
- Tạo database của bài test muộn (lúc `build`) thay cho chép đè: đổi URL giữa chừng, phá test đã lấy `url()` trước.
- Khoá theo nội dung chương trình test (băm cả file): mỗi tiến trình băm hàng trăm MB; đường dẫn + kích thước + thời
  điểm sửa đủ để phân biệt hai bản build.

Hệ quả: các bài test dùng chung khoá gốc tự sinh (nằm trong mẫu) thay vì mỗi bài một khoá — như hai máy chủ dùng chung
database. Thư mục tạm có thêm `autoxide-templates/`; Postgres dùng lâu có thêm database `autoxide_tpl_…` (tự dọn sau
một giờ). Placement riêng của test nhiều khách (`tenant_dedicated`, `placement`) vẫn migrate từ đầu.
