## D-20261125-tenant-site-verified · Nhiều khách: `kernel.public_url` và link trong thư chỉ trên tên miền đã xác minh của chính khách — 2026-11-25 — agent quyết

Chủ đề: Đa khách, tổ chức, gói

Quyết định: Ở chế độ nhiều khách, `kernel.public_url` và host của mọi link mẫu (`<module>.link.*`,
D-20261125-tenant-link-templates) chỉ được là (a) tên miền ĐANG `verified` của chính khách, hoặc (b) host nền tảng
(`AUTOXIDE_HTTP_PUBLIC_URL`); chỉ `https`, chỉ port mặc định. Mọi trạng thái khác — `pending` (kể cả sau khi xác minh
lại thất bại hay khai lại tên miền đã thu hồi), `failed`, `revoked`, chưa từng khai, của khách khác — lúc đặt trả 422
`kernel.public_url_not_allowed` / `kernel.link_host_not_allowed`, lúc dùng bị bỏ qua (warn, quay về link của máy chủ).
Giá trị đã lưu trước luật này không bị xoá: lúc dùng bỏ qua, và `GET /api/v1/kernel/settings` (cả bản cho nhân sự nền
tảng) gắn `problem: kernel.public_url_unverified` / `kernel.link_unverified` để admin biết. Chủ tên miền đọc từ ảnh
chụp tên miền đã xác minh trong bộ nhớ (`verified_domain_owner`, dùng chung với CORS — D-20261125-tenant-cors): không
truy vấn thêm mỗi thư; tên miền vượt trần của ảnh chụp không có trong đó nên không được tin (fail closed). Chế độ một
khách giữ như cũ (không có bảng tên miền để đối chiếu): link mẫu trên host nền tảng hay địa chỉ của `kernel.public_url`;
`kernel.public_url` mới phải `https` trừ `localhost` / `127.0.0.1` (phát triển) — giá trị `http` đã lưu vẫn dùng.

Lý do: phiên kiểm bảo mật (rà #516 vòng ba) tìm ra đường mở lại: tên miền `failed` (hết hạn, người lạ mua lại), admin
làm theo thông báo "xác minh lại" → xác minh thất bại đưa dòng về `pending` (hay khai lại dòng `revoked`) → dấu "đã
mất" biến mất → token đặt lại mật khẩu của người dùng khách tới chủ mới của tên miền. Chỉ tin tên miền đang `verified`
đóng cả lỗ này lẫn rủi ro còn lại P16 (host `kernel.public_url` chưa ai xác minh). Siết luật của một setting sau 1.0 là
thay đổi phá vỡ, nên làm trước 1.0 (D-20261125-api-break-before-1-0): khách nhiều-khách đang đặt `kernel.public_url`
trên host chưa xác minh phải xác minh tên miền (changelog mục Changed, "Action needed").

Đã cân nhắc: giữ dấu "đã mất" (dòng revoked / failed) — bị xoá bởi lượt xác minh lại thất bại; theo dõi lịch sử chủ
tên miền — phức tạp hơn mà vẫn tin host chưa ai xác minh. Xoá giá trị cũ không thoả — mất cấu hình của khách không báo;
bỏ qua lúc dùng + cờ trong GET thì khách xác minh xong là chạy lại ngay.

Thay: phần "Rủi ro còn lại" của D-20261125-tenant-link-templates
