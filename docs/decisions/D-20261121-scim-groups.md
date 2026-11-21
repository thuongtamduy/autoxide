## D-20261121-scim-groups · Bước 66: nhóm SCIM → role ở nút qua luật ánh xạ; chỉ membership do SCIM tạo bị SCIM gỡ; đồng bộ từng người sau khi ghi nhóm — 2026-11-21 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định:

1. **Nhóm** (`Group`) lưu ở `auth_scim_groups` + `auth_scim_group_members`; `displayName` unique trong khách (không phân
   biệt hoa thường), `externalId` unique. Thành viên phải là `User` SCIM của khách (khác thì `invalidValue`). Nhóm tự
   nó không cấp quyền gì.
2. **Luật ánh xạ** `auth_scim_group_mappings`: nhóm X → role R ở nút N, khách tự đặt qua
   `GET/POST /api/v1/auth/scim/group-mappings`, `DELETE …/{id}` (permission `auth.scim.manage`). Luật gắn với **ID
   nhóm** (ổn định khi hệ thống danh tính đổi tên nhóm), không với tên. Người tạo luật phải gán được role đó ở nút
   đó (`grant_problems` như gán membership: nút trong phạm vi, có mọi permission của role, `admin` chỉ ở gốc) — luật
   là cách gán role hàng loạt, không được dùng để tự nâng quyền.
3. **Chỉ membership do SCIM tạo bị SCIM gỡ:** `auth_scim_grants` ghi ID dòng `auth_user_role` mà SCIM đã thêm
   (`user_role_id`) kèm (người, nút, role). SCIM chỉ gỡ đúng dòng đó. (Nút, role) người đó đã có bằng tay thì SCIM
   không thêm, không ghi nhận; người quản trị gán tay lại (dòng mới) thì dấu cũ hết hiệu lực — rời nhóm không lấy mất
   membership gán tay. Người quản trị gỡ tay role SCIM cấp thì lần đồng bộ sau cấp lại (hệ thống danh tính là nguồn
   của role theo nhóm). Hai nhóm cùng cấp một role: rời một nhóm vẫn giữ role.
4. **Đồng bộ từng người sau khi ghi nhóm:** thành viên nhóm ghi trong một transaction (khoá dòng nhóm, D-181); sau
   commit, membership của từng người có thay đổi được đồng bộ trong transaction riêng của người đó
   (`RoleService::sync_scim_grants`: khoá dòng người dùng, so với `auth_scim_grants`, kiểm admin cuối cùng, event
   `auth.memberships_changed`). Lý do: hạn mức số nhân sự của gói giữ chỗ **ngoài** transaction (D-323, như
   `set_memberships`). Đồng bộ lỗi (vượt hạn mức, gỡ admin cuối) thì request SCIM trả lỗi; hệ thống danh tính gửi
   lại, lần gửi lại không đổi nhóm thêm mà chỉ đồng bộ lại — kết quả cuối giống nhau. Đổi luật ánh xạ cũng đồng bộ
   mọi thành viên của nhóm ngay.
5. **Nhóm chưa có luật** vào hộp việc (nguồn `auth.scim_unmapped_groups`, permission `auth.scim.manage`), tính từ
   dữ liệu: đặt luật thì việc tự hết. `GET /api/v1/auth/scim/groups` cho trang quản trị: số thành viên, số luật.
6. Xoá nhóm (SCIM `DELETE`) xoá luật của nó và gỡ role nó đã cấp; xoá role xoá luật trỏ tới role đó; xoá tài khoản
   xoá thành viên nhóm và dấu "do SCIM cấp" cùng transaction (không foreign key, như `auth_identity`).

Lý do: đặc tả bước 66 ("nhóm X → role R ở nút N; thành viên nhóm đổi → membership đổi"); tiêu chí "dễ bảo trì" — một
đường gán membership (cùng bảng `auth_user_role`, cùng event, cùng hạn mức) cho tay, lời mời và SCIM.

Đã cân nhắc:
- Luật theo tên nhóm: đặt trước khi nhóm có được, nhưng đổi tên ở hệ thống danh tính là mất luật âm thầm — loại.
- Đồng bộ trong cùng transaction với ghi nhóm: phải đổi cách giữ hạn mức của gói cho mọi đường gán role — để sau.
- Gỡ mọi membership trùng (nút, role) khi rời nhóm: lấy mất quyền gán tay — loại.
