## D-20261128-ban-lift-scope · Bỏ lệnh cấm và tác động lên tài khoản người khác so cả phạm vi — 2026-11-28 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định (P16 của bước 70, module `auth`):

1. Bỏ một lệnh cấm — gỡ role có cấm (thay role, membership, xoá role, SCIM) hay sửa role bỏ cấm — đòi người làm có
   permission bị cấm ở **độ sâu `tenant`, không bị rào** (`lifts_ban` = `actor_covers(gốc, Scope::Tenant)`), không chỉ
   có TÊN permission ở bất kỳ nút nào. Cấm thắng mọi role ở mọi nút, nên bỏ cấm mở lại permission ở cả khách.
2. `RoleService::require_covers` (xoá / khoá / đổi email / nhận về tài khoản, thu hồi API key người khác, token SCIM)
   so từng grant (nút, độ sâu) của người bị tác động bằng `covers_all` — cùng `actor_covers` của luật gán role
   (D-20261121-grant-scope-check), thay vì so tên. Người bị tác động có toàn quyền: người làm phải toàn quyền, không rào.
   Đây là mục (i) của "các chỗ bất nhất của luật cấp quyền". Đổi hành vi: API key gắn nhánh không còn khoá, đổi email
   hay thu hồi key của nhân sự giữ permission không `scoped` (grant của nó có độ sâu `tenant`, mà key bị rào thì không
   bao được độ sâu `tenant`), kể cả nhân sự cùng nhánh — việc đó cần người làm ở gốc.

Còn mở, không đổi ở đây: (iii) tạo vượt quyền 422 / xoá vượt quyền 403 — đổi mã lỗi là đổi hợp đồng API, cần quyết định
riêng trước 1.0 (D-20261125-api-break-before-1-0); (iv) thu hẹp permission của role người khác đang giữ không qua luật
"gỡ được"; khách có guardrail: role cấm một permission nằm ngoài rào thì không ai trong khách gỡ được (đã kiểm chứng
bằng đọc code: admin trong rào không còn permission đó nên `lifts_ban` sai) — sửa cần kernel cho biết lý do cấm của
từng permission (`outside_guardrail`) hay đưa rào vào `grantable`.

Lý do: rà soát P16 — so theo tên cho người có quyền ở một nhánh gỡ cấm của cả khách, hay khoá / đổi email tài khoản có
quyền rộng hơn mình.

Đã cân nhắc: cho bỏ cấm khi người làm có permission ở nút của membership bị gỡ — cấm không theo nút, nên sai nghĩa.
