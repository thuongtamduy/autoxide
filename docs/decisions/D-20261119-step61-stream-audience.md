## D-20261119-step61-stream-audience · Stream SSE của kernel đưa event tới "người được nêu tên" — 2026-11-19 — agent quyết

Chủ đề: Kiến trúc module và kernel

Bối cảnh: bước 61 (chat) cần tin mới tới người mua gần như tức thì, và đặc tả nói "dùng stream SSE của bước 40, không
có WebSocket riêng". Stream của kernel (D-378) chỉ có hai mức: người có permission của event ở cả khách nhận mọi event,
quyền hẹp hơn chỉ nhận event chính mình gây ra. Người mua (tài khoản `customer`) không có permission nào, nên không
bao giờ nhận được tin nhân viên trả lời; nhân viên của một nhánh không nhận được tin người mua viết vào cuộc trò chuyện
giao cho mình.

Quyết định:

1. `EventInfo::to_audience()` (cài đặt mặc định: tắt, D-038): event khai nó còn tới những người có tên trong trường
   `audience` (mảng user id) của payload. Người không có permission của event vẫn mở stream được cho event đó nhưng chỉ
   nhận event nêu tên mình; người có quyền hẹp nhận việc của mình cộng event nêu tên mình; quyền ở cả khách như cũ.
2. Trường `audience` bị bỏ khỏi payload trước khi gửi ra client (danh sách người nhận không lộ). Danh mục module ghi
   `stream_audience: true` cạnh `stream_permission`.
3. Module tự quyết ai có tên trong `audience` lúc ghi event (vd `conversations`: người mua và nhân viên được giao).
   Không đổi bảng, không migration; endpoint vẫn là `GET /api/v1/kernel/events/stream`.

Đã cân nhắc: endpoint SSE riêng `/api/v1/conversations/stream` đọc bảng của module — lặp lại hub, vòng hỏi, giới hạn
kết nối của kernel (hai cách làm cho một việc); lọc theo nhánh (`org_id` trên event) — event chưa mang nút và kiểm cây
cần database ở mỗi event, để sau khi cần; WebSocket — đang làm ở bước 69, chat không cần kênh hai chiều (gửi tin đi
qua REST có `client_id` chống trùng).

Hệ quả: đặc tả bước 61 sửa: client mở `GET /api/v1/kernel/events/stream?names=conversations.*`. Test
`event_toi_nguoi_duoc_neu_ten` chạy trên cả hai database.
