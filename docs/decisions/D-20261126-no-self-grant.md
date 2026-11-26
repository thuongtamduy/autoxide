## D-20261126-no-self-grant · Gán role và sửa role theo luật "chỉ cấp cái mình có", cả cho chính mình — 2026-11-26 — chủ dự án quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: chặn (PT-9, nợ P16). `PUT /api/v1/auth/users/{id}/roles` và tool MCP `set_user_roles` dùng cùng luật với
membership, lời mời và SCIM (D-20261121-grant-scope-check, D-20261125-scim-grant-cover): mỗi role được THÊM hay bị GỠ
ở nút gốc phải là role người gọi đang cấp được (`require_grantable` / `grant_problems`, không viết luật thứ hai), áp cho
mọi người được gán, kể cả chính người gọi; không thì 403 `auth.role_beyond_you`, không đổi gì. Đường tương đương là sửa
luật của role mình đang giữ: `PUT /api/v1/auth/roles/{id}` và tool `replace_role` không được thêm permission, nới phạm vi
của permission, hay bỏ một permission cấm, khi người sửa không có permission đó ít nhất rộng như vậy ở nút gốc — 403
`auth.permission_beyond_you`.

Gỡ một role (khỏi một người qua `PUT /users/{id}/roles` hay `PUT /users/{id}/memberships`, hoặc xoá hẳn role qua
`DELETE /roles/{id}` — cùng các tool MCP `set_user_roles`, `set_memberships`, `delete_role`) là bỏ cả permission CẤM của
role; cấm thắng mọi role khác nên bỏ cấm là mở thêm quyền. Một luật dùng chung (`grantable`, đường gỡ): người gọi phải
trao được các permission cấp của role như trước VÀ có mọi permission role cấm (`allows`), cùng luật với bỏ cấm khi sửa
role. Membership bị bỏ ở `PUT /users/{id}/memberships` nay cũng qua luật này (trước đây chỉ kiểm role được cấp). Nới
`requires_mfa` của một role (kể cả role `admin`) đòi người sửa trao được role đó; siết thì không cần.

Kiểm ở service (`set_user_roles_as`, `update_as`, `delete_as`, `set_memberships`), trong transaction ghi; đường hệ thống
(`set_user_roles`, `update` với `Actor::system()`: lệnh CLI, test) giữ như cũ. Tạo role mới với permission mình không có
vẫn được: role chưa ai giữ, gán nó thì qua luật trên.

Không khoá khách khỏi chính họ: role `admin` (mọi permission, không bị rào) cấp được mọi role; luật "không bỏ admin cuối
cùng" giữ nguyên. Đổi hành vi của endpoint cũ trước 1.0 theo D-20261125-api-break-before-1-0: ghi ở changelog
**Security** và **Changed**.

Lý do: chủ dự án quyết chặn sau pentest 2026-11 — `auth.roles.manage` cho tự gán mọi role kể cả `admin`, nên ai giữ nó
(hay một API key của họ) thực chất là quản trị viên; trong khi membership, lời mời và SCIM đã theo "chỉ cấp cái mình có".
Sửa role mình giữ là cùng một lỗ qua cửa khác, nên chặn cùng lúc.

Đã cân nhắc: chỉ chặn khi gán cho chính mình (người quản lý role vẫn cấp `admin` cho đồng phạm rồi được cấp lại); chỉ
chặn role `admin` (role tự tạo với mọi permission đi vòng được); bắt người sửa role phải cấp được role ở mọi nút đang có
người giữ (đúng hơn với người quản lý theo nhánh, nhưng phải đọc mọi membership của role — kiểm ở nút gốc chặt hơn và đủ
cho người quản lý role toàn khách, trường hợp thường gặp).
