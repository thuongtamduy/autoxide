## D-20261126-tenant-rate-limit · Giới hạn request theo khách: tầng thứ ba, mức theo gói — 2026-11-26 — agent quyết

Chủ đề: HTTP, REST, OpenAPI

Quyết định: Thêm tầng giới hạn request thứ ba, theo KHÁCH, sau tầng theo địa chỉ client và tầng theo API key. Mức:
`kernel.api_rate_per_minute` của gói khách khi gói nêu (kể cả "không giới hạn"; burst bằng chính mức đó), không thì
`AUTOXIDE_HTTP_RATE_LIMIT_TENANT_PER_MINUTE` / `_BURST` (mặc định 0 = tắt → hành vi cũ). Áp cho route REST của mọi
module (lớp bọc router của module, cùng chỗ đếm `kernel.api_requests`) và mọi lời gọi tool MCP. Vượt mức: 429
`tenant_rate_limited` kèm `Retry-After`. Đếm như tầng theo API key: GCRA trong bộ nhớ mỗi tiến trình, hay chung qua
database (`AUTOXIDE_HTTP_RATE_LIMIT_STORE=database`, cửa sổ cố định một phút, lấy theo lô; khoá đếm
`tenant:<khách>` tách khỏi `api_key:…`). Không truy vấn database mỗi request ở kiểu bộ nhớ; mức theo gói đọc từ ảnh chụp
gói (`plans` có cache ngắn).

Ai được tính vào khách (sửa sau rà soát, trước khi lên `main`):

- **Chỉ người gọi đã xác thực** (phiên, API key, token MCP), vào **khách của chính họ**. Request không có thông tin xác
  thực, hay có mà không xác thực được, KHÔNG bao giờ trừ vào khách nào, dù nêu khách bằng `Autoxide-Tenant` hay
  `Host`: những gợi ý đó chưa xác thực, ai cũng gửi được — nếu tính, người ngoài làm cạn hạn mức của khách bất kỳ và
  mọi người dùng thật của khách nhận 429. Hệ quả: route công khai và webhook gửi đến của nhà cung cấp (không mang
  thông tin xác thực của Autoxide) không bao giờ bị 429 vì khách ở trần; chúng chỉ chịu tầng theo địa chỉ. Dấu hiệu
  dùng là chính việc xác thực thành công, không có danh sách đường dẫn viết tay. Đăng nhập, làm mới token, callback
  OIDC vốn ẩn danh nên cũng không bị chặn — khách ở trần vẫn đăng nhập được.
- **Nhân sự nền tảng thao tác trong khách khác** (`Principal::operator_from`) và **quyền hỗ trợ**
  (`AccessPath::Support`) KHÔNG tính vào khách đích: để hỗ trợ được chính khách đang bị nghẽn. Uỷ quyền
  (`AccessPath::Delegation`, bên được khách giao việc) vẫn tính vào khách — không thì là đường vòng qua hạn mức.
- **Lô ghi đồng bộ** (`/kernel/sync/batch`) tính MỘT lượt: request đã tính mang dấu, lệnh con chép phần mở rộng của
  lô nên không tính lại (không 429 giữa lô).
- Cùng quy tắc cho hạn mức `kernel.api_requests` (trước đây cũng lấy khách từ gợi ý chưa xác thực).
- `0` trong gói là chặn mọi request (như mọi hạn mức của gói); `0` ở biến môi trường là tắt (như các biến rate limit
  khác) — ghi rõ ở mô tả của cả hai.

Lý do: chủ dự án yêu cầu một khách ồn ào không làm nghẽn khách khác (SaaS nhiều khách). Tầng theo API key không đủ:
một khách có nhiều key và nhiều phiên đăng nhập. Dùng lại cách đếm sẵn có (một cách làm cho mỗi việc).

Đã cân nhắc: đặt ở lớp HTTP chung (trước router module) — lúc đó chưa giải được khách; giải khách hai lần tốn thêm. Mức
theo cấu hình khách (`kernel.settings`) — admin của khách tự nâng mức của mình, sai mục đích; mức thuộc gói (người vận
hành bán). Burst riêng theo gói — thêm một hạn mức nữa mà ít người cần; để bằng mức mỗi phút.
