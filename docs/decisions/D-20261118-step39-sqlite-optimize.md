## D-20261118-step39-sqlite-optimize · SQLite tự lập thống kê cho bộ lập kế hoạch truy vấn (`PRAGMA optimize`) — 2026-11-18 — agent quyết

Chủ đề: Vận hành và hiệu năng

Quyết định: kết nối ghi SQLite của kernel (`Database::connect`) đặt `PRAGMA analysis_limit = 400` rồi chạy
`PRAGMA optimize = 0x10002` lúc mở, và `optimize` lần nữa khi đóng (`optimize_on_close` của sqlx). Cờ `0x10002` xét mọi
bảng và chỉ phân tích bảng cần (chưa có thống kê hay đã đổi nhiều); `analysis_limit` giới hạn mỗi index đọc ~400 dòng,
nên chạy nhanh cả khi bảng hàng triệu dòng (2,4 s cho cả database đo 6,3 GB, phần lớn là lần đầu). Postgres không đổi:
autovacuum đã tự `ANALYZE`.

Lý do: đo trên bộ dữ liệu `full` của bước 39 (1 triệu bình luận) — chưa từng có `sqlite_stat1`, SQLite chọn index
`(tenant_id, id)` cho luồng bình luận của một bài (vì `ORDER BY id`) và quét gần hết bảng: 0,4 s một câu, `GET
/comments/public/threads` 3,7 s một request, dưới tải 64 kết nối không request nào xong trong 10 s. Sau `ANALYZE` SQLite
chọn đúng `comments_comments_thread_idx`: 0,4 ms. Đây là lỗi của mọi bảng lớn trên SQLite, không riêng `comments`, nên
sửa một lần ở kernel thay vì thêm index hay viết lại câu SQL từng module. Không đổi tính đúng: chỉ là thống kê.

Giới hạn: máy chủ chạy rất lâu không khởi động lại thì thống kê theo dữ liệu lúc mở; mỗi lần triển khai / khởi động lại
cập nhật. Nếu số đo cho thấy cần, thêm job định kỳ chạy `PRAGMA optimize` (mục sau, không làm trước khi đo được cần).

Đã cân nhắc: thêm index hay viết lại điều kiện `visible_root` của `comments` (chỉ sửa một chỗ, các bảng lớn khác vẫn
chọn sai); chạy `ANALYZE` đầy đủ (chậm trên bảng lớn, giữ write lock lâu); job định kỳ ngay bây giờ (chạm module kernel
`builtin` đang có phiên khác sửa, chưa đo được cần).
