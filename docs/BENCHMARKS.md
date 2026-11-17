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

## Mốc trước bước 33 (đa khách) — 15/11/2026

Đo trên `main` ngay trước phần A của bước 33 (commit `b5435c7`), để so sau khi thêm ngữ cảnh khách vào mọi
truy vấn (đặc tả 33, mục C4). Cùng máy 4 CPU, máy tạo tải chạy chung, 6 giây mỗi bài
(`LOADTEST_DURATION=6s scripts/loadtest.sh`). Số 429 của bài đăng nhập là login throttling làm đúng việc.

### SQLite

| Bài đo                             | Kết nối |  Request/s |   p50 ms |   p95 ms |   p99 ms |  Thành công |
| ---------------------------------- | ----: | --------: | -------: | -------: | -------: | ------: |
| GET /health/ready                  |    64 |     15978 |     4.13 |     5.10 |     6.13 | 100.00% |
| GET /auth/me (access token)        |    64 |      5315 |    11.86 |    14.68 |    16.58 | 100.00% |
| GET /auth/me (API key)             |    64 |      3789 |    16.59 |    20.12 |    22.41 | 100.00% |
| GET /auth/users?limit=50           |    64 |      2032 |    31.22 |    36.26 |    40.09 | 100.00% |
| GET /audit?limit=50                |    64 |      1363 |    46.86 |    53.78 |    57.31 | 100.00% |
| GET /files/{id}/content (16 KiB)   |    64 |      4597 |    13.74 |    17.18 |    19.23 | 100.00% |
| MCP tools/call auth.whoami         |    64 |      4788 |    13.23 |    16.27 |    17.96 | 100.00% |
| PATCH /auth/users/{id} (ghi + event) |    64 |      1156 |    54.97 |    67.80 |    77.15 | 100.00% |
| POST /files (4 KiB, ghi)           |    64 |      1398 |    45.42 |    56.37 |    61.79 | 100.00% |
| POST /auth/login (Argon2id)        |    16 |      3668 |     3.98 |     5.63 |     7.85 |   0.03% |  non-2xx {'429': 21997}

### Tăng dần số kết nối: đọc (GET /auth/me) và ghi (PATCH /auth/users/{id})

| Bài đo                             | Kết nối |  Request/s |   p50 ms |   p95 ms |   p99 ms |  Thành công |
| ---------------------------------- | ----: | --------: | -------: | -------: | -------: | ------: |
| đọc                                |     1 |      1327 |     0.72 |     0.95 |     1.15 | 100.00% |
| đọc                                |    16 |      4787 |     3.26 |     4.77 |     5.71 | 100.00% |
| đọc                                |    64 |      4812 |    13.03 |    16.57 |    19.22 | 100.00% |
| đọc                                |   256 |      4405 |    56.33 |    67.32 |    71.38 | 100.00% |
| đọc                                |  1024 |      4363 |   236.20 |   264.74 |  1227.21 | 100.00% |
| ghi                                |     1 |       503 |     1.80 |     2.40 |    11.34 | 100.00% |
| ghi                                |    16 |      1111 |    13.67 |    23.17 |    28.12 | 100.00% |
| ghi                                |    64 |      1139 |    55.95 |    66.76 |    69.86 | 100.00% |
| ghi                                |   256 |      1174 |   223.78 |   246.89 |   253.78 | 100.00% |
| ghi                                |  1024 |      1189 |   965.37 |  1781.15 |  2020.25 | 100.00% |

Máy chủ vẫn sẵn sàng sau tải.
Dòng ERROR trong log máy chủ: 0

### PostgreSQL 16

| Bài đo                             | Kết nối |  Request/s |   p50 ms |   p95 ms |   p99 ms |  Thành công |
| ---------------------------------- | ----: | --------: | -------: | -------: | -------: | ------: |
| GET /health/ready                  |    64 |     15352 |     4.31 |     5.45 |     6.44 | 100.00% |
| GET /auth/me (access token)        |    64 |      4437 |    14.07 |    18.70 |    20.78 | 100.00% |
| GET /auth/me (API key)             |    64 |      3467 |    17.91 |    24.79 |    29.21 | 100.00% |
| GET /auth/users?limit=50           |    64 |      3390 |    18.50 |    23.83 |    26.75 | 100.00% |
| GET /audit?limit=50                |    64 |      3012 |    20.47 |    27.97 |    35.25 | 100.00% |
| GET /files/{id}/content (16 KiB)   |    64 |      3735 |    16.95 |    21.34 |    23.62 | 100.00% |
| MCP tools/call auth.whoami         |    64 |      4538 |    13.53 |    19.21 |    22.58 | 100.00% |
| PATCH /auth/users/{id} (ghi + event) |    64 |      1459 |    42.88 |    56.08 |    65.99 | 100.00% |
| POST /files (4 KiB, ghi)           |    64 |      2205 |    28.59 |    38.42 |    43.97 | 100.00% |
| POST /auth/login (Argon2id)        |    16 |      7252 |     2.00 |     4.04 |     5.66 |   0.00% |  non-2xx {'429': 43518}

### Tăng dần số kết nối: đọc (GET /auth/me) và ghi (PATCH /auth/users/{id})

| Bài đo                             | Kết nối |  Request/s |   p50 ms |   p95 ms |   p99 ms |  Thành công |
| ---------------------------------- | ----: | --------: | -------: | -------: | -------: | ------: |
| đọc                                |     1 |      1009 |     0.92 |     1.39 |     3.11 | 100.00% |
| đọc                                |    16 |      4391 |     3.54 |     5.46 |     6.68 | 100.00% |
| đọc                                |    64 |      4349 |    14.47 |    19.84 |    22.97 | 100.00% |
| đọc                                |   256 |      3800 |    66.96 |    81.34 |    87.89 | 100.00% |
| đọc                                |  1024 |      4503 |   220.83 |   247.27 |  1274.31 | 100.00% |
| ghi                                |     1 |       552 |     1.74 |     2.34 |     2.98 | 100.00% |
| ghi                                |    16 |      1376 |    10.00 |    23.12 |    32.72 | 100.00% |
| ghi                                |    64 |      1382 |    45.20 |    60.27 |    69.87 | 100.00% |
| ghi                                |   256 |      1387 |   188.79 |   215.74 |   227.00 | 100.00% |
| ghi                                |  1024 |      1530 |   698.78 |   756.59 |  1755.63 | 100.00% |

Máy chủ vẫn sẵn sàng sau tải.
Dòng ERROR trong log máy chủ: 0

## Sau bước 33 (đa khách) — 16/11/2026

Đo lại để so với mốc ở trên (đặc tả 33, mục C4). Máy khác mốc cũ nên **đo lại cả bản trước** trên cùng máy: bản
`b5435c7` (trước phần A) và bản sau phần C kèm index `(tenant_id, id)` (D-302), cùng máy 4 CPU, máy tạo tải chạy chung,
6 giây mỗi bài, **mỗi bài 3 lần, lấy trung vị** (D-179: dao động ±20%). `scripts/loadtest.sh`, chế độ `single`.


### SQLite

| Bài đo | Kết nối | Trước (req/s) | Sau (req/s) | Đổi | p50 trước → sau (ms) |
| --- | ---: | ---: | ---: | ---: | ---: |
| GET /health/ready | 64 | 21249 | 17508 | -18% | 3.41 → 4.10 |
| GET /auth/me (access token) | 64 | 5724 | 3351 | -41% | 11.02 → 18.81 |
| GET /auth/me (API key) | 64 | 4016 | 2597 | -35% | 15.68 → 24.14 |
| GET /auth/users?limit=50 | 64 | 2387 | 1300 | -46% | 25.35 → 48.76 |
| GET /audit?limit=50 | 64 | 1507 | 1335 | -11% | 42.49 → 47.91 |
| GET /files/{id}/content (16 KiB) | 64 | 4886 | 4183 | -14% | 12.94 → 15.20 |
| MCP tools/call auth.whoami | 64 | 5015 | 3240 | -35% | 12.68 → 19.64 |
| PATCH /auth/users/{id} (ghi + event) | 64 | 1336 | 1300 | -3% | 46.22 → 48.24 |
| POST /files (4 KiB, ghi) | 64 | 1472 | 1471 | -0% | 43.78 → 42.97 |
| POST /auth/login (Argon2id) | 16 | 4387 | 4053 | -8% | 3.43 → 3.71 |
| đọc | 1 | 1379 | 856 | -38% | 0.69 → 1.13 |
| đọc | 64 | 4949 | 2891 | -42% | 12.80 → 21.91 |
| đọc | 256 | 4403 | 2681 | -39% | 57.20 → 95.24 |
| ghi | 1 | 569 | 508 | -11% | 1.58 → 1.74 |
| ghi | 64 | 1288 | 1269 | -1% | 47.41 → 49.24 |
| ghi | 256 | 1332 | 1302 | -2% | 195.10 → 198.74 |

### PostgreSQL 16

| Bài đo | Kết nối | Trước (req/s) | Sau (req/s) | Đổi | p50 trước → sau (ms) |
| --- | ---: | ---: | ---: | ---: | ---: |
| GET /health/ready | 64 | 19956 | 17641 | -12% | 3.58 → 4.06 |
| GET /auth/me (access token) | 64 | 4937 | 2980 | -40% | 12.50 → 21.18 |
| GET /auth/me (API key) | 64 | 3637 | 2404 | -34% | 17.11 → 25.94 |
| GET /auth/users?limit=50 | 64 | 3833 | 2143 | -44% | 16.46 → 29.45 |
| GET /audit?limit=50 | 64 | 3068 | 2857 | -7% | 20.42 → 22.05 |
| GET /files/{id}/content (16 KiB) | 64 | 3816 | 3526 | -8% | 16.48 → 17.86 |
| MCP tools/call auth.whoami | 64 | 4694 | 2637 | -44% | 13.18 → 24.17 |
| PATCH /auth/users/{id} (ghi + event) | 64 | 1155 | 1024 | -11% | 53.09 → 60.64 |
| POST /files (4 KiB, ghi) | 64 | 2016 | 1869 | -7% | 31.03 → 33.40 |
| POST /auth/login (Argon2id) | 16 | 8500 | 232 | -97% | 1.70 → 100.19 |
| đọc | 1 | 1218 | 710 | -42% | 0.78 → 1.38 |
| đọc | 64 | 5140 | 2549 | -50% | 11.96 → 25.07 |
| đọc | 256 | 4219 | 2433 | -42% | 57.97 → 103.15 |
| ghi | 1 | 498 | 450 | -10% | 1.82 → 2.15 |
| ghi | 64 | 1099 | 1060 | -4% | 55.88 → 58.34 |
| ghi | 256 | 1124 | 1058 | -6% | 231.86 → 243.80 |

Đọc kết quả:

- Phần chung mọi request (giải khách ở chế độ `single`, lọc `tenant_id`) nằm trong dao động: kiểm sức khoẻ −12 / −18%,
  tải file −8 / −14%, ghi (`PATCH`, `POST /files`, lượt ghi tăng dần) −0 đến −11%, danh sách audit −7 / −11%.
- Trước khi thêm index `(tenant_id, id)`, danh sách audit trên SQLite chậm −47%: danh sách của một khách sắp theo `id`
  không có index nào khớp nên SQLite đọc hết dòng của khách rồi sắp xếp tạm (chậm dần theo cỡ bảng). D-302 thêm
  index cho 20 bảng có danh sách như vậy.
- `GET /auth/me`, `auth.whoami` và `GET /auth/users` chậm 35–46%: các endpoint này giờ trả thêm `has_password`,
  `mfa_enabled`, `pending_email`, `last_sign_in_at` (đề bài giao diện, không phải đa khách) — thêm 2–3 câu SQL mỗi
  request (đếm bằng `log_statement` của Postgres: 3 → 6 câu cho một `GET /auth/me`). Ghi thành việc ở bước 39.
- `POST /auth/login` ở bài đo này gần như toàn 429 (login throttling làm đúng việc). Trên Postgres lượt bị chặn chậm đi
  nhiều (8500 → 232 request/s): bộ đếm lượt thử giờ cập nhật cùng một dòng bằng nhiều câu `UPDATE` nối tiếp, 16 kết
  nối dồn vào một email thì xếp hàng trên khoá dòng. Chỉ ảnh hưởng lúc có người dò mật khẩu dồn dập một tài khoản;
  ghi thành việc ở bước 39.

### Chế độ nhiều khách: 100 khách chung + 10 khách SQLite riêng

`LOADTEST_TENANCY=multi scripts/loadtest.sh` (bản sau phần C, trước index `(tenant_id, id)`, một lần). Người dùng của
một khách, chọn khách bằng credential; database chính có 111 khách.

**SQLite (database chính)**

| Bài đo                             | Kết nối |  Request/s |   p50 ms |   p95 ms |   p99 ms |  Thành công |
| ---------------------------------- | ----: | --------: | -------: | -------: | -------: | ------: |
| khách chung: GET /auth/me          |    64 |      2609 |    24.29 |    28.80 |    30.96 | 100.00% |
| khách chung: GET /auth/users       |    64 |      1741 |    36.51 |    41.43 |    45.83 | 100.00% |
| khách chung: PATCH /auth/users     |    64 |      1212 |    51.12 |    64.78 |    77.16 | 100.00% |
| khách riêng: GET /auth/me          |    64 |      2304 |    27.75 |    32.71 |    37.12 | 100.00% |
| khách riêng: GET /auth/users       |    64 |      1537 |    40.95 |    47.30 |    52.64 | 100.00% |
| khách riêng: PATCH /auth/users     |    64 |      1234 |    50.58 |    61.16 |    71.04 | 100.00% |

**PostgreSQL 16 (database chính)**

| Bài đo                             | Kết nối |  Request/s |   p50 ms |   p95 ms |   p99 ms |  Thành công |
| ---------------------------------- | ----: | --------: | -------: | -------: | -------: | ------: |
| khách chung: GET /auth/me          |    64 |      2063 |    30.74 |    37.87 |    41.72 | 100.00% |
| khách chung: GET /auth/users       |    64 |      1923 |    32.92 |    40.58 |    46.25 | 100.00% |
| khách chung: PATCH /auth/users     |    64 |       964 |    63.76 |    88.21 |   112.45 | 100.00% |
| khách riêng: GET /auth/me          |    64 |      2015 |    31.28 |    38.27 |    42.18 | 100.00% |
| khách riêng: GET /auth/users       |    64 |      1477 |    43.26 |    49.79 |    53.75 | 100.00% |
| khách riêng: PATCH /auth/users     |    64 |      1297 |    47.81 |    57.61 |    60.68 | 100.00% |

Khách riêng (database SQLite của riêng nó) chạy ngang khách chung; máy chủ không lỗi dòng nào trong cả hai lượt.

## Sau bước 34 (tổ chức, danh tính) — 16/11/2026

So bản `15a6de4` (đóng bước 33) với bản cuối bước 34, cùng máy 4 CPU, máy tạo tải chạy chung, SQLite chế độ
`single`, 6 giây mỗi bài, mỗi bản 3 lần xen kẽ, lấy trung vị (D-179: dao động ±20%). `scripts/loadtest.sh` với
`LOADTEST_BINARY`, `LOADTEST_RAMP=64`.

| Bài đo | Kết nối | Trước bước 34 (req/s) | Sau bước 34 (req/s) | Đổi | p50 trước → sau (ms) |
| --- | ---: | ---: | ---: | ---: | ---: |
| GET /health/ready | 64 | 22486 | 22766 | +1% | 3.13 → 3.08 |
| GET /auth/me (access token) | 64 | 4269 | 3200 | -25% | 14.61 → 19.34 |
| GET /auth/me (API key) | 64 | 3286 | 2664 | -19% | 19.11 → 23.59 |
| GET /auth/users?limit=50 | 64 | 1625 | 1394 | -14% | 39.07 → 45.36 |
| GET /audit?limit=50 | 64 | 1599 | 1556 | -3% | 39.55 → 41.10 |
| GET /files/{id}/content (16 KiB) | 64 | 5202 | 4997 | -4% | 11.98 → 12.58 |
| MCP tools/call auth.whoami | 64 | 4019 | 3080 | -23% | 15.61 → 20.44 |
| PATCH /auth/users/{id} (ghi + event) | 64 | 1542 | 1377 | -11% | 39.94 → 45.30 |
| POST /files (4 KiB, ghi) | 64 | 1653 | 1669 | +1% | 38.52 → 38.70 |
| POST /auth/login (Argon2id) | 16 | 4984 | 4955 | -1% | 3.02 → 3.06 |
| đọc | 64 | 3581 | 2770 | -23% | 17.50 → 22.77 |
| ghi | 64 | 1548 | 1379 | -11% | 40.38 → 44.90 |

`GET /auth/me` (và `auth.whoami`, bài "đọc") thêm một câu: role bắt buộc MFA chưa bật (D-308) — lần đo đầu là -35%
vì `GET /me` còn đọc lại dòng người dùng và kiểm MFA bằng hai câu; gộp lại còn -25% (D-317). Phần còn lại thuộc mục
"Rút số câu SQL của `GET /auth/me`" của bước 39. Đường ghi, file, audit trong khoảng dao động.

## Rút số câu SQL của đường đã đăng nhập — 16/11/2026

So bản `e5576e0` (cuối bước 34) với bản sửa (D-319), cùng máy 4 CPU, máy tạo tải chạy chung, SQLite chế độ `single`,
6 giây mỗi bài, mỗi bản 3 lần xen kẽ, lấy trung vị. `scripts/loadtest.sh` với `LOADTEST_BINARY`, `LOADTEST_RAMP=64`.
Hôm đo máy ồn hơn các lần trước (bản cũ `GET /auth/me` 2048–2972 req/s, lần trước 3200): so tương đối giữa hai bản
cùng lúc, không so với bảng cũ.

| Bài đo | Kết nối | Trước (req/s) | Sau (req/s) | Đổi | p50 trước → sau (ms) |
| --- | ---: | ---: | ---: | ---: | ---: |
| GET /health/ready | 64 | 16914 | 18986 | +12% | 4.04 → 3.83 |
| GET /auth/me (access token) | 64 | 2110 | 5730 | +172% | 29.74 → 11.02 |
| GET /auth/me (API key) | 64 | 2049 | 3783 | +85% | 30.77 → 16.74 |
| GET /auth/users?limit=50 | 64 | 1123 | 1271 | +13% | 56.60 → 50.10 |
| GET /audit?limit=50 | 64 | 1346 | 1484 | +10% | 47.55 → 43.09 |
| GET /files/{id}/content (16 KiB) | 64 | 4055 | 5087 | +25% | 15.57 → 12.47 |
| MCP tools/call auth.whoami | 64 | 2563 | 4715 | +84% | 24.59 → 13.50 |
| PATCH /auth/users/{id} (ghi + event) | 64 | 1219 | 1240 | +2% | 51.46 → 50.19 |
| POST /files (4 KiB, ghi) | 64 | 1309 | 1433 | +9% | 49.07 → 44.23 |
| POST /auth/login (Argon2id) | 16 | 3844 | 3922 | +2% | 3.82 → 3.73 |
| đọc | 64 | 2249 | 4672 | +108% | 28.20 → 13.59 |
| ghi | 64 | 1129 | 1138 | +1% | 55.37 → 55.79 |

`GET /auth/me` bằng access token: 7 → 3 câu SQL (xác thực 2 → 1, thân 5 → 2), về lại mức trước bước 33 (5315–5724
req/s ở hai lần đo mốc). Mọi request đã đăng nhập bớt một câu (file, audit +10–25%); đường ghi và đăng nhập không đổi.

## Sau bước 35 (gói, chặn theo gói, hạn mức) — 16/11/2026

So bản `4344086` (trước bước 35) với bản `ed1a700` (bước 35 phần A–F), cùng máy 4 CPU, máy tạo tải chạy chung, SQLite,
chế độ `multi` (`LOADTEST_TENANCY=multi LOADTEST_TENANTS=20 LOADTEST_DEDICATED=2 LOADTEST_PLAN=1`), 6 giây mỗi bài, mỗi
bản 3 lần xen kẽ, lấy trung vị. Bản mới: khách s1 có thuê bao gói bật mọi module (không giới hạn), s2 không có thuê
bao (không có gói mặc định); bản cũ không có module `plans` nên hai khách giống nhau — so cùng đường đi.

| Bài đo | Kết nối | Trước (req/s) | Sau (req/s) | Đổi | p50 trước → sau (ms) |
| --- | ---: | ---: | ---: | ---: | ---: |
| GET /health/ready | 64 | 13377 | 15241 | +14% | 5.08 → 4.69 |
| GET /auth/me (access token) | 64 | 4653 | 4588 | -1% | 13.33 → 13.58 |
| GET /auth/me (API key) | 64 | 3139 | 3298 | +5% | 19.88 → 19.21 |
| GET /auth/users?limit=50 | 64 | 1027 | 1040 | +1% | 61.91 → 61.43 |
| GET /audit?limit=50 | 64 | 1171 | 1188 | +1% | 54.79 → 53.82 |
| GET /files/{id}/content (16 KiB) | 64 | 4123 | 4171 | +1% | 15.32 → 15.09 |
| MCP tools/call auth.whoami | 64 | 4040 | 3930 | -3% | 15.51 → 15.99 |
| PATCH /auth/users/{id} (ghi + event) | 64 | 1000 | 1017 | +2% | 61.82 → 62.14 |
| POST /files (4 KiB, ghi) | 64 | 1218 | 1159 | -5% | 52.34 → 54.54 |
| POST /auth/login (Argon2id) | 16 | 143 | 144 | +1% | 122.94 → 120.00 |
| đọc | 64 | 3936 | 3762 | -4% | 16.01 → 16.63 |
| ghi | 64 | 1005 | 993 | -1% | 62.99 → 63.94 |
| khách chung, có gói: GET /auth/me | 64 | 3908 | 3811 | -2% | 16.08 → 16.62 |
| khách chung, có gói: GET /auth/users | 64 | 1584 | 1574 | -1% | 39.91 → 40.12 |
| khách chung, có gói: PATCH /auth/users | 64 | 985 | 989 | +0% | 64.40 → 64.40 |
| khách chung, có gói: GET /forms | 64 | 4448 | 4388 | -1% | 13.97 → 14.23 |
| khách chung, không gói: GET /auth/me | 64 | 4047 | 3874 | -4% | 15.53 → 16.21 |
| khách chung, không gói: GET /auth/users | 64 | 1661 | 1591 | -4% | 38.24 → 39.52 |
| khách chung, không gói: PATCH /auth/users | 64 | 1018 | 978 | -4% | 61.55 → 64.00 |
| khách chung, không gói: GET /forms | 64 | 4599 | 4361 | -5% | 13.55 → 14.34 |
| khách riêng: GET /auth/me | 64 | 3426 | 3309 | -3% | 18.14 → 18.92 |
| khách riêng: GET /auth/users | 64 | 1525 | 1455 | -5% | 41.80 → 44.03 |
| khách riêng: PATCH /auth/users | 64 | 1019 | 1028 | +1% | 61.53 → 60.44 |
| khách riêng: GET /forms | 64 | 3697 | 3683 | -0% | 16.77 → 17.05 |

Mọi bài trong ±5% (dao động ±20%, D-179). Lớp chặn theo gói ở mỗi request của module (đếm request, đọc quyền dùng từ
bộ nhớ đệm) không đo được trên đường nóng; khách có gói và khách không gói chạy ngang nhau. Bảng đầu là người dùng
của khách nền tảng (luôn không giới hạn: tay cầm không hỏi `plans`).
