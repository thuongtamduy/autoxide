## D-20261126-api-quota-signed-in · Hạn mức `kernel.api_requests`: đếm sau giới hạn theo khách, chỉ chặn người gọi của khách — 2026-11-26 — agent quyết

Chủ đề: HTTP, gói

Quyết định: Bổ sung D-20261126-tenant-rate-limit cho hạn mức số request mỗi kỳ của gói (`kernel.api_requests`):

- **Đếm sau khi request được cho qua**: lớp giới hạn theo khách đặt NGOÀI lớp gói, và lớp gói chỉ đếm khi
  request đã qua mọi chốt (hết lượng, module không có trong gói, khách chỉ đọc). Request bị 429 `tenant_rate_limited`
  hay bị chốt của gói từ chối không tiêu lượng của kỳ. MCP cũng vậy: lời gọi tool bị 429 không được đếm.
- **Chặn khi hết lượng cũng theo `budget_tenant`**: chỉ người gọi được tính vào khách (đã xác thực, trong khách của
  mình hay khách đã uỷ quyền) mới nhận 429 `quota_exhausted`. Request ẩn danh (trang công khai, webhook gửi đến),
  nhân sự nền tảng thao tác trong khách và quyền hỗ trợ vẫn qua — đúng ý "hỗ trợ giúp được khách đang ở trần".
- **Xác thực thất bại được nhớ trong request** (`AuthenticationFailed` cạnh `Authenticated`): các lớp và handler không
  xác thực lại một credential sai (trước đây tới 4 lần một request), nên API key hợp lệ mà hỏng ở bước sau (khách
  không khớp, dải IP, origin) không bị trừ bộ đếm riêng của key nhiều lần.

Lý do: rà soát #532 — agent retry qua 429 sẽ đốt hạn mức của gói; khách hết lượng khiến cả webhook thanh toán và
người hỗ trợ bị chặn; xác thực lặp lại tốn công và đếm sai.

Đã cân nhắc: đếm trước rồi hoàn lại khi bị từ chối — thêm đường hoàn tác cho bộ đếm, dễ lệch; đổi thứ tự lớp đơn giản
hơn. Chặn mọi request khi khách hết lượng (như trước) — chặn cả webhook của nhà cung cấp, mất event thanh toán.
