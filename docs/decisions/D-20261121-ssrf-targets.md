## D-20261121-ssrf-targets · Địa chỉ máy chủ do khách chọn: kiểm lúc lưu, lọc IP lúc kết nối, lỗi chung — 2026-11-21 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định:

1. Kernel gom chống SSRF ở `autoxide_kernel::net` (tiếp D-149): `check_target_url(raw, schemes, allow_private)` kiểm lúc
   LƯU (scheme trong danh sách, có host, host không bị biết trước là nội bộ — IP không public, `localhost`) và trả lỗi ô
   `value` (`invalid` / `private_address`); `resolve_target(host, port, allow_private)` phân giải NGAY TRƯỚC khi kết nối,
   chỉ giữ IP public, và bên gọi kết nối vào đúng IP trả về (chống DNS rebinding). `PublicOnly` (reqwest) dùng lại hàm
   này. Giao thức không qua reqwest dùng hai hàm trên.
2. Mọi cài đặt của khách chứa URL / host phải `.checked(…)`. `mail.connection.smtp.url`: chỉ `smtp://` / `smtps://`,
   kiểm lúc lưu; lúc gửi và lúc thử kết nối (`POST /kernel/settings/{name}/test`), module `mail` tự phân giải host, lọc
   IP, rồi dựng kết nối lettre tới IP đó (TLS vẫn kiểm chứng chỉ theo tên miền trong URL); kết nối dùng lại tối đa 10
   phút rồi phân giải lại.
3. Mạng riêng chỉ được dùng khi người vận hành bật `AUTOXIDE_MAIL_ALLOW_PRIVATE_NETWORKS` (mặc định `false`, như
   `AUTOXIDE_WEBHOOKS_ALLOW_PRIVATE_NETWORKS`). `AUTOXIDE_MAIL_SMTP_URL` của người vận hành không bị giới hạn.
4. Lỗi trả cho khách (thử kết nối, `last_error`) là câu chung: không được phép / không kết nối được / mã trạng thái SMTP
   máy chủ trả — không kèm lỗi mạng chi tiết hay câu trả lời của máy chủ. Lỗi đầy đủ chỉ ở log và job queue.

Lý do: URL SMTP riêng của khách được lưu không kiểm rồi dùng để mở kết nối TCP từ máy chủ, kèm lỗi chi tiết trả về; cần
cùng cách chặn như webhook và OIDC.

Đã cân nhắc: chỉ kiểm lúc lưu (không chặn tên miền đổi IP sau đó); để lettre tự phân giải (không lọc được IP); một cờ
chung cho mọi module (webhook đã có cờ riêng; giữ cách đặt tên theo module).
