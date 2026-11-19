## D-20261118-step54-role-denies · Bước 54: role cấm permission (deny thắng allow) — 2026-11-18 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: Role có thêm danh sách `denies`. Lưu trong chính `auth_role_permission` với cột mới `effect`
(`allow` / `deny`, migration `m0037_role_denies`), nên quyền của người gọi vẫn nạp bằng một câu (D-319). Permission
bị một role cấm thì không role nào khác của người dùng cấp lại được, kể cả role `admin`: kernel thêm
`Principal::without(denied, catalog)`; người toàn quyền mà có role cấm thì thành người có mọi permission của danh
mục trừ phần bị cấm. Áp ở cả ba chỗ dựng quyền: đăng nhập, uỷ quyền sang khách con, lần vào hỗ trợ. Lỗi
`permission_denied` có `reason`: `denied_by_role` hay `missing`. Một role không vừa cấp vừa cấm cùng permission
(422 `conflict`); role `admin` dựng sẵn không nhận `denies`. Người giữ role cấm **không tính là quản trị viên**
khi đếm quản trị viên còn hoạt động. Nhờ vậy chốt "quản trị viên cuối cùng" (`auth.last_admin`) chặn cả việc gán
role cấm cho quản trị viên cuối, lẫn việc thêm `denies` vào role mà người đó đang giữ.

Lý do: Đặc tả 54 P4 xếp thứ tự quyết định cố định: rào → deny → allow → từ chối. Đặt deny ở bảng riêng thì
đường nóng thêm một câu hay một JOIN nhân dòng. Cột `effect` giữ một câu mà không đổi khoá của bảng. Không chặn
quản trị viên cuối thì một role cấm `auth.roles.manage` gán nhầm cho người đó sẽ khoá cả khách: không ai còn sửa
được role.

Đã cân nhắc: (1) Cấm kết hợp role `admin` với mọi role có `denies`. Bị loại vì quá chặt: khách có nhiều quản trị
viên vẫn cần hạn chế một người. (2) Lưu deny dưới dạng `scope = "deny"`. Bị loại vì code cũ đọc giá trị lạ thành
`tenant`: hạ phiên bản sẽ biến cấm thành cấp.
