## D-20261119-step54-key-approval · Bước 54: quản trị duyệt API key — 2026-11-19 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: Bật / tắt bằng cấu hình theo khách `auth.api_keys.require_approval` (`true` / `false`, mặc định tắt),
đặt qua `PUT /api/v1/kernel/settings/{name}` sẵn có. Khi bật, key mới có `status = pending` (migration
`m0040_api_key_approval`) và mọi request dùng nó nhận 401 `auth.api_key_pending`, tới khi người có permission mới
`auth.api_keys.approve` (`risk = sensitive_data`) gọi `POST /all-api-keys/{id}/approve`. Người giữ quyền duyệt tạo
key thì key `active` ngay. Key xoay vòng giữ trạng thái của key nó thay. Từ chối là thu hồi bằng
`DELETE /all-api-keys/{id}`. Duyệt chỉ làm được bằng phiên đăng nhập, không bằng API key, và không có tool MCP.
Người chỉ có quyền duyệt xem được `GET /all-api-keys`, nhưng chỉ thấy key `pending`.

Lý do: Đặc tả 54 P4 (như GitHub cho chủ tổ chức duyệt token). Dùng cấu hình theo khách của kernel thay cho bảng
riêng: đã có cache, API, audit. Chặn duyệt bằng API key vì một key lộ không được tự mở đường cho key khác. Key xoay
vòng không cần duyệt lại vì cấu hình y hệt key đã duyệt; bắt duyệt lại sẽ làm key cũ hết hạn trong lúc key mới
còn chờ, tức là gián đoạn.

Đã cân nhắc: (1) Duyệt qua yêu cầu duyệt chung của bước 51 (`kernel_approval_requests`). Hoãn lại: luồng đó dành
cho lệnh chờ duyệt có payload; key chỉ cần đổi một cột. Có thể nối vào hộp việc sau. (2) Cấm người tạo tự duyệt
key của mình. Không cần: người giữ quyền duyệt thì key của họ vốn chạy ngay.
