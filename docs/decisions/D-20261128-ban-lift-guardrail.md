## D-20261128-ban-lift-guardrail · Bỏ cấm một permission ngoài guardrail không cần có permission đó — 2026-11-28 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: kernel thêm `Principal::outside_guardrail(name)` — permission bị bỏ với lý do `outside_guardrail` (không
phải `denied_by_role`). `lifts_ban` của `auth` trả đúng khi permission ngoài rào của khách, hoặc người làm có nó ở độ sâu
`tenant` không rào như trước.

Lý do: P16 của bước 70 (đã kiểm chứng bằng đọc code ở D-20261128-ban-lift-scope): role cấm một permission nằm ngoài rào
thì không ai trong khách gỡ được role ấy, vì quản trị viên trong rào cũng không có permission đó. Rào áp cho mọi người
làm việc trong khách (cả người vào bằng uỷ quyền), nên bỏ cấm một permission ngoài rào không mở thêm quyền cho ai. Khi
người làm vừa bị role cấm vừa bị rào, kernel giữ lý do `denied_by_role` (người trong khách sửa được) và bỏ cấm vẫn bị chặn
— bảo thủ.

Đã cân nhắc: đưa rào vào `grantable` của `auth` (đọc rào trong `RoleService`): thêm phụ thuộc vào `GuardrailService` và
một lần đọc; lý do đã có sẵn trên `Principal`.
