## D-20261120-step39-index-review · Rà index bằng EXPLAIN trên bộ dữ liệu `full`: không thêm migration — 2026-11-20 — agent quyết

Chủ đề: Vận hành và hiệu năng

Quyết định: không thêm index nào ở bước 39. Rà bằng `EXPLAIN` trên bộ `full` (100 nghìn bài, 400 nghìn dòng bài–danh mục/thẻ,
100 nghìn `comments_targets`, 50 nghìn ảnh, 5 nghìn người dùng, 1 triệu bình luận) trên cả hai database: Postgres 16 bằng
`auto_explain` (`log_analyze`) ghi plan thật của mọi câu khi gọi hết các endpoint danh sách `GET` không cần ID của
`content`, `media`, `comments`, `auth` (kèm từng tham số lọc/`order`/`count`) cùng lọc theo danh mục / thẻ / tác giả; SQLite
chạy `EXPLAIN QUERY PLAN` cho 273 câu SELECT khác nhau lấy từ log đó, cùng tham số. Kết quả ở `docs/BENCHMARKS.md`, mục "Rà index".

Kết quả: (1) SQLite không có câu nào quét toàn bảng lớn; (2) Postgres chỉ còn quét toàn bảng ở bốn chỗ, không chỗ nào sửa được
bằng index chạy trên cả hai database (xem BENCHMARKS): tìm chuỗi con `LIKE '%…%'` trên `search_key` (đã ghi đỏ, thuộc
`content`; `pg_trgm` chỉ Postgres, FTS5 chỉ SQLite — trái Luật số một), đếm `count=true` của `content_published` (đếm cần
đọc hết), nạp toàn bộ vector lúc khởi động (D-20261119-step39-semantic-scan), hàng đợi duyệt bình luận (`pending_count > 0`);
(3) với hàng đợi duyệt, thử index `(tenant_id, pending_count, uid)` ở mỗi database với 2% target chờ duyệt: planner không
dùng (đi theo index `uid` có sẵn và dừng sau 21 dòng), thời gian không đổi (Postgres 11 → 18 ms, SQLite 24 → 22 ms, trong
nhiễu) → không thêm. Index đảo cho lọc theo danh mục / thẻ / tác giả đã có (`content_entry_terms_term_idx`,
`content_entry_authors_author_idx`, `media_item_tag_tag_idx`).

Giới hạn: chỉ bốn module có dữ liệu cỡ thật (`auth`, `content`, `media`, `comments`); bảng của module khác trống nên plan
không có ý nghĩa — rà lại khi module đó có bộ dữ liệu mẫu (việc của bước đó). `analysis_limit = 400` của SQLite
(D-20261118-step39-sqlite-optimize) cho thống kê thô (cột dẫn đầu ước ~400 dòng thay vì 400 nghìn), plan lọc theo danh mục
trên SQLite vì vậy sắp xếp tạm cả tập khớp; không đo được lợi từ index nên không đổi ở đây.

Đã cân nhắc: index từng phần (`WHERE pending_count > 0`) — SQLite không dùng khi điều kiện là tham số ràng buộc, Postgres
chỉ dùng plan riêng từng lần; index `pg_trgm` / FTS5 — vi phạm Luật số một.
