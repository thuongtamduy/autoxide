## D-20261118-step58-automations · Tự động hoá quy trình: luật, bộ chạy, duyệt, cảnh báo số liệu — 2026-11-18 — agent quyết

Chủ đề: MCP, AI, agent dùng API

Quyết định:

1. **Hành động chỉ là tool MCP**, gọi qua sổ tool của kernel `autoxide_kernel::automation::AutomationDirectory`
   (`StartContext::automation_directory()`, điền sau khi mọi module khởi động). `AutomationDirectory::call` dùng chung
   hàm với endpoint `/mcp` (kiểm permission, gói của khách, khách chỉ đọc) — luật không làm được điều REST / MCP với cùng
   permission không làm được. Tool của chính `automations` không làm hành động được (luật gọi luật đi qua event, có chống
   vòng lặp). Thêm ba hành động sẵn có: `wait` (1 phút – 30 ngày, job hẹn giờ), `stop_if`, `notify` (thông báo
   `automations.alert` tới `run_as` qua `notifications`; không cài thì bước ghi `skipped`).
2. **Chạy với quyền HIỆN TẠI của `run_as`** (`Authenticator::principal_of`, mỗi lần chạy). `run_as` là người tạo, hay
   người sửa thân luật / bật luật gần nhất — người làm luật hoạt động chịu trách nhiệm cho nó. Thiếu permission của một tool
   → luật `paused` (`permission_lost`); tài khoản không còn → `paused` (`owner_unavailable`); event
   `automations.rule_paused`, hộp việc `automations.paused`.
3. **Luật do AI agent tạo / sửa thân** ở `pending_approval`; agent không bật được (`automations.human_required`). Đây là
   duyệt của chính module, không dùng `kernel_approval_requests` (P12): yêu cầu duyệt của kernel là "gửi lại đúng
   request HTTP", còn luật cần một người nhận trách nhiệm chạy lâu dài (`run_as` = người bật).
4. **Bước rủi ro** (tool `destructive` hay `requires_approval_for_agent`) chờ người duyệt ở MỖI lần chạy:
   `POST /runs/{id}/approve | reject`, người duyệt cần `automations.run` và permission của chính tool đó, không phải
   agent. Như điểm 3: duyệt gắn với lần chạy trong module, hộp việc `automations.approvals`.
5. **Chống vòng lặp**: thêm `AccessPath::Automation(run_id)` ở kernel; người chạy hành động mang đường vào này nên event
   sinh ra có `actor_access = automation` + ID lần chạy (cột sẵn có, không migration). Event từ lần chạy của cùng luật
   không kích hoạt lại nó; độ sâu chuỗi luật tối đa 3; vượt thì ghi lần chạy `skipped` mã `automations.loop_detected`.
   Mỗi luật tối đa `AUTOXIDE_AUTOMATIONS_MAX_RUNS_PER_HOUR` (mặc định 1000) lần chạy mỗi giờ, vượt → `paused`
   (`rate_limit`).
6. **Chạy một lần**: khoá `automations_run_keys (tenant, rule, trigger_key)` ghi cùng transaction với lần chạy và job
   (`event:<id>`, `schedule:<mốc>`, `metric:<ngày>`, `manual:<id>`). Mỗi bước ghi lại ngay khi xong (`cursor`); bước gọi
   tool mang `Idempotency-Key` `automation:<run_id>:<bước>` qua nơi lưu idempotency chung với REST — chạy lại sau lỗi
   bắt đầu từ bước hỏng, bước đã có kết quả lưu thì lấy lại kết quả.
7. **Cú pháp lịch**: không dùng cron; ba nhịp `hour` (phút), `day` (giờ:phút), `week` (thứ + giờ:phút) theo múi giờ khai
   trong kích hoạt, không thì của nút (`organizations.timezone`), không thì UTC. Giới hạn: tối đa 20 hành động, cây điều
   kiện 50 nút, sâu 6 cấp.
8. **Cảnh báo số liệu** (D-229): `fact:<loại>` (số liệu báo cáo của `Module::facts`) cộng theo giờ UTC vào bảng của module,
   CHỈ cho loại khách có luật dùng (khách không dùng không tốn lần ghi nào); giá trị là ngày địa phương vừa hết.
   `summary:<id>` (số liệu tóm tắt P4, thêm `StartContext::summary_metrics()` + `SummaryMetrics::find`) đọc lúc kiểm với
   quyền của `run_as`, mỗi lần đọc một mẫu. Mức nền = trung bình cùng thứ 4 tuần trước; thiếu dữ liệu một tuần thì không báo.
   Tỉ lệ tính phần vạn. Một lần mỗi kỳ (khoá `metric:<ngày>`).
9. **Mẫu luật**: `Module::automation_templates` (cuối trait, mặc định rỗng); `GET /templates` chỉ trả mẫu người gọi lưu được
   ngay. Mẫu nghiệp vụ (doanh thu tụt, sắp hết hàng, tỉ lệ bom hàng, phiếu hỗ trợ quá hạn) do module sở hữu số liệu khai
   khi dựng (orders, inventory, shipping, support) — `automations` không biết khái niệm thương mại.
10. Tham số của tool kiểm với JSON Schema đầu vào bằng bộ kiểm tự viết (phần schema mà tool của Autoxide sinh ra), không
    thêm thư viện; lỗi giữ mã của trường (`required`, `type`…), gom cùng lỗi khác một lần.

Lý do: luật kiến trúc 8 (một đường cho mọi lệnh), không leo thang quyền, agent hay retry nên mọi thứ chạy lại được; dữ liệu
nền tự giữ để không đọc bảng của module khác.

Đã cân nhắc: gọi service của module khác trực tiếp (vi phạm luật 8); chạy với quyền "hệ thống" (leo thang quyền); dùng
yêu cầu duyệt P12 cho bước rủi ro (cần gửi lại request HTTP, không hợp với job); cột `automation` trên mọi bảng outbox
(migration ở mọi module); cron đầy đủ (khó kiểm, khó giải thích); đọc số liệu ngày từ `reporting` (bảng của module khác,
và `reporting` không bắt buộc phải cài); thư viện JSON Schema (một phụ thuộc cho vài chục dòng).
