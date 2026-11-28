## D-20261128-scim-owners · Thành viên nhóm SCIM chờ duyệt; nâng tài khoản cao hơn người tạo liên kết SCIM thì thu hồi cửa vào cũ — 2026-11-28 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định (P16 của bước 70, module `auth`, migration `m0049_scim_owners`):

1. **Thành viên chờ duyệt.** `auth_scim_group_members.added_by` ghi người tạo token SCIM đã đưa người vào nhóm. Luật ánh
   xạ của nhóm chỉ cấp (nút, role) cho thành viên mà `added_by` — tính quyền **lúc này** như token SCIM
   (`ScimService::authority_of`: còn hoạt động, là nhân sự, còn `auth.scim.manage`) — cấp được (`role::grantable`); còn
   lại thành viên **chờ duyệt** cho cặp đó: không cấp, và dòng SCIM đã cấp trước đó cho cặp đó **không bị gỡ** (không
   đổi gì cho người đang giữ khi người đưa vào bị hạ quyền). Một chỗ quyết: `ScimService::wanted_grants`
   (`scim/owners.rs`), mọi lần đồng bộ đi qua đó. `POST /api/v1/auth/scim/group-mappings` trả thêm `waiting_user_ids`.
   Duyệt: `POST /api/v1/auth/scim/groups/{id}/members/{user_id}/approve` (`auth.scim.manage`, người duyệt phải cấp được
   mọi luật của nhóm, 403 `auth.role_beyond_you`; 404 `auth.scim_member_not_found`) ghi người duyệt vào `added_by`, ghi
   event `auth.scim_group_updated`, rồi đồng bộ người đó. Gửi lại thành viên bằng token khác không đổi `added_by`.
2. **Liên kết SCIM có người tạo.** `auth_scim_users.created_by` ghi người tạo token đã tạo hay nhận về liên kết (đổi email
   qua SCIM vừa thu hồi mọi cách vào cũ, nên người đổi thành người tạo). Trước mỗi lần đồng bộ, (nút, role) MỚI mà
   người chịu trách nhiệm lần đồng bộ cấp được nhưng người tạo liên kết không cấp được → thu hồi mọi cách vào cũ
   (`UserService::lock_out_in`, như đổi email — D-20261125-scim-sync-report mục 7) trong một transaction riêng TRƯỚC
   lần cấp, rồi ghi người chịu trách nhiệm làm người tạo liên kết (thu hồi một lần). Đóng cả hai biến thể còn hở của
   mục 7: hệ thống danh tính không quản lý email, và thêm vào nhóm admin trước khi PATCH `User`.
3. **Dòng có từ trước** (NULL): không biết ai — không cấp được gì: thành viên cũ của nhóm chờ duyệt với luật đặt SAU
   (role đã cấp giữ nguyên), tài khoản cũ được nâng qua SCIM thì thu hồi một lần. Cùng cách với
   `auth_oidc_provider.configured_by` (D-20261121-oidc-link-covers).

Lý do: hai mục `[ ]` của P16. Rủi ro chấp nhận ở D-20261125-scim-grant-cover mục 5 (ai giữ token SCIM cũng đưa người
vào nhóm chưa có luật; admin đặt luật thì mọi thành viên nhận role) và D-20261125-scim-sync-report mục 7 (token quyền
thấp cắm sẵn tài khoản, token admin nâng nó mà kẻ tấn công vẫn giữ mật khẩu). Tính quyền người đưa vào lúc dùng (không
chụp lúc thêm) cùng nguyên tắc với token SCIM.

Đã cân nhắc:
- Gỡ role của thành viên chờ duyệt (khi người đưa vào bị hạ quyền): hạ một người tạo token làm mất role của mọi người họ
  từng đưa vào — loại; chờ duyệt chỉ chặn cấp mới.
- Token admin gửi lại thành viên thì tự duyệt: hệ thống danh tính gửi lại cả danh sách định kỳ, nên mọi người token
  quyền thấp đưa vào sẽ được duyệt ngầm — loại; duyệt là việc có chủ ý của người, có event.
- Thu hồi trong cùng transaction với lần cấp: `lock_out_in` cần `UserService`, lần cấp ở `RoleService` giữ chỗ hạn mức
  ngoài transaction; thu hồi trước là thứ tự an toàn (không có lúc vừa có role mới vừa giữ cửa vào cũ), lần cấp lỗi chỉ
  làm mất cửa vào cũ.
- Dòng cũ (NULL) coi như được phép: giữ đúng lỗ hổng cho dữ liệu đã cắm trước khi nâng cấp — loại.
