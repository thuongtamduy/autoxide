## D-20261120-step39-hot-paths · Đo đường nóng của sản phẩm, đếm câu SQL mỗi request, gộp N+1 ở đọc công khai của cửa hàng — 2026-11-20 — agent quyết

Chủ đề: Vận hành và hiệu năng

Quyết định:

1. **Công cụ đếm câu SQL**: `Database::observe_statements` (kernel) gắn một hàm vào hook metric của SeaORM trên hai
   pool; testkit gắn sẵn cho mọi `TestApp`, bài test gọi `app.statements(việc).await` → (kết quả, các câu SQL đã
   chạy). App của test không chạy việc nền nên số câu là của chính việc được đo. Máy chủ thật không bật. Đường nóng
   đã từng có N+1 giữ bằng test "cùng số câu khi số bản ghi trên trang tăng" (`catalog`
   `tests::statements`, `orders` `tests::statements`, `organizations` `tra_nhieu_nut_mot_cau`).
2. **Tên người bán một lần mỗi trang**: thêm `OrgDirectory::units(ids)` (cài đặt mặc định gọi `unit` từng ID nên
   cài đặt cũ, kể cả bộ giả trong test, không phải sửa; `organizations` đọc một câu `IN`). `catalog` dùng ở danh sách
   sản phẩm và bộ lọc (facets) — trước là một câu mỗi người bán.
3. **Trang sản phẩm không đọc lại**: bản ghi đè `alt` của ảnh và số giá trị tuỳ chọn của thẻ lấy từ phần đã đọc cùng
   sản phẩm (28 → 25 câu). Response không đổi.
4. **Menu cũ của `content`** (`GET /content/public/menus/{key}`, deprecated): mọi term của menu một câu thay vì một
   câu mỗi term.
5. Đặt hàng (`orders`): một quyết định riêng, cùng đợt đo này.

Lý do — số đo (20/11/2026, binary release trước / sau trên cùng máy 4 nhân dùng chung, máy tạo tải cùng máy, oha 32
kết nối 8 giây, mỗi bản 2 lượt xen kẽ, lấy trung bình; cửa hàng mẫu 20 người bán × 10 sản phẩm qua API thật; Postgres
16 sau `ANALYZE`; chi tiết ở `docs/BENCHMARKS.md` "Đường nóng của cửa hàng"):

| Đường | DB | Câu SQL trước → sau | req/s trước → sau | p50 ms trước → sau |
| --- | --- | ---: | ---: | ---: |
| danh sách 50 sản phẩm | SQLite | 14 → 10 | 302 → 345 (+14%) | 104.8 → 91.6 |
| danh sách 50 sản phẩm | Postgres | 14 → 10 | 595 → 706 (+19%) | 53.1 → 44.3 |
| trang sản phẩm (by-slug) | SQLite | 28 → 25 | 764 → 863 (+13%) | 41.4 → 36.7 |
| trang sản phẩm (by-slug) | Postgres | 28 → 25 | 550 → 621 (+13%) | 57.2 → 50.3 |
| bộ lọc (facets) | SQLite | 27 → 8 | 415 → 1167 (+181%) | 77.1 → 27.1 |
| bộ lọc (facets) | Postgres | 27 → 8 | 536 → 1134 (+112%) | 58.6 → 27.9 |
| gợi ý tìm (đối chứng, không sửa) | cả hai | 8 → 8 | ±3% | ±3% |

Đếm câu SQL mỗi request bằng `pg_stat_statements` trên máy chủ thật chạy chế độ chỉ API (`AUTOXIDE_PROCESS_ROLE=api`,
không có worker làm lẫn số) với bộ dữ liệu `small` của `scripts/bench-seed.sh`: các đường còn lại đã gộp lô, không
N+1, không sửa — `content` công khai (resolve, posts, feed 11 câu; sitemap 4; search 5; terms 3), `comments` công khai
(threads 8, comment 3), `GET /auth/me` 3 (+1 dòng nhật ký request ghi theo lô nền), `GET /auth/users` 4, `GET /audit`
3, `/kernel/find` 5, `/kernel/work` 11 (một câu mỗi nguồn việc), đăng nhập 11 (Argon2id chi phối thời gian).

Phát hiện khi đo: Postgres trên database vừa nạp lô lớn mà autovacuum chưa kịp `ANALYZE` chọn plan nested loop cho
gợi ý tìm (10.8 ms một câu với 200 sản phẩm; sau `ANALYZE` 0.5 ms). Không phải lỗi code; bài đo Postgres phải
`ANALYZE` sau khi gieo dữ liệu. Còn mở: danh sách sản phẩm trên SQLite chậm gấp đôi Postgres dưới 32 kết nối với cùng
10 câu — cần đo riêng (không trong phạm vi lần này).

Đã cân nhắc: cache tên người bán hay bảng giá của website trong bộ nhớ — phải vô hiệu hoá đúng khi đổi tên nút / bảng
giá ở mọi tiến trình, lợi nhỏ hơn gộp câu; gộp hết phần trùng còn lại của trang sản phẩm (đọc lại sản phẩm, ảnh, cây tổ
chức khi tính giá) — phải tách lại `detail` dùng chung với API quản trị, rủi ro lớn hơn lợi ~3 câu; log câu SQL rồi đếm
theo request id — `sqlx_logging` đang tắt có chủ ý, hook metric của SeaORM chính xác hơn và không đổi cấu hình log.
