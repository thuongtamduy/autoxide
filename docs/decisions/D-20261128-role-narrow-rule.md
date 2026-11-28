## D-20261128-role-narrow-rule · Sửa role mà bỏ, thu hẹp hay cấm thêm permission phải qua luật "gỡ được" — 2026-11-28 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: `grants::rules_beyond` (sửa role qua REST / MCP) ngoài thêm / nới / bỏ cấm còn chặn: BỎ hay THU HẸP độ sâu
một permission mà người sửa không bao được grant cũ ở nút gốc; THÊM cấm một permission mà người sửa không có ở độ sâu
`tenant`, không rào (cùng luật bỏ cấm, D-20261128-ban-lift-scope). Mã lỗi vẫn `auth.permission_beyond_you` (403), câu
và tài liệu OpenAPI / MCP nêu thêm các trường hợp mới.

Lý do: P16 (iv) của bước 70. Sửa role có hiệu lực ngay với mọi người đang giữ role ở mọi nút, nên thu hẹp role là gỡ
permission khỏi họ — trước đây người có `auth.roles.manage` hạ quyền người khác được bằng cách này dù không gỡ được role
của họ. Luật mới đối xứng với khi thêm (PT-9, D-20261126-no-self-grant). Quản trị viên toàn quyền không bị ảnh hưởng.

Đã cân nhắc: so ở nút người giữ role đang giữ (như gỡ membership) — role được giữ ở nhiều nút và người giữ đổi theo thời
gian; so ở gốc như khi thêm là luật duy nhất đúng cho mọi người giữ.
