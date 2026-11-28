## D-20261128-grant-refusal-forbidden · Cấp role vượt quyền trả 403 `auth.role_beyond_you` ở mọi đường, như gỡ — 2026-11-28 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: bốn đường cấp role chọn role cụ thể — `PUT /api/v1/auth/users/{id}/memberships` (và tool MCP
`set_memberships`), `POST /api/v1/auth/invitations` (`create_invitation`), `POST /api/v1/auth/scim/group-mappings`,
role mặc định của nhà cung cấp đăng nhập (`POST` / `PATCH /api/v1/auth/oidc-providers…`, `create_oidc_provider`,
`update_oidc_provider`) — khi role vượt quyền người cấp trả **403 `auth.role_beyond_you`** thay vì 422
`validation_failed` với ô `more_than_you_have`. Ô lỗi `more_than_you_have` (kèm `role`, `role_id`, `org_id`) vẫn nằm
trong `errors` của lỗi 403 để máy biết role nào. Lỗi đầu vào khác (nút / role không có, ngoài phạm vi, trùng) vẫn 422 và
báo trước: còn lỗi đầu vào thì chỉ trả 422 với các lỗi đó (D-083), không kèm ô vượt quyền. Một chỗ quyết:
`role::refuse_grants` (`role/service/grants.rs`). Đây là thay đổi phá vỡ hợp đồng API trước 1.0
(D-20261125-api-break-before-1-0): ghi ở changelog mục Changed.

Lý do: P16 (iii) của bước 70 — cùng một luật "chỉ cấp cái mình có" (D-20261121-grant-scope-check) mà cấp vượt quyền trả
422, gỡ vượt quyền và `PUT /users/{id}/roles` trả 403. Vượt quyền là vấn đề quyền của người gọi, không phải đầu vào sai:
403 cho client (và agent) một cách nhận biết duy nhất, không phải dò mã ô trong `errors`.

Đã cân nhắc:
- Đổi gỡ / `PUT …/roles` sang 422: trái nghĩa HTTP (422 là đầu vào sai, sửa thân request là xong) và đổi nhiều đường hơn.
- Kiểm vượt quyền trước lỗi đầu vào: role không có thì không biết permission của nó; báo lỗi đầu vào trước đúng thứ tự
  sửa của người gọi.
- Mã mới `auth.role_not_grantable`: hai mã cho một luật — đúng cái bất nhất cần bỏ.
