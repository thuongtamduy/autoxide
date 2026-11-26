## D-20261126-api-key-self-scope · API key chỉ xem và thu hồi được chính nó; thu hồi key người khác cần bao trùm quyền chủ key — 2026-11-26 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: trong `ApiKeyService` (service layer, nên REST và tool MCP cùng chịu — luật kiến trúc 8), rẽ nhánh theo
loại credential (`Principal::credential()`), không theo "có `api_key_id` không" (token OAuth không có `api_key_id` mà
cũng không phải phiên đăng nhập):

| Thao tác | Phiên đăng nhập | API key | Token OAuth / loại khác |
| --- | --- | --- | --- |
| `list_own` (`GET /api/v1/auth/api-keys`, tool `auth.list_api_keys`) | mọi key của mình | chỉ chính nó | rỗng |
| `revoke` (`DELETE /api/v1/auth/api-keys/{id}`, tool `auth.revoke_api_key`) | key của mình | chỉ chính nó | không key nào |
| `rotate` | key của mình | 403 `auth.api_key_cannot_create_keys` | 403 như API key |

Id không được phép trả `404 auth.api_key_not_found` như key không có. `rotate` kiểm credential **trước** khi tra key, để
lỗi giống nhau dù id có tồn tại hay không.

`revoke_any` (`DELETE /api/v1/auth/all-api-keys/{id}`, cần `auth.api_keys.manage`): bỏ qua `require_covers` chỉ khi chính
chủ key gọi bằng phiên đăng nhập. Mọi trường hợp khác (key của người khác; key khác của chính chủ mà gọi bằng API key
hay token OAuth) đòi bên gọi bao trùm quyền của chủ key (D-079). Chủ key không phải nhân sự: không có gì để so,
`auth.api_keys.manage` là đủ (`require_covers` chỉ nhận nhân sự).

`GET /api/v1/auth/me/passkeys` đòi phiên đăng nhập như các thao tác passkey khác. Tạo, duyệt key vốn đã chỉ cho phiên.

Lý do: UT-2 (đợt dùng thử) — key quyền tối thiểu thu hồi được key khác của cùng chủ qua tool MCP (và REST, cùng service):
service chỉ kiểm "key thuộc cùng người dùng", trong khi mỗi API key là một danh tính có phạm vi riêng. Key lộ không được
tắt hay dò các key khác (kể cả key quyền cao hơn) của chủ.

Đã cân nhắc: cấm hẳn API key thu hồi key — mất cách cho một key lộ tự tắt nó; chỉ sửa tool MCP — REST cùng lỗi, và luật
8 đòi một chỗ kiểm ở service.
