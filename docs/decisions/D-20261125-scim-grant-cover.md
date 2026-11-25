## D-20261125-scim-grant-cover · Mọi thay đổi quyền qua SCIM và luật ánh xạ nhóm phải nằm trong quyền của người chịu trách nhiệm, kiểm ở một chỗ — 2026-11-25 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định:

1. **Nguyên tắc** (mở rộng D-20261121-scim-claim-covers): token SCIM không cấp, giữ hay gỡ thứ gì vượt quá quyền của
   người tạo token; người đổi luật ánh xạ nhóm không đặt **hay bỏ** luật vượt quá quyền mình. "Cấp được" nghĩa là
   `grant_problems` rỗng (nút trong phạm vi, có mọi permission của role với độ rộng đủ, `admin` chỉ ở gốc và chỉ do
   người toàn quyền không bị rào — D-20261121-grant-scope-check). Quyền của người tạo token tính **lúc dùng token**
   (`ScimService::authority`): bị hạ quyền thì token hạ theo; bị khoá, bị xoá hay không còn là nhân sự thì token
   không cấp, giữ hay gỡ được gì (vẫn tạo / đổi tên / khoá được tài khoản không có quyền gì).
2. **Chỗ kiểm chung:** `role::grantable` / `role::require_grantable` (`role/service/grants.rs`) → 403
   `auth.role_beyond_you`. Hai lớp dùng chung hàm đó:
   - **Kiểm trước khi ghi** (trả 403, không ghi gì — kể cả các op hợp lệ khác trong cùng PATCH): ghi thành viên nhóm
     (POST / PUT / PATCH `Groups`: có người vào → phải cấp được mọi luật của nhóm; có người ra → phải gỡ được), xoá
     nhóm (gỡ được mọi luật của nhóm), bỏ luật ánh xạ (gỡ được role đó ở nút đó), xoá / khoá / đổi email `User`
     (`require_covers`).
   - **Lần đồng bộ** `RoleService::sync_scim_grants` (mọi thay đổi membership do SCIM đều qua đây) nhận `authority`
     và chỉ thêm / gỡ phần chênh mà `authority` cấp được; phần còn lại giữ nguyên, ghi log `warn`. Không trả lỗi ở
     đây: phần vượt quyền chỉ còn có thể đến từ trạng thái người khác đặt (luật vừa đặt đồng thời, lần đồng bộ trước
     lỗi) và để cho lần đồng bộ của người có quyền đó — trả 403 thì request SCIM báo lỗi dù thay đổi của chính nó đã
     ghi.
   - **Đồng thời:** `create_mapping` khoá dòng nhóm như `update_group`, nên đặt luật và ghi thành viên nối đuôi nhau
     (Postgres; SQLite một bên ghi): lần ghi thành viên sau luôn thấy luật đã commit và kiểm theo nó; lần đặt luật
     sau luôn thấy thành viên đã commit và cấp cho họ (xem mục 5). Sau cả hai không có thành viên của nhóm có luật mà
     không có role của luật ("thành viên chờ").
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
5. **Nhóm chưa có luật** (rủi ro chấp nhận, mức thấp vì cần admin hành động): khi nhóm chưa có luật, thêm thành
   viên không cấp gì nên không bị kiểm — ai giữ token SCIM cũng đưa người vào được (hay đổi tên nhóm cho giống nhóm
   của hệ thống danh tính). Admin đặt luật thì mọi thành viên nhận role. Giảm nhẹ: response `POST
   …/group-mappings` liệt kê người nhận role (`granted_user_ids`), hướng dẫn SCIM dặn xem thành viên trước khi ánh
   xạ tới role mạnh. Cách chặt hơn (ghi người đưa vào ở thành viên nhóm, chỉ cấp cho thành viên mà người đưa vào cấp
   được role) cần migration — mục `[ ]` dưới P16.
6. **Vận hành:** token mạnh đúng bằng người tạo lúc dùng, nên người tạo nghỉ hay bị hạ quyền thì gỡ người khỏi nhóm
   và khoá người nghỉ việc qua SCIM bị 403 ở hệ thống danh tính. `GET /auth/scim/tokens` có `creator_status`
   (`active` / `disabled` / `deleted` / `missing_permission`) để thấy token cần xoay; hướng dẫn SCIM (EN + VI) và
   changelog dặn xoay token khi người tạo nghỉ hay bị hạ quyền.
7. Lỗi ở endpoint SCIM theo lược đồ lỗi SCIM, status 403, **không** có `scimType` (RFC 7644 bảng 9 không có loại cho
   403 do quyền); ở REST quản trị là Problem 403 `auth.role_beyond_you`.

Ghi nhận cho chủ dự án (không đổi trong lần này): `PUT /api/v1/auth/users/{id}/roles` (permission `auth.roles.manage`,
đường cũ trước bước 34) cho người giữ permission đó tự gán mọi role, kể cả `admin` — mô tả permission nói là cố ý —
trong khi `PUT …/memberships`, lời mời, luật nhóm SCIM và nay mọi đường SCIM theo "chỉ cấp cái mình có". Hai đường bất
nhất; việc chọn giữ hay đưa đường cũ về cùng luật để ở `docs/ROADMAP.md` (P16) cho chủ dự án quyết.

Lý do: rà soát bảo mật độc lập (2026-11) cho thấy người chỉ có `auth.scim.manage` dùng token SCIM của mình đưa tài khoản
bất kỳ vào nhóm mà chủ khách đã ánh xạ tới `admin` (leo lên admin), xoá tài khoản quyền cao hơn, và bỏ luật `admin`
của chủ khách — trong khi khoá tài khoản và đặt luật đã bị chặn đúng. Vá từng endpoint để sót đường khác; lần đồng
bộ (mọi thay đổi role do SCIM đều qua đó) không bao giờ cấp vượt quyền kể cả ở đường chưa nghĩ tới, còn kiểm trước khi
ghi giữ cho nhóm **đã có luật** không chứa thành viên chờ (nhóm chưa có luật: mục 5).

Đã cân nhắc:
- Chỉ kiểm ở `sync_scim_grants`: thành viên nhóm vẫn được ghi rồi mới báo lỗi, lần đồng bộ sau do token của admin hay
  do đổi luật sẽ cấp quyền cho người kẻ khác đã đưa vào — loại.
- Lần đồng bộ trả 403 khi phần chênh vượt quyền: với luật đặt đồng thời, request SCIM báo 403 dù thành viên đã ghi
  (test đồng thời bắt được trên cả hai database) — đổi thành "bỏ qua phần vượt quyền, ghi log".
- Chỉ kiểm sớm ở ghi nhóm: sót đường đổi luật và trạng thái cũ còn trong bảng — loại; giữ cả hai.
- Tính quyền theo lúc tạo token (lưu bản chụp quyền): người bị hạ quyền vẫn giữ quyền cũ qua token — loại.
- Thêm `scimType` riêng: chuẩn không có, client SCIM (Entra ID, Okta) chỉ hiểu các loại của bảng 9 — loại.
