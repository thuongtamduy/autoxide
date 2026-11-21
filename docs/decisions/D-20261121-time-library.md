## D-20261121-time-library · Thay `chrono` / `chrono-tz` / `humantime` / `paste`: `jiff` qua lớp `autoxide_kernel::time`, kernel tự giữ migration và MCP server, S3 bằng `rusty-s3` — 2026-11-21 — agent quyết

Chủ đề: Database, dữ liệu, job, event

Quyết định: chủ dự án yêu cầu bỏ hết thư viện bị `cargo deny` gắn "không còn bảo trì" (RUSTSEC-2026-0543 `chrono`,
RUSTSEC-2026-0555 `chrono-tz`, RUSTSEC-2026-0542 `humantime`, RUSTSEC-2024-0436 `paste`), chọn thứ được bảo trì lâu dài,
không có thì tự viết. Đích: gỡ bốn dòng `ignore` khỏi `deny.toml`.

1. **Thời gian: `jiff` 1.0 cho mọi thứ** (thời điểm, ngày, múi giờ IANA, khoảng thời gian, tính lịch). Kernel có
   `autoxide_kernel::time::{Timestamp, Date}` bọc `jiff::Timestamp` / `jiff::civil::Date` và tự cắm vào SeaORM / sqlx
   (`crates/kernel/src/values/time/`); múi giờ (`jiff::tz::TimeZone`), khoảng (`jiff::SignedDuration`), lịch
   (`jiff::civil`) dùng thẳng `jiff`. Cơ sở dữ liệu múi giờ nhúng trong binary (`tzdb-bundle-always`), không đọc
   `/usr/share/zoneinfo` — mọi máy tính giờ địa phương như nhau, như `chrono-tz`.
2. **Ba thứ không đổi, có test giữ** (`values/time/tests.rs` so từng ký tự với `chrono`; `tests/integration/time.rs` chạy
   trên cả hai database):
   - *Dạng lưu trên đĩa.* `chrono` qua `sqlx` ghi cột SQLite là chuỗi `2026-11-21T10:00:00.123456+00:00` (phần lẻ
     0/3/6/9 chữ số, đuôi `+00:00`), Postgres là `timestamptz`. Kiểu mới ghi đúng chuỗi đó; dòng cũ và mới trộn trong
     một cột vẫn sắp xếp, lọc `>`/`BETWEEN`/`=` đúng; đọc được mọi dạng `sqlx` + `chrono` đã đọc (kể cả
     `CURRENT_TIMESTAMP` của SQLite, số giây Unix, ngày Julian). Không cần migration dữ liệu.
   - *JSON*: RFC 3339 UTC đuôi `Z`, phần lẻ 0/3/6/9 chữ số; nhận vào mọi độ lệch giờ; ngày `YYYY-MM-DD`.
   - *OpenAPI*: `utoipa` nhận kiểu theo TÊN lúc biên dịch (feature `jiff_0_2`: `Timestamp` → `string`/`date-time`,
     `Date` → `string`/`date`); feature này không kéo crate nào. Trường giờ trong ngày (`NaiveTime`, ít chỗ) khai
     `#[schema(value_type = String)]` cho ra đúng `string` như cũ.
3. **Cách cắm vào SeaORM** (vì thư viện không có sẵn cách đúng dạng cũ): khi ghi, giá trị đi dưới dạng
   `sea_query::Value::Enum { type_name: "timestamptz" | "date", value: <chuỗi dạng cũ> }` — biến thể duy nhất mà SQLite
   nhận nguyên chuỗi còn Postgres nhận `$1::"timestamptz"` (Postgres tự đọc chuỗi RFC 3339). Cột không khai là enum
   (`enum_type_name` = `None`) nên SeaORM không ép kiểu cột khi đọc. Khi đọc, `TryGetable` lấy dòng sqlx bên dưới
   (`QueryResult::try_as_pg_row` / `try_as_sqlite_row`, đều công khai) và giải mã bằng `sqlx::Decode` tự viết (Postgres
   nhị phân: micro giây từ 2000-01-01 UTC).
4. **Độ chính xác**: `Timestamp::now()` và `From<jiff::Timestamp>` cắt về micro giây (độ chính xác của Postgres) — giá trị
   trong bộ nhớ bằng giá trị đọc lại trên cả hai database (trước đây SQLite giữ nano giây, Postgres cắt: hai database
   khác nhau). Giá trị đọc từ database giữ nguyên độ chính xác đã lưu (dòng SQLite cũ có nano giây): cắt bớt thì
   `WHERE at > <giá trị vừa đọc>` lại ra chính dòng đó (phân trang lặp).
5. **`sea-orm-migration` → kernel tự giữ phần đang dùng.** `sea-orm-migration` 2.0.4 khai `sea-orm` không tắt feature
   mặc định nên luôn bật `with-chrono` (+ `with-time`, `with-rust_decimal`); còn dùng nó thì `chrono` không ra khỏi cây.
   Module chỉ dùng qua `autoxide_kernel::migration::prelude` (`MigrationTrait`, `MigrationName`, `SchemaManager` với
   `create_table` / `drop_table` / `create_index` / `drop_index` / `alter_table` / `has_table` / `exec_stmt` /
   `get_connection` / `get_database_backend`, và `schema::*`) → kernel viết lại đúng các tên đó (cùng chữ ký), module
   không phải sửa; bảng `<m>_migrations` giữ nguyên cột và dạng (`version`, `applied_at` giây Unix).
6. **`rmcp` → MCP server tự viết trong kernel, nếu tới lúc đó upstream vẫn bắt buộc `chrono`.** `rmcp` 3.11 (mới nhất)
   phụ thuộc `chrono` không qua feature (`Annotations.last_modified`). MCP là đặc tả công khai (JSON-RPC 2.0 + Streamable
   HTTP); kernel chỉ dùng phần server (initialize, ping, tools, resources, prompts, phiên `Mcp-Session-Id`) — thay
   bằng code của kernel theo đặc tả, client cho test cũng tự viết trên `reqwest`. Trước khi làm, kiểm lại bản mới của
   `rmcp`: bỏ được `chrono` thì chỉ nâng phiên bản.
7. **`object_store` → `rusty-s3` 0.12 + `reqwest`** (S3 / R2 / MinIO): thư viện ký S3 kiểu sans-IO (chỉ dựng và ký
   request, tự gửi bằng `reqwest` sẵn có), được bảo trì (bản 18/11/2026), dựng trên đúng `jiff` 1.0, `hmac` 0.13,
   `sha2` 0.11, `base64` 0.23 repo đã có; BSD-2-Clause (đã cho phép, D-353). Đủ các thao tác `files` dùng: PutObject,
   GetObject (kể cả `Range`), HeadObject, DeleteObject, ListObjectsV2, multipart (Create / UploadPart / Complete /
   Abort), link ký sẵn cho PUT. Ổ đĩa cục bộ tự viết trên `tokio::fs` (ghi file tạm rồi đổi tên, `fsync`). Gỡ
   `object_store` thì `humantime` ra khỏi cây.
8. **`paste` (proc-macro của `rav1e`, bộ mã hoá AVIF)**: không có bộ mã hoá AVIF Rust thuần nào khác; `rav1e` 0.8.1 vẫn là
   bản mới nhất và vẫn dùng `paste`. AVIF đã tắt mặc định lúc chạy (D-20261119-step39-avif) → chuyển thành feature
   cargo `avif` của `media` (và `autoxide`), không bật mặc định: bản build mặc định (và `cargo deny`, chạy với feature
   mặc định) không còn `rav1e` / `paste`. Bật `AUTOXIDE_MEDIA_AVIF=true` trên bản build không có `avif` thì máy chủ
   từ chối khởi động với thông báo nói rõ cách build. Làm khi phiên giữ `media` đồng ý.
9. **Chuyển dần, mỗi PR chạy trên cả hai database**: (a) lớp `time` + cầu nối hai chiều với `chrono` (PR này);
   (b) kernel + testkit dùng kiểu mới, migration của kernel thay `sea-orm-migration`; (c) từng nhóm module; (d) MCP
   server; (e) S3; (f) AVIF; (g) PR cuối gỡ `chrono`, `chrono-tz`, `humantime`, `sea-orm-migration`, `rmcp`, `with-chrono`
   của SeaORM, cầu nối, và bốn dòng `ignore` của `deny.toml`. `cargo xtask upgrade-test` từ v0.16.0 phải xanh ở (b), (c)
   và (g).

Lý do: `jiff` do tác giả `regex` / `ripgrep` viết và bảo trì, 1.0 ổn định (18/11/2026), API theo Temporal (tính giờ địa
phương đúng cả khi đổi giờ mùa hè), có sẵn múi giờ IANA — một thư viện thay cả `chrono` lẫn `chrono-tz`; `rusty-s3` đã kéo
nó vào. `time` không có múi giờ IANA (phải thêm thư viện thứ hai) và bản sqlx / sea-query của nó cũng đổi dạng lưu SQLite
(`…Z`, cắt số 0 — sắp sai thứ tự: `.12Z` > `.125Z`), nên chọn `time` không bớt được lớp tự viết nào. Lớp tự viết ở kernel
là một chỗ duy nhất, có test so với `chrono` từng ký tự khi `chrono` còn trong repo.

Đã cân nhắc:
- `time` cho cột database + `jiff` cho múi giờ: hai thư viện thời gian, vẫn phải tự mã hoá dạng SQLite (xem Lý do).
- `jiff` qua feature `with-jiff` của `sea-query`: `sea-query-sqlx` 0.9 dừng chương trình (`panic!`) khi gửi giá trị `jiff`
  cho Postgres, và ghi SQLite dạng `…Z` cắt số 0.
- Giá trị dạng chuỗi (`Value::String`): Postgres từ chối gán `text` vào cột `timestamptz`. Ép kiểu bằng `save_as` của
  SeaORM: SQLite hiểu `CAST(… AS timestamptz)` là ép sang số (`NUMERIC`) và ghi `2026` thay cho cả chuỗi; `save_as`
  cũng không áp cho `WHERE`. Cờ toàn cục "đang dùng database nào": một chương trình có thể giữ nhiều database (D-185).
- Đổi dạng lưu sang dạng của thư viện mới + migration dữ liệu: vi phạm ràng buộc "dữ liệu cũ đọc được, không migration
  dữ liệu", và bản cũ đọc dữ liệu mới sẽ sai khi phải lùi phiên bản.
- Giữ `chrono` tới khi `sqlx` / SeaORM có `jiff`: không có mốc thời gian; `sea-orm-migration` và `rmcp` còn bắt buộc
  `chrono` độc lập với điều đó.
- Fork / `[patch]` `sea-orm-migration`, `rmcp`, `rav1e`: `deny.toml` chỉ nhận nguồn crates.io, và fork phải tự theo dõi
  bản vá bảo mật.
- Tự viết AWS SigV4: `rusty-s3` là đúng phần đó, nhỏ, được bảo trì, không kéo thêm thư viện mật mã.
- Bỏ hẳn AVIF: website muốn ảnh nhẹ hơn WebP một phần ba mất lựa chọn; feature cargo giữ được nó cho ai tự build.

Thay: phần "Việc kế tiếp … theo dõi `sqlx` / SeaORM chuyển sang `jiff`" của D-346
