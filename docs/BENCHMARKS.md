# Đo hiệu năng

Bộ đo của core nằm ở `modules/notes/benches/core.rs` (D-071). Nó đi qua toàn bộ đường
của một request thật — các lớp HTTP của kernel (request id, log, CORS, idempotency, ngôn
ngữ, giới hạn thân request…) → handler → service → database — bằng module mẫu `notes`.

```bash
cargo bench -p autoxide-notes                                          # SQLite
TEST_DATABASE_URL=postgres://user:pass@host:5432/postgres cargo bench -p autoxide-notes
```

Bốn việc, mỗi việc 2000 lần (sau 200 lần chạy nóng), một database trống mỗi lượt:

| Việc | Đo gì |
| ---- | ----- |
| write | `POST /api/v1/notes`: kiểm đầu vào, INSERT + event outbox trong một transaction |
| read | `GET /api/v1/notes/{id}`: một SELECT theo khoá chính |
| list | `GET /api/v1/notes?limit=50`: một trang 50 dòng qua `fetch_page` (index chủ + id) |
| concurrent write | như write, từ 16 task cùng lúc |

## Số gốc

Đo ngày 11/11/2026, build release (`cargo bench`), máy 4 nhân dùng chung, database
cùng máy (SQLite: file trên đĩa, WAL; PostgreSQL 16 cài thường, chưa tinh chỉnh). Số
tuyệt đối phụ thuộc máy — dùng để **so tương đối** trước và sau một thay đổi, trên
cùng một máy.

SQLite:

| Việc | Việc / giây | p50 | p95 | p99 |
| ---- | ----------: | --: | --: | --: |
| write | 2020 | 0.36 ms | 0.74 ms | 3.72 ms |
| read | 5243 | 0.17 ms | 0.21 ms | 0.42 ms |
| list (50) | 1945 | 0.47 ms | 0.66 ms | 1.33 ms |
| concurrent write (16) | 1798 | 7.24 ms | 18.63 ms | 21.08 ms |

PostgreSQL:

| Việc | Việc / giây | p50 | p95 | p99 |
| ---- | ----------: | --: | --: | --: |
| write | 1020 | 0.90 ms | 1.46 ms | 2.38 ms |
| read | 3349 | 0.23 ms | 0.34 ms | 0.51 ms |
| list (50) | 2304 | 0.39 ms | 0.62 ms | 1.00 ms |
| concurrent write (16) | 3708 | 3.53 ms | 5.97 ms | 8.27 ms |

Đọc số: SQLite nhanh hơn ở việc đơn lẻ (không qua mạng), nhưng chỉ có một kết nối ghi
(single-writer) nên ghi đồng thời xếp hàng — thời gian chờ tăng theo số task, tổng số
việc / giây không tăng. PostgreSQL ghi đồng thời tốt hơn gấp đôi. Đây là lý do
production nhiều người ghi nên dùng PostgreSQL (`STACK.md`, mục Database).

## Khi nào chạy lại

Trước và sau mọi thay đổi chạm đường nóng (lớp HTTP của kernel, `fetch_page`, outbox,
cách mở kết nối hay transaction). Chậm đi rõ rệt (hơn ~20% ở p50 hay số việc / giây,
chạy vài lần cho chắc) thì tìm nguyên nhân trước khi commit; nhanh lên nhờ một thay đổi
có chủ ý thì cập nhật bảng trên kèm ngày.
