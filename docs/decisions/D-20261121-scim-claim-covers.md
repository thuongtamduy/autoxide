## D-20261121-scim-claim-covers · SCIM chỉ tác động lên tài khoản mà người tạo token bao trùm quyền — 2026-11-21 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: Khi `POST /Users` nhận về tài khoản nhân sự có sẵn, và khi PUT / PATCH đổi email hay `active` của tài
khoản đã liên kết, người tạo token SCIM (`created_by`) phải có mọi permission của tài khoản đích (cùng luật với quản
trị viên, D-079); không thì 403. Người tạo đã bị xoá hay khoá coi như không có permission nào (xoay token). Email đổi
qua SCIM chỉ coi là đã xác minh với tài khoản do SCIM tạo (liên kết và tài khoản sinh cùng lúc); tài khoản có sẵn được
nhận về thì email mới chờ chủ tài khoản xác minh lại.

Lý do: token SCIM là một đường quản trị tài khoản; không có luật này thì người chỉ có `auth.scim.manage` tác động được
lên tài khoản quyền cao hơn mình, điều D-079 đã chặn ở REST. Tài khoản do SCIM tạo thì hệ thống danh tính là nguồn
của email (D-20261121-scim mục 7); tài khoản có sẵn thì email do chủ nó xác minh, không nên bị thay bằng email chưa
ai xác nhận rồi coi là đã xác minh.

Đã cân nhắc: thêm cột đánh dấu "nhận về" ở `auth_scim_users` — cần migration; liên kết và tài khoản do SCIM tạo có
cùng `created_at` (ghi cùng một giá trị trong một transaction) nên phân biệt được mà không thêm cột. Chặn hẳn việc
nhận về — khách bật SCIM sau khi đã có nhân viên sẽ không dùng được.

Thay: phần "email coi là đã xác minh" của D-20261121-scim (chỉ với tài khoản được nhận về)
