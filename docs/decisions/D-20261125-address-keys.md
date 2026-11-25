## D-20261125-address-keys · Giới hạn theo địa chỉ client đếm theo một IPv4 hay một khối /64 IPv6, qua một hàm của kernel — 2026-11-25 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: mọi giới hạn đếm theo địa chỉ client (bảng `<module>_throttle`) dựng khoá bằng
`autoxide_kernel::throttle::address_key(kind, ip)` — `kind` + client của địa chỉ (`ip_hash::client`: đúng địa chỉ
với IPv4, khối /64 với IPv6; IPv4 viết dạng IPv6 tính là IPv4) — cùng cách gom với rate limiter HTTP (D-157). Đã chuyển: mã một lần SMS của `compliance` (đăng nhập bằng số và xác thực số),
mã đặt lịch khách vãng lai của `bookings`, tra đơn khách vãng lai của `orders`, cặp email + địa chỉ của login
throttle `auth`, giới hạn đăng ký bằng số của `auth`. Luật `address-key` của `cargo xtask check` chặn chuỗi `format!`
có `ip:{` (khoá kiểu `otp-ip:{ip}`); khoá đã gom theo khối bằng cách khác (`ip_hash::bucket` của anti-spam và cookie
consent, gom /24 theo D-115) ghi `// address-key: <lý do>`.

Lý do: pentest 2026-11 (PT-4) — sau reverse proxy, xoay IPv6 trong một /64 gửi được 30/30 mã SMS tới 30 số (cửa hàng
trả tiền mỗi tin) vì khoá là địa chỉ đầy đủ. Một client IPv6 thường giữ cả /64; `auth` đã đếm đăng ký bằng số theo
khối, các chỗ khác tự viết khoá và quên. Một hàm + một luật gate thì module mới không lặp lại lỗi. Đổi dạng khoá làm
các bộ đếm đang chạy bắt đầu lại từ 0 khi nâng cấp (cửa sổ tối đa một giờ) — chấp nhận được.

IPv4 không gom theo /24: lỗ của PT-4 là xoay địa chỉ trong /64 IPv6 (một client giữ cả khối); mỗi địa chỉ IPv4 là
một tài nguyên thật, còn khách dùng mạng di động ở Việt Nam hay đứng sau NAT của nhà mạng (CGNAT) chung một /24 —
gom /24 làm đăng nhập SMS của họ bị 429 oan dù hạn mức (20 mã mỗi giờ) không đổi.

Đã cân nhắc: gom IPv4 theo /24 như `ip_hash::prefix` (chặn oan sau CGNAT; muốn giữ thì phải nâng mọi ngưỡng mặc định); sửa tại chỗ từng module (lặp lại được ở module sau); gom theo /48 cho IPv6 (chặn oan cả một tổ chức hay
nhà mạng cấp /48 cho nhiều nhà); băm khối bằng `ip_hash::bucket` cho mọi khoá (bảng throttle đã băm SHA-256 khoá trước
khi lưu, thêm HMAC không thêm gì).
