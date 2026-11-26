## D-20261126-permission-gate · Kiểm permission ở một lớp HTTP chung đọc `x-permissions`, trước mọi extractor và service — 2026-11-26 — agent quyết

Chủ đề: HTTP, REST, OpenAPI

Quyết định: kernel đặt một lớp (`http/permission_gate.rs`) ở NGOÀI CÙNG các route của mỗi module, dựng từ tài liệu
OpenAPI của module như `approvals::rules`: operation có `x-permissions` khác rỗng thì người gọi đã xác thực phải có ít
nhất một permission trong danh sách (ở bất kỳ nút nào, `Principal::require_any_named`), không thì 403
`permission_denied` — trước extractor của handler (thân JSON, `If-Match`, tham số), trước gói (`plan` gate), duyệt của
agent và trước khi service tra bản ghi. Không xác thực được thì lớp để handler trả 401 như cũ; `x-permissions: []` và
operation công khai không bị chặn. Phạm vi (nút), bản ghi, step-up vẫn do service kiểm. `x-permissions` vì thế là hợp
đồng được THI HÀNH: thiếu một permission mà service vẫn nhận thì người giữ nó bị 403 — test e2e
`x_permissions_khop_cho_service_kiem` (mỗi permission một tài khoản, gọi mọi operation cùng module) giữ hai bên khớp, và
`kiem_quyen_truoc_du_lieu_va_ban_ghi` giữ thứ tự (thân sai, thiếu `If-Match`, ID không có → vẫn 403).

Endpoint của người mua mà nhân sự cũng gọi được với một permission (ví voucher, nhắc flash sale của `promotions`) khai
thêm `x-buyers: true`: tài khoản người mua (không bao giờ có permission, D-309) qua lớp này, service quyết phần còn lại;
nhân sự vẫn cần một permission trong danh sách. Bài dò PT-13 chỉ thử permission CÙNG module với operation (thử mọi cặp
là hàng trăm nghìn request); operation nhận permission của module khác sẽ lộ ra ở test của module đó — chạy test toàn
workspace đã bắt hai chỗ như vậy (`promotions` người mua, `support` thành viên tự tắt nhận việc) trước khi bật.

Lý do: pentest 2026-11 (PT-14) — khoảng 210 operation trả 428 / 412 / 422 / 404 trước khi kiểm permission, người không
quyền học được endpoint cần `ETag`, trường nào sai, bản ghi nào tồn tại; đo lại trên toàn bộ module là 372 operation.
Sửa từng handler (đổi thứ tự extractor, gọi `require` trước khi tra bản ghi) không giữ được với module mới; một lớp đọc
cùng nguồn với tài liệu thì sửa một lần cho mọi module, kể cả module viết ngoài repo. PT-13 (`x-permissions` thiếu
permission service vẫn nhận) đã sửa 18 operation trước khi bật lớp này, để không chặn oan.

Đã cân nhắc: một extractor `RequirePermission<P>` đặt đầu danh sách tham số của từng handler (phải sửa hơn 1000 handler,
dễ quên); kiểm trong `ValidJson` / `IfMatch` (không biết permission của route); sinh `x-permissions` từ lời gọi `require`
trong code (không đọc được lúc biên dịch, D-205).
