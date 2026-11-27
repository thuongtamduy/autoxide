## D-20261127-oauth-private-key-jwt-later · `private_key_jwt` cho client tài liệu metadata để sau 1.0 — 2026-11-27 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: Máy chủ uỷ quyền OAuth của `auth` (D-20261126-mcp-oauth) giữ chỉ public client (`token_endpoint_auth_method`
`none` + PKCE `S256` + refresh token xoay vòng) tới sau bản 1.0. Không nhận `private_key_jwt` (RFC 7523) cho client
tài liệu metadata ở bản này; metadata tiếp tục khai `token_endpoint_auth_methods_supported: ["none"]`, và tài liệu
metadata khai phương thức khác vẫn bị từ chối như hiện tại.

Lý do: không nhỏ và không chỉ thêm code — nó thêm một lớp bảo mật mới ngay giai đoạn đóng băng tính năng: tải và cache
JWKS (`jwks_uri`) qua bộ lọc SSRF với hạn mức riêng, kiểm chữ ký nhiều thuật toán (RS256 / ES256 / EdDSA) và danh sách
thuật toán cho phép, kiểm `aud` / `iss` / `sub` / `exp`, chống phát lại `jti` (một bảng mới, ghi đồng thời, chạy và test
trên cả hai database), xoay khoá, và một loại client "confidential" mà D-20261126-mcp-oauth mục 5 đã để sau. Client MCP
hiện có (Claude, ChatGPT, IDE, SDK chính thức) đều là public client; PKCE + refresh xoay vòng + thu hồi khi dùng lại đã
bảo vệ chúng; đặc tả MCP 2026-07-28 chỉ nêu `private_key_jwt` là tuỳ chọn. Thêm sau là thay đổi thêm vào (metadata khai
thêm phương thức), không phá client cũ, nên để sau 1.0 không làm khó nâng cấp. Làm khi có client MCP thật cần nó, cùng
lúc với client confidential đăng ký trước, và qua kiểm độc lập (D-20261126-independent-review).

Đã cân nhắc: làm ngay trong PR 3 của bước 71 G4 — lớn hơn phần còn lại của PR và chạm xác thực khi chưa có người dùng
cần; chỉ nhận `jwks` nhúng trong tài liệu (bỏ `jwks_uri`) — vẫn cần chống phát lại `jti` và kiểm chữ ký, lại lệch thói
quen của client thật (dùng `jwks_uri` để xoay khoá).
