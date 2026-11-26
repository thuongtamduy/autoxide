## D-20261126-tenant-rate-limit · Giới hạn request theo khách: tầng thứ ba, mức theo gói — 2026-11-26 — agent quyết

Chủ đề: HTTP, REST, OpenAPI

Quyết định: Thêm tầng giới hạn request thứ ba, theo KHÁCH, sau tầng theo địa chỉ client và tầng theo API key. Mức:
`kernel.api_rate_per_minute` của gói khách khi gói nêu (kể cả "không giới hạn"; burst bằng chính mức đó), không thì
`AUTOXIDE_HTTP_RATE_LIMIT_TENANT_PER_MINUTE` / `_BURST` (mặc định 0 = tắt → hành vi cũ). Áp cho route REST của mọi
module (lớp bọc router của module, cùng chỗ đếm `kernel.api_requests`, dùng kết quả giải khách đã giữ trong request) và
mọi lời gọi tool MCP. Request chưa giải được khách (trang công khai không có khách) chỉ chịu tầng theo địa chỉ. Vượt
mức: 429 `tenant_rate_limited` kèm `Retry-After`. Đếm như tầng theo API key: GCRA trong bộ nhớ mỗi tiến trình, hay
chung qua database (`AUTOXIDE_HTTP_RATE_LIMIT_STORE=database`, cửa sổ cố định một phút, lấy theo lô; khoá đếm
`tenant:<khách>` tách khỏi `api_key:…`). Không truy vấn database mỗi request ở kiểu bộ nhớ; mức theo gói đọc từ ảnh chụp
gói (`plans` có cache ngắn).

Lý do: chủ dự án yêu cầu một khách ồn ào không làm nghẽn khách khác (SaaS nhiều khách). Tầng theo API key không đủ:
một khách có nhiều key và nhiều phiên đăng nhập. Dùng lại cách đếm sẵn có (một cách làm cho mỗi việc).

Đã cân nhắc: đặt ở lớp HTTP chung (trước router module) — lúc đó chưa giải được khách; giải khách hai lần tốn thêm. Mức
theo cấu hình khách (`kernel.settings`) — admin của khách tự nâng mức của mình, sai mục đích; mức thuộc gói (người vận
hành bán). Burst riêng theo gói — thêm một hạn mức nữa mà ít người cần; để bằng mức mỗi phút.
