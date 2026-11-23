## D-20261123-http-header-timeout · Máy chủ HTTP tự phục vụ kết nối bằng `hyper-util` có timer: timeout đọc header 30 giây, ping HTTP/2 — 2026-11-23 — agent quyết

Chủ đề: HTTP, REST, OpenAPI

Quyết định: `autoxide_kernel::http::serve` không dùng `axum::serve` nữa mà tự nhận kết nối và phục vụ bằng
`hyper_util::server::conn::auto::Builder` có `TokioTimer`: HTTP/1 phải gửi xong header trong 30 giây
(`header_read_timeout`), HTTP/2 ping kết nối im lặng mỗi 60 giây (đóng nếu 20 giây không trả lời). Đóng êm giữ
như cũ (ngừng nhận, báo từng kết nối `graceful_shutdown`, chờ tối đa `grace`); mỗi request vẫn mang `ConnectInfo`;
WebSocket (nâng cấp HTTP/1, CONNECT của HTTP/2) giữ nguyên. Thêm `hyper` và `hyper-util` vào
`[workspace.dependencies]` — cả hai đã có trong cây phụ thuộc qua axum, không thêm mã mới vào binary.

Lý do: rà soát bảo mật 2026-11 (M-5 của `docs/security/review-2026-11.md`): `axum::serve` 0.8 dựng builder của
hyper không có timer, mà hyper 1.x chỉ áp `header_read_timeout` khi có timer — client nhỏ giọt header giữ kết nối
mãi (slowloris), rate limiting không thấy request nào. Timeout của tầng `tower-http` chỉ chạy sau khi có request.

Đã cân nhắc:
- Bọc IO bằng timeout đọc: giết luôn SSE / WebSocket (client im lặng hợp lệ hàng chục phút).
- Chờ axum thêm cấu hình: chưa có ở 0.8; để mặc định là lỗ hổng mở ngay bây giờ.
- Giới hạn số kết nối đồng thời: để cho reverse proxy (ghi ở báo cáo, L-10), tránh từ chối nhầm người dùng thật
  sau NAT.
