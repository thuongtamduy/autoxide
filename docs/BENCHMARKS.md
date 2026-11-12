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

## Đo tải qua API thật

Bộ đo ở trên đo code và database, không qua mạng. Bài này đo **đúng thứ người dùng gặp**:
binary release, `autoxide serve` thật, TCP thật, đăng nhập thật (access token, API key),
bộ phát event chạy nền, database thật — bằng `oha` (D-076):

```bash
cargo install oha --locked
scripts/loadtest.sh                                                    # SQLite
LOADTEST_DATABASE_URL=postgres://user:pass@host:5432/empty_db scripts/loadtest.sh
LOADTEST_BINARY=/path/to/autoxide-before scripts/loadtest.sh            # đo một binary có sẵn
```

So trước / sau một thay đổi: build bản trước, chép binary ra ngoài, build bản sau, rồi chạy
xen kẽ hai binary (mỗi bản ít nhất hai lượt, database trống mới mỗi lượt) — máy tạo tải và máy
chủ chung một máy nên một lượt riêng lẻ lệch tới ~15%. Ví dụ: D-163.

Kịch bản tạo admin, đăng nhập, tạo API key, gieo 120 người dùng (audit log có dữ liệu)
và một file 16 KiB, rồi bắn tải 10 giây mỗi bài: đọc (`/auth/me` bằng token và bằng API
key, list 50 dòng, audit log, tải file), MCP `tools/call`, ghi (`PATCH` một người dùng —
kèm event; tải file 4 KiB lên), đăng nhập (Argon2id). Cuối cùng tăng dần 1 → 1024 kết
nối cho một bài đọc và một bài ghi, rồi kiểm máy chủ vẫn sẵn sàng và log không có lỗi.
Rate limiting được tắt (một máy tạo tải sẽ bị chặn ngay ở mức mặc định).

Máy tạo tải và máy chủ chạy **chung một máy 4 nhân** nên tranh CPU: số dưới đây là cận
dưới. PostgreSQL 16 cài thường, cùng máy.

### SQLite — 4 CPU, 10s mỗi bài

| Bài đo                             | Kết nối |  Request/s |   p50 ms |   p95 ms |   p99 ms |  Thành công |
| ---------------------------------- | ----: | --------: | -------: | -------: | -------: | ------: |
| GET /health/ready                  |    64 |     10431 |     6.17 |     7.26 |     8.67 | 100.00% |
| GET /auth/me (access token)        |    64 |      6948 |     9.09 |    10.93 |    13.29 | 100.00% |
| GET /auth/me (API key)             |    64 |      4834 |    12.90 |    16.43 |    20.17 | 100.00% |
| GET /auth/users?limit=50           |    64 |      2588 |    24.71 |    28.28 |    30.31 | 100.00% |
| GET /audit?limit=50                |    64 |      1866 |    34.14 |    38.42 |    41.30 | 100.00% |
| GET /files/{id}/content (16 KiB)   |    64 |      5455 |    11.63 |    13.93 |    15.57 | 100.00% |
| MCP tools/call auth.whoami         |    64 |      5528 |    11.46 |    13.60 |    15.57 | 100.00% |
| PATCH /auth/users/{id} (ghi + event) |    64 |      1237 |    49.99 |    60.75 |    67.35 | 100.00% |
| POST /files (4 KiB, ghi)           |    64 |      1680 |    37.07 |    44.78 |    47.79 | 100.00% |
| POST /auth/login (Argon2id)        |    16 |       156 |   102.53 |   127.86 |   139.55 | 100.00% |

#### Tăng dần số kết nối: đọc (GET /auth/me) và ghi (PATCH /auth/users/{id})

| Bài đo                             | Kết nối |  Request/s |   p50 ms |   p95 ms |   p99 ms |  Thành công |
| ---------------------------------- | ----: | --------: | -------: | -------: | -------: | ------: |
| đọc                                |     1 |      1927 |     0.50 |     0.64 |     0.84 | 100.00% |
| đọc                                |    16 |      8415 |     1.84 |     2.69 |     3.34 | 100.00% |
| đọc                                |    64 |      8251 |     7.60 |     9.10 |    11.87 | 100.00% |
| đọc                                |   256 |      7900 |    32.03 |    35.49 |    38.65 | 100.00% |
| đọc                                |  1024 |      7178 |   143.06 |   155.60 |   161.06 | 100.00% |
| ghi                                |     1 |       783 |     1.12 |     1.41 |     8.72 | 100.00% |
| ghi                                |    16 |      1246 |    12.02 |    20.17 |    21.56 | 100.00% |
| ghi                                |    64 |      1252 |    49.24 |    59.38 |    73.14 | 100.00% |
| ghi                                |   256 |      1251 |   207.88 |   223.32 |   238.13 | 100.00% |
| ghi                                |  1024 |      1292 |   841.52 |   905.01 |  1790.14 | 100.00% |

Máy chủ vẫn sẵn sàng sau tải.
Dòng ERROR trong log máy chủ: 0

### PostgreSQL — 4 CPU, 10s mỗi bài

| Bài đo                             | Kết nối |  Request/s |   p50 ms |   p95 ms |   p99 ms |  Thành công |
| ---------------------------------- | ----: | --------: | -------: | -------: | -------: | ------: |
| GET /health/ready                  |    64 |     14262 |     4.24 |     6.86 |     8.62 | 100.00% |
| GET /auth/me (access token)        |    64 |      5929 |    10.65 |    13.46 |    16.40 | 100.00% |
| GET /auth/me (API key)             |    64 |      4385 |    14.10 |    18.18 |    20.83 | 100.00% |
| GET /auth/users?limit=50           |    64 |      4150 |    15.23 |    18.49 |    20.79 | 100.00% |
| GET /audit?limit=50                |    64 |      4636 |    13.56 |    16.97 |    19.13 | 100.00% |
| GET /files/{id}/content (16 KiB)   |    64 |      5112 |    12.38 |    15.24 |    16.99 | 100.00% |
| MCP tools/call auth.whoami         |    64 |      5847 |    10.79 |    13.27 |    15.05 | 100.00% |
| PATCH /auth/users/{id} (ghi + event) |    64 |      1496 |    41.25 |    55.30 |    64.98 | 100.00% |
| POST /files (4 KiB, ghi)           |    64 |      2629 |    23.59 |    31.54 |    42.42 | 100.00% |
| POST /auth/login (Argon2id)        |    16 |       161 |    98.20 |   123.51 |   140.82 | 100.00% |

#### Tăng dần số kết nối: đọc (GET /auth/me) và ghi (PATCH /auth/users/{id})

| Bài đo                             | Kết nối |  Request/s |   p50 ms |   p95 ms |   p99 ms |  Thành công |
| ---------------------------------- | ----: | --------: | -------: | -------: | -------: | ------: |
| đọc                                |     1 |      1318 |     0.71 |     1.01 |     2.54 | 100.00% |
| đọc                                |    16 |      4579 |     3.30 |     5.71 |     7.73 | 100.00% |
| đọc                                |    64 |      4800 |    13.20 |    16.71 |    19.37 | 100.00% |
| đọc                                |   256 |      4815 |    51.54 |    63.00 |    70.76 | 100.00% |
| đọc                                |  1024 |      4399 |   232.45 |   265.46 |   275.92 | 100.00% |
| ghi                                |     1 |       527 |     1.84 |     2.21 |     2.87 | 100.00% |
| ghi                                |    16 |      1225 |    10.92 |    26.79 |    37.85 | 100.00% |
| ghi                                |    64 |      1261 |    48.71 |    65.60 |    82.91 | 100.00% |
| ghi                                |   256 |      1288 |   197.62 |   222.52 |   247.63 | 100.00% |
| ghi                                |  1024 |      1327 |   814.88 |   883.64 |  1744.32 | 100.00% |

Máy chủ vẫn sẵn sàng sau tải.
Dòng ERROR trong log máy chủ: 0

### Đọc số

- **Không lỗi nào**: 100% request thành công ở mọi bài, kể cả 1024 kết nối đồng thời;
  máy chủ vẫn sẵn sàng sau tải, log không có dòng ERROR.
- **Đọc** bão hoà quanh 7–8 nghìn request/s (SQLite) và ~5 nghìn (PostgreSQL) với một
  máy chủ trên 4 nhân dùng chung với máy tạo tải. Quá ngưỡng thì thời gian chờ tăng đều
  theo số kết nối (xếp hàng), không có request hỏng.
- **Ghi cùng một dòng** (`PATCH` một người dùng) đứng ở ~1.250–1.500 request/s: mỗi lệnh
  ghi là một transaction (thay đổi + event) và các lệnh cùng dòng phải nối đuôi nhau —
  SQLite vì chỉ có một kết nối ghi, PostgreSQL vì khoá dòng. Ghi vào các dòng khác nhau
  (tải file lên) trên PostgreSQL đạt ~2.600/s.
- **Đăng nhập** ~150–160/s là chủ ý: Argon2id tốn bộ nhớ và CPU để chống dò mật khẩu;
  số lần hash cùng lúc giới hạn bằng số nhân CPU. Agent nên dùng API key thay vì đăng
  nhập lại.

### Chỗ đã sửa nhờ bài đo này (11/11/2026)

| Bài đo (64 kết nối) | SQLite trước → sau | PostgreSQL trước → sau | Nguyên nhân |
| --- | --- | --- | --- |
| Tải file 16 KiB | 1.419 → 5.455/s, p50 44 → 11,6 ms | 1.430 → 5.112/s | Thiếu `TCP_NODELAY`: header và thân gửi thành hai mảnh vướng Nagle + delayed ACK, mỗi request chờ ~40 ms dù máy chủ rảnh |
| MCP `tools/call` | 3.687 → 5.528/s | 4.600 → 5.847/s | Như trên |
| `GET /auth/me` | 6.742 → 6.948/s | 3.987 → 5.929/s | Mỗi request đã đăng nhập tốn hai câu query để lấy quyền; gộp thành một câu join |
| `/auth/me` bằng API key | 4.209 → 4.834/s | 3.290 → 4.385/s | Như trên |

Cùng lúc: thư viện MCP ghi mức ERROR mỗi khi client ngắt kết nối giữa chừng (một việc
bình thường) — đã lọc đúng thông điệp đó để không làm nhiễu cảnh báo vận hành.

## Thử tải dài hạn (soak) — 12/11/2026

`LOADTEST_SOAK=15m scripts/loadtest.sh` chạy song song suốt 15 phút: đọc `/auth/me`, list
50 người dùng, MCP `tools/call`, tải file 16 KiB, ghi `PATCH` (200/s) và tải file lên
(20/s); đo bộ nhớ (RSS), số file đang mở và số kết nối PostgreSQL của máy chủ mỗi 30
giây. Cùng máy 4 nhân với máy tạo tải. Bản PostgreSQL chồng với một lượt gate chạy ở vài
phút cuối, nên số request/s của nó thấp hơn thật — kết luận về rò rỉ không đổi.

| Database   | Request    | Thành công | RSS (MiB) đầu → cuối | File mở | Kết nối DB | Dòng ERROR |
| ---------- | ---------: | ---------: | -------------------: | ------: | ---------: | ---------: |
| SQLite     | 3.834.543  | 100,000%   | 64,5 → 69,6          | 85–88   | —          | 0          |
| PostgreSQL | 2.374.901  | 100,000%   | 63,3 → 67,8          | 82–84   | 21 (đứng yên) | 0       |

Kết luận: **không thấy rò rỉ** bộ nhớ, file hay kết nối database. Bộ nhớ chỉ tăng vài MiB
trong phút đầu (bộ nhớ đệm, pool ấm lên) rồi gần như phẳng (nửa sau so với nửa đầu: +4%
SQLite, +0,4% PostgreSQL); số kết nối PostgreSQL giữ nguyên 21 suốt bài. Máy chủ vẫn sẵn
sàng sau tải.

## Tìm theo ý nghĩa: duyệt hết vector — 12/11/2026

Bước 18 (D-109) tìm theo ý nghĩa bằng cách so câu tìm với **mọi** vector của một ngôn ngữ
trong bộ nhớ (tích vô hướng trên vector đã chuẩn hoá, giữ 50 kết quả tốt nhất), không dùng
index ANN hay `pgvector` — để chạy như nhau trên SQLite và Postgres. Đo phần tính toán đó
bằng đúng phép tính của `autoxide_kernel::ai::vector` (`dot`, `normalize`), build tối ưu
(`-C opt-level=3`, không `target-cpu=native`), **một nhân** Intel Xeon 2,1 GHz của máy 4
nhân dùng chung; mỗi cỡ 7 lần, lấy trung vị. Không gồm lần gọi embedding cho câu tìm (một
request tới nhà cung cấp, có bộ nhớ đệm) và lần đọc một dòng `version`.

| Số đoạn × chiều | Trung vị | Nhanh nhất – chậm nhất | Bộ nhớ vector |
| --------------- | -------: | ---------------------: | ------------: |
| 10.000 × 1.024  | 8,3 ms   | 8,1 – 11,4 ms          | ~41 MB        |
| 100.000 × 1.024 | 107 ms   | 94 – 115 ms            | ~410 MB       |

Thời gian tăng tuyến tính theo số đoạn; một bài ~1.500 chữ là 5–6 đoạn, nên 10.000 đoạn
≈ 2.000 bài một ngôn ngữ. **Ngưỡng chấp nhận được** (D-110): dưới 50 ms cho phần tính toán,
tức khoảng 40.000 đoạn mỗi ngôn ngữ trên máy cỡ này. Vượt ngưỡng đó mới là lý do cho một bước
sau (vector `f16` / `int8` để giảm bộ nhớ và băng thông, chia việc ra nhiều nhân, hay index
ANN / nhánh `pgvector` sau một trait) — không làm trước khi đo được cần.

Nạp lại vector vào bộ nhớ (sau khi index đổi) đọc mọi dòng của ngôn ngữ đó một lần; với trang
sửa bài liên tục mà có nhiều đoạn, đó là chi phí đáng theo dõi trước tiên.
