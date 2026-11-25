## D-20261125-scim-sync-report · Đồng bộ SCIM: hạn mức theo trạng thái thật, đồng bộ hết mọi người và báo ai lỗi — 2026-11-25 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định:

1. **Hạn mức số nhân sự theo trạng thái thật:** `RoleService::with_member_limit` vẫn giữ chỗ TRƯỚC khi đổi khi người
   đó có thể sắp giữ role, nhưng sau khi đổi thì đối chiếu với việc người đó **còn giữ role nào thật** (không với ý
   định): giữ thì xác nhận chỗ, không giữ thì trả chỗ. Lý do: lần đồng bộ SCIM chỉ áp phần chênh mà người chịu trách
   nhiệm cấp được (D-20261125-scim-grant-cover), nên "muốn giữ" và "đang giữ" có thể khác — trước đây chỗ bị giữ cho
   người không có role, hay bị trả cho người còn role. Chi tiết:
   - `has_membership` (trước và sau khi đổi) đọc qua kết nối **ghi**: kết nối đọc có thể là read replica trễ hơn bản
     chính, đọc ở đó ngay sau commit thì vừa gán role vẫn thấy "chưa giữ" và trả chỗ — gói bị vượt hạn mức, ở mọi
     đường gán role.
   - Lần đồng bộ SCIM tính `will_hold` theo phần nó **sẽ áp được** (role muốn mà đã có hay cấp được; role SCIM đã cấp
     mà không gỡ được; membership gán tay), nên lượt bị bỏ qua hết không giữ chỗ.
   - Lượt **không đổi gì** không xác nhận cũng không trả chỗ: mã giữ chỗ `auth.member:<id>` dùng chung cho mọi lần
     đổi role của một người, lượt chạy đồng thời khác có thể đang dựa vào nó (lượt không đổi gì mà trả chỗ sẽ trả
     mất chỗ của lượt kia). Chỗ đã giữ mà không ai xác nhận tự hết hạn.
   - Còn mở: hai lần đổi role **thật** cho cùng một người chạy đồng thời vẫn có thể lệch (một lượt gỡ hết đọc trạng
     thái trước khi lượt kia commit) — có từ trước, cần khoá dòng người dùng quanh "giữ chỗ → đổi → xác nhận" hay
     job đối soát; mục `[ ]` dưới P16.
2. **Đồng bộ hết mọi người, báo ai lỗi:** `ScimService::sync_in` đồng bộ mọi người dù có người lỗi (vd vượt hạn mức)
   và trả `SyncReport` (ai xong, ai lỗi), ghi log `warn` cho từng người lỗi. `POST /auth/scim/group-mappings` trả 201
   (luật đã ghi) kèm `granted_user_ids` (người đang giữ role sau khi áp luật, qua luật này hay vì đã có) và `failed`
   (`user_id`, `code`, `message`); người lỗi nhận role ở lần đồng bộ thành công sau. Request SCIM và bỏ luật vẫn trả
   lỗi đầu tiên nếu có người lỗi (hệ thống danh tính gửi lại), nhưng chỉ sau khi đã thử hết mọi người. **Gửi lại**
   đúng request cũ thì thành viên nhóm không đổi thêm; vì vậy sau mỗi lần ghi nhóm, ngoài người có thành viên đổi, còn
   đồng bộ lại **thành viên còn chờ** của nhóm (thiếu một (nút, role) mà luật của nhóm cấp — mỗi luật một câu) — gửi
   lại sau khi đã có chỗ thì người đó nhận role. `granted_user_ids` chỉ gồm người thật sự giữ role ở nút sau khi đồng
   bộ (đọc qua kết nối ghi). Log của người lỗi ghi lỗi đầy đủ; ra ngoài lỗi nội bộ chỉ là "Internal error.".
3. **Khoá nhóm cả khi bỏ luật:** `delete_mapping` khoá dòng nhóm như `create_mapping` và `update_group`, để mọi lần
   đổi luật hay thành viên của một nhóm nối đuôi nhau (thứ tự khoá: nhóm trước).
4. **Test ép thứ tự:** `dat_luat_cho_khoa_nhom_cua_lan_ghi_thanh_vien` giữ một transaction đóng vai lần ghi thành
   viên (khoá nhóm, ghi thành viên, chưa commit) rồi đặt luật song song: đặt luật phải chờ. Bỏ khoá trong
   `create_mapping` thì test này đỏ chắc chắn trên Postgres (đã chạy thử; SQLite một bên ghi nên luôn xanh).

5. **Mọi thay đổi trên `User` đã liên kết cần bao trùm (PT-19):** PUT / PATCH `User` đổi bất kỳ thuộc tính nào
   (`userName`, `externalId`, tên, email, `active`) của tài khoản mà người tạo token không bao trùm quyền → 403
   (`require_creator_covers`); gửi lại hồ sơ không đổi gì thì không kiểm. `userName` không dùng để đăng nhập, nhưng
   nó và `externalId` là khoá hệ thống danh tính dùng để khớp người: đổi chúng trên tài khoản quyền cao thì lần đẩy
   sau bằng token của người quyền cao có thể ghi hồ sơ của người khác (kể cả email) vào tài khoản đó.

Đính chính D-20261125-scim-grant-cover (không sửa file đó):

- Mục 2, "sau cả hai không có thành viên của nhóm có luật mà không có role của luật": đúng khi lần đồng bộ sau khi
  đặt luật thành công. Lần đồng bộ cho một người có thể lỗi (vượt hạn mức, gỡ admin cuối cùng) — người đó ở lại trong
  nhóm có luật mà chưa có role, và lần đồng bộ thành công sau (của người có quyền cấp role đó) sẽ cấp. Nay việc này
  không còn im lặng: người lỗi có trong `failed` của response đặt luật và trong log. Token quyền thấp vẫn không cấp
  được role đó (lần đồng bộ của nó bỏ qua phần vượt quyền).
- Mục 2, "mọi lỗi 403 đến từ bước kiểm trước khi ghi": chỉ đúng với lỗi `auth.role_beyond_you`. Sau khi ghi vẫn có thể
  có 403 khác, vd vượt hạn mức số nhân sự của gói (`limit_exceeded`) ở lần đồng bộ.

Lý do: rà soát độc lập #515 chỉ ra hạn mức tính lệch khi đồng bộ áp một phần, và lỗi giữa chừng khi đặt luật để lại
thành viên chưa đồng bộ mà không ai biết (token quyền thấp có thể nhồi người đệm cho chạm hạn mức trước người cần cấp).

Đã cân nhắc:
- Đưa việc cấp role cho thành viên vào cùng transaction với việc ghi luật: hạn mức của gói giữ chỗ ngoài transaction
  (D-323, D-20261121-scim-groups mục 4) — phải đổi cách giữ hạn mức cho mọi đường gán role; để sau.
- Trả lỗi khi có người lỗi lúc đặt luật: luật đã commit nên câu trả lời lỗi gây hiểu nhầm là chưa có gì xảy ra — loại.
