# Bước 52 — Trải nghiệm quản trị nâng cao

> Lộ trình: `docs/ROADMAP.md` bước 52 · Thiết kế: `docs/research/ui-design.md` mục 5 · Quyết định:
> D-109 (AI chỉ thành bản nháp), D-200, D-201

## Mục tiêu

Những năng lực quản trị mà các sản phẩm hàng đầu có và nhà phát triển, nhà vận hành, AI agent dùng hằng
ngày, sau khi bước 51 đã có menu, hộp việc, tìm kiếm, view, thao tác hàng loạt.

## Bắt đầu khi

SAU bước 51. Mục AI cần module `ai` (bước 18) và P7 của bước 51 (schema bộ lọc).

## Thiết kế

### 1. Bàn kỹ thuật

- Nhật ký request theo khách (bảng `kernel_request_log`, cấp khách hàng, có hạn lưu là cấu hình, mặc định
  14 ngày, xoá bằng job): thời điểm, API key hay người dùng, phương thức, **đường dẫn mẫu** (không có ID
  thật), status, `code` lỗi, thời gian xử lý, `Idempotency-Key`. Không lưu thân request, header xác thực,
  query có dữ liệu cá nhân. Ghi bất đồng bộ, không làm chậm request; đo trước / sau.
- Webhook: danh sách lần giao (module `webhooks` đã có bảng giao), lọc theo trạng thái, gửi lại một lần
  giao. Event theo bản ghi: đọc từ outbox theo `kind` + `id`. Dead-letter queue: chạy lại có audit.
- Permission `kernel.devtools.read`. Tool MCP tương ứng.

### 2. Hoàn tác an toàn

- Thao tác lưu trữ / ẩn của module có `restore` tương ứng; giao diện hiện "Hoàn tác" vài giây.
- Lệnh ghi lớn (thao tác hàng loạt, nhập CSV, đổi giá hàng loạt) nhận `dry_run=true`: trả danh sách thay
  đổi sẽ xảy ra, không ghi gì, không phát event.

### 3. AI trong quản trị

- `POST /api/v1/ai/filters/parse {resource, text}` → `params` đã kiểm theo schema bộ lọc (P7 bước 51) và
  phần không hiểu. Không bao giờ tự chạy lệnh ghi.
- Trợ lý gọi tool MCP với đúng quyền của người dùng; lệnh ghi chỉ ra bản nháp hay `dry_run` chờ người duyệt
  (D-109); lệnh rủi ro đi qua yêu cầu duyệt của P12 bước 51.

### 4. Thông báo và ghim

- `notifications` (bước 37): mỗi tin có `reason`; trạng thái `done` / `saved` (tin `done` giữ 90 ngày);
  bộ lọc đã lưu; cấu hình kênh theo loại event.
- `/api/v1/me/pins`: ghim mục điều hướng hay bản ghi (tối đa 20), theo khách.

### 5. Sức khoẻ người bán và tuân thủ

- `marketplace`: bảng dẫn xuất chỉ số người bán (giao đúng hạn, huỷ do người bán, đánh giá, phản ánh vi
  phạm) theo cửa sổ 3 tháng, tính lại hằng ngày bằng job; ngưỡng và huy hiệu "người bán tin cậy" là dữ
  liệu có ngày hiệu lực; endpoint công khai trả huy hiệu kèm tiêu chí. Huy hiệu xác thực danh tính (bước
  37) tách riêng, không gộp.
- `compliance`: `GET /api/v1/compliance/health` tóm tắt theo nghĩa vụ (website TMĐT, KYC, dữ liệu cá nhân,
  văn bản, vi phạm) với mức đỏ / vàng / xanh và hạn gần nhất.

## Test bắt buộc

`request_log_khong_luu_than_va_token`, `request_log_het_han_bi_xoa`, `dry_run_khong_ghi_khong_phat_event`,
`restore_tra_lai_trang_thai_truoc`, `ai_filter_parse_chi_tra_tham_so_hop_le` (dùng `FakeTextModel`),
`seller_badge_tinh_tu_so_lieu_that` — trên cả hai database.

## Agent làm bước này tự quyết

Hạn lưu mặc định của nhật ký request; ghi nhật ký bằng outbox hay hàng đợi trong bộ nhớ có giới hạn (đo
trước); danh sách chỉ số sức khoẻ người bán đầu tiên. Ghi vào `docs/DECISIONS.md`.

## Bổ sung theo D-202 (nhật ký lệnh của agent)

- Lệnh gọi qua MCP ghi `mcp:<tên tool>` (vd `mcp:comments.answer`) vào cột đường dẫn mẫu của
  `kernel_request_log`, phương thức `MCP`; **không ghi tham số** của tool (có thể chứa dữ liệu cá nhân).
  Ghi ở lớp MCP của kernel (`mcp.rs`), một dòng cho mỗi lần gọi tool, kể cả khi tool trả lỗi.
- Cột mới `actor_kind` (`human` / `agent` / `system`, P12 bước 51) và `api_key_id`; `GET
  /api/v1/kernel/devtools/requests` lọc theo `actor_kind`, `api_key_id`, `path` (chính xác, vd
  `mcp:orders.refund`), `status`, khoảng thời gian; phân trang bằng `fetch_page`, index (`tenant_id`,
  `actor_kind`, `id`) và (`tenant_id`, `api_key_id`, `id`).
- Lệnh nhận `202 approval_required` (P12 bước 51): dòng nhật ký giữ `approval_id` để giao diện nối tới
  yêu cầu duyệt; khi lệnh được duyệt và chạy, dòng mới ghi cùng `approval_id`.
- Tool MCP `kernel.request_log` (`kernel.devtools.read`, `read_only`) như REST.
- **Test:** `lenh_mcp_ghi_dung_ten_tool`, `tham_so_tool_khong_lot_vao_nhat_ky`,
  `loc_nhat_ky_theo_actor_kind_va_key`, `lenh_cho_duyet_giu_approval_id` — cả hai database.

## Bổ sung theo đối chiếu thiết kế 2 (D-208…D-218)

> Nguồn: `docs/prompts/ui-trace-roadmap.md`; mã `A-…` / `B-…` trỏ tới `docs/research/ui-trace/`.

- **Tên endpoint bàn kỹ thuật (A-c10).** `GET /api/v1/kernel/devtools/requests` (đã ghi ở mục D-202),
  `POST /api/v1/webhooks/deliveries/{id}/redeliver`, `GET /api/v1/kernel/devtools/events?object=<kind>:<id>`
  — **đọc từ audit**, vì outbox bị dọn sau khi giao. Permission đọc `kernel.devtools.read`, ghi (gửi lại,
  chạy lại job) `kernel.devtools.manage`. Nhật ký request lưu thêm `request_id`, `trace_id` để nối với OTLP.
- **Trợ lý AI trong quản trị (A-c12).** `POST /api/v1/ai/assistant/messages {page, text}`: chạy tool MCP với
  **quyền của người gọi**, chỉ tool `read_only` khi khách chưa bật ghi; lệnh ghi thành bản nháp hay yêu cầu
  duyệt (D-109, P12). Trả lời kèm danh sách tool đã gọi để người dùng kiểm.
