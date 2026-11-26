## D-20261126-require-any-step-up · `require_any` đòi xác nhận lại chỉ khi mọi permission đang có đều cần — 2026-11-26 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: thay đoạn về step-up của `Principal::require_any` trong D-20261126-permission-audit ("theo permission đầu
tiên người gọi có"). `require_any(&[&A, &B, …])` cho qua khi người gọi dùng được ít nhất một permission trong danh
sách mà họ có: permission thường, hay permission rủi ro cao (destructive, money) với phiên vừa xác nhận lại. Chỉ khi
MỌI permission trong danh sách mà người gọi có đều đòi xác nhận lại và phiên đã cũ thì trả `step_up_required`. Thứ tự
danh sách không đổi kết quả.

`require_any` chỉ nói người gọi được bắt đầu việc. Chỗ service thật sự làm phần rủi ro cao (xoá hẳn, chuyển tiền)
vẫn gọi `Principal::require` của permission đó — bước xác nhận lại được đòi ở đó.

Lý do: rà soát độc lập #541 — theo "permission đầu tiên người gọi có" thì kết quả phụ thuộc thứ tự module viết danh
sách: người có cả `x.delete` (rủi ro cao) lẫn `x.delete_own` bị đòi xác nhận lại hay không tuỳ cái nào đứng trước.

Đã cân nhắc: đòi xác nhận lại khi BẤT KỲ permission đang có nào cần — chặn oan người chỉ dùng phần quyền thường.

Thay: phần step-up của `require_any` trong D-20261126-permission-audit
