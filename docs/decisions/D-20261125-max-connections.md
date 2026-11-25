## D-20261125-max-connections · Máy chủ HTTP giới hạn số kết nối mở (`AUTOXIDE_HTTP_MAX_CONNECTIONS`, mặc định 4096) và đóng kết nối không gửi byte nào — 2026-11-25 — agent quyết

Chủ đề: HTTP, REST, OpenAPI

Quyết định: `autoxide_kernel::http::serve_with_limit` giữ tối đa `AUTOXIDE_HTTP_MAX_CONNECTIONS` kết nối mở trên port
API (mặc định `4096`, `0` là không giới hạn); binary dùng hàm này. Hết chỗ thì vòng nhận kết nối chờ một `Semaphore`
(async) trước khi `accept`: kết nối mới nằm trong hàng đợi của hệ điều hành chứ không bị từ chối, và được phục vụ ngay
khi một kết nối đóng. Mọi kết nối (cả `serve` cũ) phải gửi byte đầu trong 30 giây (cùng timeout đọc header), không thì
bị đóng. `serve` giữ chữ ký và không giới hạn số kết nối (cổng `/metrics`, module ngoài dùng). Sửa L-10 của
`docs/security/review-2026-11.md`.

Lý do: không có giới hạn thì một client mở hàng chục nghìn kết nối rỗi làm process hết file descriptor — mọi người
dùng khác bị từ chối. Chờ thay vì từ chối để giới hạn không làm hỏng người dùng thật sau một địa chỉ chung (lý do
D-20261123-http-header-timeout từng để việc này cho reverse proxy). Kết nối im lặng từ đầu nằm ở bước hyper-util đoán
HTTP/1 hay HTTP/2, nơi timeout đọc header của hyper chưa chạy: kết nối như vậy giữ chỗ mãi, nên phải có timeout riêng
(`TcpStream::peek`, không lấy byte khỏi socket). Đã đọc mã hyper 1.12: kết nối HTTP/1 keep-alive rỗi sau một request
đã bị timeout đọc header đóng sau 30 giây.

Đã cân nhắc: giới hạn theo từng địa chỉ IP — người dùng sau NAT hay proxy chung bị chặn nhầm, việc của reverse proxy;
từ chối (đóng ngay) kết nối vượt giới hạn — trình duyệt và client báo lỗi thay vì chờ chút; mặc định không giới hạn —
người cài chạy thẳng không có proxy thì vẫn hở.

Thay: phần "Giới hạn số kết nối đồng thời: để cho reverse proxy" của D-20261123-http-header-timeout.
