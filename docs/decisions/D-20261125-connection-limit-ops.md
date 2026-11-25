## D-20261125-connection-limit-ops · Giới hạn kết nối: tính cả WebSocket, kết nối im lặng không giữ graceful shutdown, cảnh báo khi vượt `ulimit -n` — 2026-11-25 — agent quyết

Chủ đề: HTTP, REST, OpenAPI

Quyết định: bổ sung D-20261125-max-connections sau khi kiểm độc lập #505.

1. **WebSocket được đếm.** "Vé" của `AUTOXIDE_HTTP_MAX_CONNECTIONS` gắn vào chính socket (kiểu `Counted` bọc
   `TcpStream` trong `crates/kernel/src/http/server.rs`), không vào task phục vụ kết nối: khi nâng cấp lên WebSocket,
   hyper trao chính socket cho task của WebSocket nên vé giữ tới khi socket đóng.
2. **Tắt máy chủ không chờ kết nối im lặng.** Bước chờ byte đầu dừng ngay khi có tín hiệu dừng, nên một kết nối vừa mở
   mà chưa gửi gì không giữ graceful shutdown tới 30 giây.
3. **`ulimit -n`.** Mặc định 4096 lớn hơn giới hạn file mở 1024 thường gặp: lúc khởi động `autoxide serve` đọc
   `/proc/self/limits` (Linux; không cần `unsafe`) và cảnh báo khi giới hạn kết nối cộng 64 chỗ dự phòng không thấp hơn
   giới hạn mềm. Unit systemd mẫu trong `docs/DEPLOY.md` đặt `LimitNOFILE=8192`. Không tự nâng giới hạn của process
   (cần `setrlimit`, tức `unsafe` hay thêm thư viện).
4. **Health check khi đầy.** Chạm giới hạn thì `/health/ready` cùng port cũng chờ — readiness chậm đưa máy ra khỏi vòng
   phân tải là đúng ý; liveness thì đặt timeout đủ chịu một lúc quá tải ngắn (`deploy/kubernetes/autoxide.yaml`:
   `timeoutSeconds: 5` × 3 lần), runbook "Too many connections" trong `docs/DEPLOY.md`. Không dành chỗ riêng cho health
   check: phải đọc request mới biết đường dẫn, tức đã nhận kết nối rồi.

Lý do: giới hạn không đếm WebSocket thì một đợt mở WebSocket vẫn làm hết file descriptor; kết nối im lặng giữ shutdown
có thể ra `TimedOut` với grace mặc định 30 giây; giới hạn lớn hơn `ulimit -n` thì không có tác dụng mà người cài không biết.

Đã cân nhắc: ghi WebSocket là ngoại lệ (chỉ dựa `AUTOXIDE_EVENTS_STREAM_MAX_CONNECTIONS`) — hai giới hạn cộng lại khó
đoán; hạ mặc định xuống dưới 1024 — quá thấp cho máy chạy thẳng không proxy với `LimitNOFILE` đã nâng.
