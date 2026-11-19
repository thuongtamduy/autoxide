## D-20261119-step-up-writes · Xác nhận lại chỉ cho việc ghi rủi ro cao — 2026-11-19 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: Xác nhận lại ("sudo mode") chỉ áp cho permission có `risk` `destructive` hay `money`. Permission
`sensitive_data` (đọc dữ liệu cá nhân của người khác) không đòi xác nhận lại: quyền đọc đã có permission riêng và
audit. Token phát trước khi có `auth_time` vẫn bị đòi xác nhận lại khi làm việc `destructive` / `money`. Nếu sau
này cần, xác nhận lại cho việc đọc chỉ làm riêng cho thao tác xuất / tải hàng loạt.

Lý do: Sau khi #217 vào `main`, test nâng cấp đỏ. Access token của bản trước (không có `auth_time`) gọi
`GET /api/v1/forms/{id}` (`forms.read`, `sensitive_data`) và nhận 401 `step_up_required`: nâng cấp làm người dùng mất
quyền đọc. Ngoài ra, cứ 15 phút bắt xác nhận lại chỉ để đọc là quá nặng cho người dùng. GitHub "sudo mode" cũng chỉ
đòi cho việc thay đổi rủi ro cao. Phiên điều phối phát hiện lỗi và đề xuất hướng sửa.

Thay: phần "Permission có `risk` `destructive`, `money` hay `sensitive_data`" của D-20261119-step54-step-up
