## D-20261125-domain-claims · Nhiều khách cùng chờ một tên miền, khách xác minh trước được; unique index chỉ trên dòng `verified` — 2026-11-25 — agent quyết

Chủ đề: Đa khách, tổ chức, gói

Quyết định: migration mới `m0040_domain_shared_claims` của kernel bỏ unique index `kernel_tenant_domains_domain_unique`,
thay bằng hai index: unique `(domain, tenant_id)` (mỗi khách tối đa một dòng cho một tên miền) và unique một phần
`(domain) WHERE status = 'verified'` (mỗi tên miền tối đa một khách đã xác minh; cú pháp chạy trên cả SQLite và
PostgreSQL). `TenantDomains::add` chỉ từ chối (`tenant_domain_taken`) khi khách khác đã **xác minh**; khách khác đang chờ
không chặn. Lúc xác minh, nếu khách khác đã xác minh trước (unique index báo trùng) thì lượt chờ của khách này thành
`failed` với `check_error` "Another tenant verified this domain first." Dòng `failed` / `revoked` của một khách chỉ khách
đó nhận lại; không còn chuyển dòng của khách này sang khách khác. `down` từ chối khi một tên miền có nhiều dòng (không xoá
lượt chờ của khách). Sửa P-4 của `docs/security/review-2026-11.md`.

Lý do: trước đây một khách khai tên miền của người khác mà không xác minh thì chặn chủ thật 7 ngày, lặp lại được mãi.
Cho nhiều khách cùng chờ giải quyết trọn mà không cần ai can thiệp: chỉ người điều khiển DNS của tên miền mới xác minh
được, nên người đó luôn thắng. Unique index trên dòng `verified` bảo đảm hai lần xác minh đồng thời không cùng thắng.

Đã cân nhắc: endpoint cho nhân sự nền tảng thu hồi lượt chờ của khách khác — cần người trực và chủ tên miền phải liên hệ,
kẻ gian khai lại ngay sau đó; rút ngắn 7 ngày — chỉ giảm, không hết chặn, và làm khó khách thật cần thời gian sửa DNS.
