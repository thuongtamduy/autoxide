## D-20261121-registration-uniform · `POST /registrations` trả lời như nhau dù email đã có tài khoản; `/register` deprecated — 2026-11-21 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: Thêm `POST /api/v1/auth/registrations`, cùng thân với `/register`, luôn trả 202 với `{"data": {"email":
…}}` cho đầu vào hợp lệ. Email chưa có tài khoản: tạo tài khoản (chưa xác minh) và gửi mail xác minh như cũ. Đã có:
không đổi gì, gửi chủ email thư báo "có người thử tạo tài khoản bằng email của bạn" (mỗi tài khoản nhiều nhất một thư
mỗi 60 giây). Cả hai nhánh đều kiểm đầu vào và hash mật khẩu trước, nên thời gian trả lời không khác. `POST /register`
(trả 409 `auth.email_taken`) khai deprecated từ 2026-11-21, sunset 2027-05-21, thay bằng endpoint mới (D-059).

Lý do: 409 cho biết email nào đã có tài khoản — đúng thứ `/login`, `/password-reset`, `/login/link` đã giấu. Đổi
hành vi của `/register` là thay đổi phá vỡ nên làm endpoint mới.

Đã cân nhắc: trả 201 giả cho email đã có — thân response phải chứa tài khoản, không giả được mà không nói dối client.
