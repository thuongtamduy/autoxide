## D-20261121-access-token-revocation · Access token của phiên đã kết thúc bị từ chối ngay — 2026-11-21 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: bộ xác thực của `auth` kiểm phiên (`sid` trong access token = `family_id` của refresh token) còn sống —
họ còn một refresh token chưa dùng, chưa thu hồi — trong **cùng một câu** đã đọc người dùng và quyền (D-319): thêm
cột `EXISTS (…)` trên index `auth_refresh_token_family_idx (tenant_id, family_id)`. Phiên đã kết thúc trả 401
`auth.session_ended`. Áp cho mọi đường thu hồi có sẵn: đăng xuất, `DELETE /me/sessions/{id}`, đổi / đặt lại mật khẩu,
quản trị viên đăng xuất người dùng, phát hiện dùng lại refresh token. Token cũ không có `sid` chỉ kiểm người dùng như
trước. API key không đổi (đã tra database mỗi request).

Lý do: người thử bản 0.17.0 hỏi đăng xuất có hiệu lực ngay không; trước đây access token còn dùng được tới khi hết hạn
(mặc định 15 phút) dù phiên đã thu hồi. Mỗi request đã có một câu tra người dùng, nên thêm điều kiện vào câu đó không
thêm vòng mạng nào, chạy như nhau trên SQLite và Postgres. Đọc qua kết nối đọc: read replica trễ chỉ làm phiên vừa thu
hồi còn sống thêm khoảng trễ đó, không bao giờ từ chối nhầm phiên còn sống (đổi refresh token ghi dòng cũ và dòng mới
trong một transaction).

Đã cân nhắc: danh sách token bị thu hồi trong bộ nhớ (không chia được giữa nhiều máy chủ); rút ngắn thời hạn access
token (vẫn có cửa sổ, thêm refresh); một câu riêng tra bảng phiên (thêm một vòng mạng mỗi request trên Postgres).
