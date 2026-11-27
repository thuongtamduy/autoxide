## D-20261127-db-read-within · Đọc có trần thời gian trong lớp `db` của kernel: `within`, database dừng câu — 2026-11-27 — agent quyết

Chủ đề: Database, dữ liệu, job, event

Quyết định: lớp `db` của kernel có `within(limit)` cho câu đọc trên handle đọc — của bộ dựng câu (`Find::within`,
dùng cả khi `select(…)`: `tenant.db().read().find::<R>()….within(…).all()`) và của câu viết tay (`Sql::within`).
Database tự dừng câu khi nó chạy quá `limit` và khi người gọi thôi chờ (future bị drop). Câu bị dừng trả lỗi `503 db.query_timeout` (`db::QUERY_TIMEOUT`,
`db::is_query_timeout`), nhận ra bằng mã lỗi của database, không bằng đồng hồ.

- Postgres: transaction chỉ đọc với `SET LOCAL statement_timeout` (chỉ trong transaction đó, kết nối về pool không
  đổi); mã `57014`. Future bị drop giữa chừng: gửi `pg_cancel_backend(pid)` của kết nối đó trên một kết nối khác của
  pool (sqlx không tự huỷ câu khi future bị drop).
- SQLite: lấy một kết nối của pool đọc, gắn progress handler bằng API an toàn của sqlx
  (`LockedSqliteHandle::set_progress_handler`). Handler trả `false` khi quá hạn hay khi cờ dừng bật, SQLite dừng
  câu với `SQLITE_INTERRUPT` (mã `9`). Gỡ handler trước khi trả kết nối về pool. Future bị drop: bật cờ dừng và đóng
  kết nối thay vì trả về pool (handler còn gắn trên nó).
- Giá trị bind trên SQLite đi thẳng qua sqlx, như sea-query-sqlx: bool, số nguyên (cả không dấu của `LIMIT`), số
  thực, chuỗi, byte, UUID, ngày giờ của kernel (biến thể `Enum` mang kiểu, bind là chuỗi). Kiểu khác là lỗi nội bộ,
  không đoán.
- `within` trên transaction ghi là lỗi: câu ghi không bị cắt giữa chừng.

Test: `crates/kernel/tests/integration/db_within.rs` và `db_layer.rs` (`doc_co_tran_tren_bo_dung_cau`), cả hai
database. Câu quá trần bị dừng và mọi kết nối của pool
dùng lại được. Future bị drop thì câu dừng thật: Postgres không còn tiến trình chạy câu; SQLite đóng xong kết nối
dưới 3 giây, trong khi sqlx bỏ chờ sau 5 giây nếu câu còn chạy. Đã thử bỏ phần huỷ: cả hai nhánh đều đỏ.

Lý do: tìm chữ công khai của `content` (PT-15, D-20261126-content-word-index, câu dựng bằng bộ dựng câu) cần trần thời
gian thật trên cả hai database. Trần ở tầng task (`tokio::time::timeout`) chỉ thôi chờ, câu vẫn chạy nốt và giữ kết
nối. Đặt trong kernel thì module không cần sqlx (D-20261126-library-isolation), và mọi câu tìm có chi phí do người gọi quyết sau này dùng
chung một cách.

Đã cân nhắc:

- `sqlite3_interrupt` trực tiếp: sqlx-sqlite 0.9 không có API an toàn cho nó, và `unsafe` bị cấm.
- Progress handler gắn sẵn lên mọi kết nối đọc (lúc mở pool): mọi câu đọc chịu một trần chung, kể cả câu không cần,
  và trần không theo từng câu.
- Đo thời gian để đoán lỗi là quá hạn: lỗi khác xảy ra gần hạn bị báo nhầm thành quá hạn (review của #573).
- Chỉ trần `statement_timeout`, không huỷ khi drop: câu Postgres chạy tới trần dù không ai cần kết quả.
