## D-20261125-scim-grant-cover · Mọi thay đổi quyền qua SCIM và luật ánh xạ nhóm phải nằm trong quyền của người chịu trách nhiệm, kiểm ở một chỗ — 2026-11-25 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định:

1. **Nguyên tắc** (mở rộng D-20261121-scim-claim-covers): token SCIM không cấp, giữ hay gỡ thứ gì vượt quá quyền của
   người tạo token; người đổi luật ánh xạ nhóm không đặt **hay bỏ** luật vượt quá quyền mình. "Cấp được" nghĩa là
   `grant_problems` rỗng (nút trong phạm vi, có mọi permission của role với độ rộng đủ, `admin` chỉ ở gốc và chỉ do
   người toàn quyền không bị rào — D-20261121-grant-scope-check). Quyền của người tạo token tính **lúc dùng token**
   (`ScimService::authority`): bị hạ quyền thì token hạ theo; bị khoá, bị xoá hay không còn là nhân sự thì token
   không cấp, giữ hay gỡ được gì (vẫn tạo / đổi tên / khoá được tài khoản không có quyền gì).
2. **Chỗ kiểm chung:** `role::require_grantable(connection, tenant, authority, give, take)` → 403
   `auth.role_beyond_you`. Mọi thay đổi membership do SCIM đi qua `RoleService::sync_scim_grants`, nay nhận
   `authority` và kiểm mọi (nút, role) thêm / gỡ trong transaction trước khi ghi — kể cả khi thành viên nhóm hay luật
   đã được ghi từ trước (lần đồng bộ trước lỗi), token quyền thấp không biến chúng thành quyền. Thêm hai chỗ kiểm sớm
   dùng cùng hàm, để không ghi "ý định" mà lần đồng bộ sau (của người khác) biến thành quyền: ghi thành viên nhóm
   (PATCH / PUT / POST `Groups`: có người vào → kiểm cấp mọi luật của nhóm; có người ra → kiểm gỡ) và xoá nhóm (kiểm
   gỡ mọi luật của nhóm); bỏ luật ánh xạ (`DELETE /auth/scim/group-mappings/{id}`) kiểm gỡ role đó ở nút đó. Gỡ thì
   nút / role không còn trong khách được bỏ qua (không còn quyền gì để gỡ).
3. **Phạm vi đã rà** (module `auth`): `Users` POST (tạo mới: không cấp quyền; nhận về tài khoản có sẵn: đã kiểm bao
   trùm), PUT / PATCH (đổi email, `active`: đã kiểm bao trùm; tên: không đổi quyền, không kiểm), **DELETE (thiếu —
   nay kiểm bao trùm như khoá)**; `Groups` POST / PUT / PATCH thêm và bớt thành viên (**thiếu — nay kiểm**), đổi tên
   (luật gắn ID nhóm, không đổi quyền ai), DELETE (**thiếu — nay kiểm**); luật ánh xạ POST (đã kiểm, 422), **DELETE
   (thiếu — nay kiểm, 403)**, không có PUT; token tạo (cần phiên đăng nhập, P-1) và thu hồi (không đổi quyền ai — một
   người có `auth.scim.manage` thu hồi được token của người khác, chấp nhận: chỉ làm ngừng đồng bộ, có audit);
   Bulk không hỗ trợ (`ServiceProviderConfig`).
4. **Phiên và audit:** principal tính lại từ database mỗi request (D-20261121-access-token-revocation), nên role mất
   qua SCIM có hiệu lực ngay như đường gán tay — không cần thu hồi phiên riêng; khoá qua SCIM vẫn thu hồi phiên, refresh
   token, API key (D-20261121-scim). Mọi thay đổi vẫn ghi event (`auth.memberships_changed`, `auth.scim_group_*`,
   `auth.user_*`, actor `scim`); lần bị từ chối không đổi gì, chỉ ghi log `warn`.
5. Lỗi ở endpoint SCIM theo lược đồ lỗi SCIM, status 403, **không** có `scimType` (RFC 7644 bảng 9 không có loại cho
   403 do quyền); ở REST quản trị là Problem 403 `auth.role_beyond_you`.

Ghi nhận cho chủ dự án (không đổi trong lần này): `PUT /api/v1/auth/users/{id}/roles` (permission `auth.roles.manage`,
đường cũ trước bước 34) cho người giữ permission đó tự gán mọi role, kể cả `admin` — mô tả permission nói là cố ý —
trong khi `PUT …/memberships`, lời mời, luật nhóm SCIM và nay mọi đường SCIM theo "chỉ cấp cái mình có". Hai đường bất
nhất; việc chọn giữ hay đưa đường cũ về cùng luật để ở `docs/ROADMAP.md` (P16) cho chủ dự án quyết.

Lý do: rà soát bảo mật độc lập (2026-11) cho thấy người chỉ có `auth.scim.manage` dùng token SCIM của mình đưa tài khoản
bất kỳ vào nhóm mà chủ khách đã ánh xạ tới `admin` (leo lên admin), xoá tài khoản quyền cao hơn, và bỏ luật `admin`
của chủ khách — trong khi khoá tài khoản và đặt luật đã bị chặn đúng. Vá từng endpoint để sót đường khác; kiểm ở đường
đồng bộ (mọi thay đổi role do SCIM đều qua đó) chặn cả những đường chưa nghĩ tới, còn kiểm sớm giữ dữ liệu nhóm không
chứa "quyền chờ".

Đã cân nhắc:
- Chỉ kiểm ở `sync_scim_grants`: thành viên nhóm vẫn được ghi rồi mới báo lỗi, lần đồng bộ sau do token của admin hay
  do đổi luật sẽ cấp quyền cho người kẻ khác đã đưa vào — loại.
- Chỉ kiểm sớm ở ghi nhóm: sót đường đổi luật và trạng thái cũ còn trong bảng — loại; giữ cả hai.
- Tính quyền theo lúc tạo token (lưu bản chụp quyền): người bị hạ quyền vẫn giữ quyền cũ qua token — loại.
- Thêm `scimType` riêng: chuẩn không có, client SCIM (Entra ID, Okta) chỉ hiểu các loại của bảng 9 — loại.
