## D-20261121-export-entry · Xuất dữ liệu và tạo sandbox của khách khác cần vào khách đó — 2026-11-21 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Căn cứ: Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15 (hiệu lực 01/01/2026), NĐ 356/2025/NĐ-CP — xử lý dữ liệu cá nhân có
căn cứ và để lại vết (như D-313; cần đối chiếu điều khoản cụ thể với văn bản gốc / người có chuyên môn pháp lý).

Quyết định:

1. `GET|POST /kernel/tenants/{id}/exports…` và tool MCP `export_tenant` / `get_tenant_export`: nhân sự nền tảng
   (`kernel.tenants.operate`) chỉ xuất được khách KHÁC khi đã vào khách đó (header `Autoxide-Tenant`, với module `auth`
   là lần cấp quyền vào hỗ trợ còn hạn — căn cứ `support`, hay `legal_request` kèm số văn bản đã ghi ở lần cấp, D-313).
   Gọi từ khách nền tảng với `{id}` của khách khác → 403 `kernel.export_needs_tenant_access`. Bỏ lối tắt "nhân sự nền
   tảng xuất mọi khách". Quản trị của chính khách (`kernel.data.export`) không đổi; khách nền tảng vẫn tự xuất được.
2. Mỗi lượt xin ghi `kernel.tenant_export_requested`, mỗi lần tải / tạo link / mở link ghi
   `kernel.tenant_export_accessed` (`via`) vào outbox của CHÍNH khách bị xuất, người làm kèm đường vào (`actor_access`
   = `support` và ID lần cấp) — transaction riêng sau khi dòng ở database điều khiển đã commit (database có thể khác).
3. Cùng luật cho `POST /kernel/tenants/{id}/sandboxes` (sandbox sao cấu hình và sản phẩm của khách): khác khách đang ở →
   403 `tenancy.sandbox_needs_tenant_access` với nhân sự nền tảng. Xem danh sách, gia hạn, xoá sandbox vẫn làm từ khách
   nền tảng (không chạm dữ liệu của khách); đẩy sang khách thật vốn đã đòi ở trong khách.

Lý do: D-313 đặt luật "nhân sự nền tảng chỉ vào dữ liệu khách bằng lần cấp có thời hạn" cho request vào khách, nhưng hai
endpoint nhận khách qua đường dẫn vẫn mở cho `kernel.tenants.operate` / `kernel.tenants.manage` mà không qua lần cấp,
không để vết ở khách.

Đã cân nhắc: tra lần cấp ngay trong `exports.rs` (kernel không biết bảng của `auth`; đường vào đã do
`Authenticator::enter` quyết ở mỗi request, dùng lại là đủ); giữ lối tắt nhưng chỉ ghi audit (vẫn trái D-313); chặn cả
danh sách / gia hạn / xoá sandbox (việc vận hành, không lộ dữ liệu).
