## D-20261122-layout-public · Đọc công khai trang dựng từ khối: dữ liệu khối trong `blocks_data`, ETag theo cả câu trả lời; số đo p99 — 2026-11-22 — agent quyết

Chủ đề: Module nội dung, file, liên lạc

Quyết định (bước 67 mục "Đọc công khai", chi tiết hoá D-229 mục 5):

1. `GET /api/v1/content/public/resolve` và link xem trước trả thân `layout` đã bỏ khối khách không còn dùng được
   (module tắt, gói không còn tính năng) cùng `blocks_data` (`id` khối → dữ liệu). Danh sách bài / trang công khai
   không đọc dữ liệu khối (`blocks_data` rỗng): chỉ trang được mở mới cần, và một danh sách N trang sẽ thành N lần gọi.
2. Dữ liệu khối đọc qua `Blocks::resolve` của kernel với `Site` của request (website đang xem) và ngôn ngữ của
   mục: mỗi loại khối một lần gọi, song song, hạn 2 giây; khối lỗi / hết giờ / tham chiếu không còn công khai là
   `null`, trang vẫn trả 200. Xem trước dùng website của nút của mục (`Site::of`), không có thì cả khách.
3. `ETag` giữ cách sẵn có — dấu vân tay SHA-256 của cả câu trả lời — nên tự gồm dữ liệu khối: giá đổi thì ETag đổi,
   không cần mỗi `BlockData` báo "phiên bản dữ liệu" riêng như phác thảo ở đặc tả. `Cache-Control` giữ
   `public, no-cache` (hỏi lại máy chủ mỗi lần, 304 khi không đổi): trang có giá không bị cache cũ.
4. Số đo (test `doc_cong_khai_tra_du_lieu_khoi_theo_lo`, trang 50 khối gồm 20 khối có dữ liệu, 200 lần gọi qua router
   thật, bản debug, máy phát triển): SQLite p50 ≈ 3,1 ms, p99 ≈ 5,8 ms; Postgres p50 ≈ 3,7 ms, p99 ≈ 10,4 ms. Test
   chỉ chặn p99 dưới 2 giây (không chặn theo máy); số ở đây để so khi đổi đường đọc.

Lý do: dùng lại đúng đường đọc công khai và ETag sẵn có, thêm đúng một lần gọi mỗi loại khối.

Đã cân nhắc: phiên bản dữ liệu do từng `BlockData` báo (thêm hợp đồng cho mọi module mà ETag theo nội dung đã đủ);
`Cache-Control: max-age` ngắn (giá có thể cũ trong khoảng đó).
