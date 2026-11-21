## D-20261121-raw-statement · Câu SQL viết tay dựng bằng `autoxide_kernel::database::statement` — 2026-11-21 — agent quyết

Chủ đề: Database, dữ liệu, job, event

Quyết định: câu SQL viết tay kèm tham số (trong `src/` lẫn `tests/` của mọi crate trừ xtask) dựng bằng
`autoxide_kernel::database::statement(backend, sql, values)`, không bằng `sea_orm::Statement::from_sql_and_values`.
Trên Postgres hàm này thêm tên kiểu sau mỗi tham số mang kiểu (`$n` → `$n::"timestamptz"` / `$n::"date"`), y như
builder của sea-query; SQLite giữ nguyên câu. Luật `raw-statement` của `cargo xtask check` chặn cách cũ; file duy nhất
được gọi thẳng là `crates/kernel/src/data/database.rs`.

Lý do: `Timestamp` / `Date` của kernel (D-20261121-time-library) đi xuống driver dưới dạng chuỗi kèm tên kiểu, để
SQLite lưu đúng chuỗi như trước. Builder tự chèn tên kiểu vào câu Postgres; câu viết tay thì không, Postgres nhận
tham số kiểu `text` và từ chối (lỗi 42804) — 17 bài test hỏng trên Postgres mà SQLite vẫn xanh, chỉ lộ ra ở gate đầy
đủ / CI. Một hàm chung cho mọi câu viết tay sửa tận gốc và luật xtask giữ cho lỗi không quay lại.

Đã cân nhắc: viết `CAST(? AS timestamptz)` trong câu (SQLite cho kiểu lạ affinity NUMERIC, chuỗi ngày thành số);
đổi từng câu sang builder (đúng nhưng không ngăn câu viết tay mới lặp lại lỗi); cho `Timestamp` bind bằng kiểu của
sqlx (mất dạng lưu SQLite cũ, vì `From<Timestamp> for Value` không biết đang ở database nào).
