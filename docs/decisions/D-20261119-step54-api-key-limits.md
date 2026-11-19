## D-20261119-step54-api-key-limits · Bước 54: API key có hạn bắt buộc và dải IP — 2026-11-19 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: Mọi API key đều có hạn. Trần là `AUTOXIDE_AUTH_API_KEY_MAX_DAYS` (mặc định 365, từ 1 tới 3650 ngày).
Tạo key không nêu `expires_at` thì key nhận đúng hạn dài nhất đó, không trả lỗi. Nêu xa hơn trần thì 422 với ô
`expires_at` mã `too_far`. Migration `m0038_api_key_networks` đặt hạn 365 ngày, tính từ lúc nâng cấp, cho mọi key cũ
chưa có hạn. Key có thêm `allowed_ips`: tối đa 50 địa chỉ hay dải CIDR, lưu dạng chuẩn
(`203.0.113.7/32`). Kernel kiểm địa chỉ client của request ở **một chỗ**, ngay sau bước xác thực
(`data/tenancy/request.rs`, `Principal::admits`), nên áp cho mọi endpoint REST và MCP. Ngoài dải thì trả 403
mã `network_not_allowed`. Không biết địa chỉ client thì cũng từ chối, vì không đoán. Địa chỉ client là địa chỉ
đã giải qua proxy tin cậy (`ClientIp`, D-152).

Lý do: Đặc tả 54 P4 (như token chi tiết của GitHub). Không trả 422 khi thiếu `expires_at`, vì client đang
dùng `/api/v1` sẽ vỡ (D-059); cho trần mặc định giữ nguyên hình dạng request mà vẫn bảo đảm không key nào sống
mãi. Mã lỗi riêng `network_not_allowed` thay cho `permission_denied` + `reason = ip_not_allowed` như đặc tả
gợi ý: câu thông báo của `permission_denied` nói về permission, còn ở đây người gọi cần biết phải đổi mạng hay
sửa danh sách địa chỉ của key. Kiểm ở chỗ xác thực chung thay vì trong từng handler, để không endpoint nào
quên.

Đã cân nhắc: (1) Kiểm IP trong bộ xác thực của `auth`. Bị loại vì trait `Authenticator` không nhận địa chỉ
client: phải đổi trait của kernel, mọi bộ xác thực phải sửa. (2) Bắt buộc gửi `expires_at`. Bị loại vì là thay đổi
phá vỡ.
