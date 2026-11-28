## D-20261128-in-tenant-moves-root · `Principal::in_tenant` dời grant ở gốc khách cũ sang gốc khách mới — 2026-11-28 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: `Principal::in_tenant(t)` — khi không phải nhân sự nền tảng đang vào khách (`operator_from` là `None`) — dời
mọi grant ở nút gốc của khách hiện tại (và rào đặt ở gốc đó) sang nút gốc của `t`. `require_at_root` bỏ nhánh
`legacy_root` (grant ở gốc khách mặc định được coi là gốc của mọi khách): chỉ còn gốc của chính khách và gốc khách nhà
của nhân sự nền tảng.

Lý do: kiểm độc lập #614. `Principal::new` (và mọi `with_grants` của đường xác thực trước khi đổi khách) đặt grant ở gốc
khách mặc định; `in_tenant` đổi khách mà không dời grant, nên `require_at_root` phải nhận gốc khách mặc định cho mọi khách
— đúng chỉ khi không khách nào có nút nhánh mang đúng ID đó. Dời grant làm grant đúng chỗ, bỏ được giả định ấy; mọi chỗ
khác (phạm vi `tenant` không xét nút) không đổi hành vi. Nhân sự nền tảng (`operating`) giữ grant ở gốc khách nhà như
trước.

Đã cân nhắc: chặn `OrgTree::insert` nhận ID gốc của khách mặc định ở khách khác — giữ nguyên giả định ngầm và mọi chỗ
đọc grant vẫn thấy grant ở nút của khách khác.
