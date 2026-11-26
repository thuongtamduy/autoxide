## D-20261126-api-key-self-scope · API key chỉ xem và thu hồi được chính nó; thu hồi key người khác cần bao trùm quyền chủ key — 2026-11-26 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: trong `ApiKeyService` (service layer, nên REST và tool MCP cùng chịu — luật kiến trúc 8):

- `list_own` (`GET /api/v1/auth/api-keys`, tool `auth.list_api_keys`): gọi bằng API key thì chỉ trả chính key đó.
- `revoke` (`DELETE /api/v1/auth/api-keys/{id}`, tool `auth.revoke_api_key`): gọi bằng API key thì chỉ thu hồi được chính
  key đó; id khác trả `404 auth.api_key_not_found` như key không có (không lộ key nào tồn tại).
- `revoke_any` (`DELETE /api/v1/auth/all-api-keys/{id}`, cần `auth.api_keys.manage`): key của người khác cần thêm
  `require_covers` với chủ key (D-079), như khoá hay xoá tài khoản.

Phiên đăng nhập của chủ vẫn quản lý mọi key của mình. Tạo, xoay, duyệt key vốn đã cấm API key
(`auth.api_key_cannot_create_keys`, `auth.api_key_cannot_approve_keys`).

Lý do: UT-2 (đợt dùng thử) — key quyền tối thiểu thu hồi được key khác của cùng chủ qua tool MCP (và REST, cùng service):
service chỉ kiểm "key thuộc cùng người dùng", trong khi mỗi API key là một danh tính có phạm vi riêng. Key lộ không được
tắt hay dò các key khác (kể cả key quyền cao hơn) của chủ.

Đã cân nhắc: cấm hẳn API key thu hồi key — mất cách cho một key lộ tự tắt nó; chỉ sửa tool MCP — REST cùng lỗi, và luật
8 đòi một chỗ kiểm ở service.
