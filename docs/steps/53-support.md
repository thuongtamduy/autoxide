# Bước 53 — Hỗ trợ khách hàng (`support`)

> Lộ trình: `docs/ROADMAP.md` bước 53 · Quyết định: D-204 · Đặc tả liên quan: `docs/steps/37-compliance.md`
> (vụ việc `consumer_complaint`, `notifications`), `docs/steps/45-orders.md` (nút "mở hỗ trợ" của đơn,
> mục D-202), `docs/steps/46-marketplace.md` (tranh chấp), `docs/steps/51-ui-contracts.md` (hộp việc)

## Mục tiêu

Người mua (có tài khoản hay vãng lai) gửi được yêu cầu hỗ trợ — hỏi về đơn, sản phẩm, tài khoản, thanh
toán — và nhân viên của cửa hàng nhận, phân công, trả lời, theo dõi hạn trả lời cho tới khi xong. Mỗi yêu
cầu là một **phiếu** (ticket) gồm các tin nhắn qua lại. Không phải chat thời gian thực (để sau, D-194).

Phiếu thuộc về một nút tổ chức (cửa hàng, gian hàng của người bán trên sàn) để chuỗi và sàn chia việc theo
nhánh như mọi dữ liệu khác (D-185).

## Ranh giới với module khác — không làm trùng

| Việc | Ở đâu | `support` làm gì |
|---|---|---|
| Khiếu nại người tiêu dùng có hạn xử lý theo luật (Luật 19/2023) | `compliance_cases` kind `consumer_complaint` (bước 37) | Nhân viên **chuyển** phiếu thành vụ việc qua trait `ComplaintIntake` của `compliance`; phiếu giữ `case_id`, đóng với kết quả `escalated`. Người mua chọn "khiếu nại" trên form thì phiếu tạo vụ việc ngay, không giữ hai bản |
| Tranh chấp người mua – người bán trên sàn | `marketplace_disputes` (bước 46) | Không xử lý tiền hay phán quyết; phiếu chỉ liên kết `dispute_id` khi người mua mở tranh chấp từ phiếu |
| Yêu cầu trả hàng | `orders_returns` (bước 45) | Không tạo yêu cầu trả hàng; phiếu dẫn người mua sang luồng trả hàng của `orders` |
| Báo vi phạm sở hữu trí tuệ / hàng giả | `compliance_cases` (bước 37) | Không nhận; form chỉ dẫn sang `/public/complaints` |
| Thông báo cho người mua và nhân viên | `notifications` (bước 37) | Gửi qua `Notifier`, tin **giao dịch** (không cần đồng ý quảng cáo) |

## Bắt đầu khi

- Bước 37 xong: `notifications` (`Notifier`), `compliance` (`ComplaintIntake` — nếu chưa có thì bước này
  thêm trait đó vào `compliance::api`, gọi chính service tạo vụ việc).
- Sẵn từ trước: 33 (khách), 34 (nút tổ chức, quyền theo độ sâu, người mua là tài khoản theo khách), 35
  (tính năng `support` trong gói), 16 (`media` cho tệp đính kèm), D-123 (sổ đối tượng để gắn phiếu với
  đơn / sản phẩm).
- Song song được với 41–49. Bước 45 đã có thì nút "mở hỗ trợ" của đơn vãng lai gọi `SupportIntake`; bước
  45 làm trước thì nó gọi qua điểm nối tuỳ chọn (không có `support` → ẩn nút, trả 404 `orders.support_unavailable`).
- Bước 51 có hộp việc thì `support` khai nguồn việc ngay trong bước này; chưa có thì bước 51 khai thay.

## Phạm vi

Làm: phiếu và tin nhắn; tạo từ website (có tài khoản, vãng lai qua email / số điện thoại đã xác thực OTP,
từ token xem đơn của bước 45), từ nhân viên (ghi hộ cuộc gọi), từ email trả lời (tuỳ chọn, xem dưới);
phân loại, mức ưu tiên, phân công người / nhóm; trạng thái và hạn trả lời (SLA) theo chính sách của nút;
câu trả lời mẫu; ghi chú nội bộ; tệp đính kèm qua `media`; gắn phiếu với bản ghi khác (đơn, sản phẩm…);
chuyển thành khiếu nại; đánh giá mức hài lòng sau khi đóng; tìm và lọc; tool MCP; gợi ý trả lời bằng AI
(chỉ bản nháp, D-109).

Không làm: chat thời gian thực, gọi điện, tổng đài; trợ lý AI tự trả lời người mua (luôn có người duyệt);
cơ sở tri thức / trang câu hỏi thường gặp (dùng `content`).

## Thiết kế

### Phiếu và trạng thái

`open` (mới, chưa ai trả lời) → `pending_agent` (người mua vừa viết) ⇄ `pending_customer` (nhân viên vừa
trả lời, chờ người mua) → `solved` (nhân viên đánh dấu xong) → `closed` (sau `auto_close_days` không ai
mở lại, hay người mua xác nhận). Người mua viết vào phiếu `solved` thì phiếu về `pending_agent`; phiếu
`closed` không mở lại — người mua tạo phiếu mới, liên kết `follow_up_of`. Chuyển trạng thái chỉ qua một
hàm của service (máy trạng thái, như `orders`).

- **Kết quả khi đóng** (`resolution`): `answered`, `escalated` (thành khiếu nại), `duplicate`, `spam`,
  `no_reply` (người mua không trả lời).
- **Ưu tiên:** `low` / `normal` / `high` / `urgent`. **Loại** (`category`): danh sách do khách cấu hình
  (mã + nhãn i18n), mặc định `order`, `product`, `payment`, `account`, `other`.
- **Hạn trả lời (SLA):** chính sách theo nút, kế thừa theo cây (khuôn cài đặt bước 34): thời gian phản hồi
  đầu tiên và thời gian giải quyết theo ưu tiên, tính theo **giờ làm việc** của nút (giờ mở cửa ở hồ sơ
  công khai của nút, D-200; ngày nghỉ là dữ liệu). Phiếu lưu `first_response_due_at`, `resolve_due_at`
  tính sẵn lúc tạo / đổi ưu tiên; đồng hồ dừng khi chờ người mua. Job đánh dấu quá hạn và phát event.
- **Phân công:** `assignee_user_id` (người) và / hoặc `team_id` (nhóm nhân viên của nút). Quy tắc tự phân
  công là dữ liệu: theo loại / nút / kênh → nhóm; trong nhóm chia vòng (round-robin) cho người đang bật
  "nhận việc". Không phân công tự động cho người không có quyền ở nút của phiếu.

### Ai tạo, ai xem

- **Người mua có tài khoản:** thấy phiếu của mình (`/me/support/tickets`), mọi tin nhắn trừ ghi chú nội bộ.
- **Người mua vãng lai:** tạo bằng email hay số điện thoại; trước khi phiếu nhận tin nhắn thứ hai phải xác
  thực OTP (email: link; điện thoại: SMS của bước 37). Xem phiếu bằng **token phiếu** (băm lưu, gửi trong
  thư / SMS, sống tới khi phiếu `closed` + 30 ngày). Tạo tài khoản sau đó thì `POST /me/support/claim`
  gộp các phiếu cùng email / số đã xác thực (như `orders` claim).
- **Từ token xem đơn của bước 45:** phiếu tự gắn đơn và dùng số đã xác thực, không hỏi OTP lại.
- **Nhân viên:** permission `support.tickets.read` / `support.tickets.reply` / `support.tickets.manage`
  theo phạm vi nút (bước 34): thấy phiếu của nút mình và nhánh dưới. Người bán trên sàn chỉ thấy phiếu của
  gian hàng mình; nhân viên sàn thấy phiếu được chuyển lên sàn (`escalated_to_platform`).
- Chống spam form công khai qua `SpamGuard` (D-119) và throttle theo IP băm, email, số điện thoại.

### Tin nhắn, đính kèm, ghi chú

- Tin nhắn chỉ thêm, không sửa (sửa nhầm → nhân viên ẩn tin, ghi lý do, audit); chữ thuần (an toàn,
  hiển thị như `comments`: `body_html` đã thoát).
- `kind`: `public` (người mua thấy) / `internal` (ghi chú nội bộ, nhắc tên đồng nghiệp bằng `@` → thông
  báo) / `system` (đổi trạng thái, phân công — sinh tự động, i18n key).
- Đính kèm: `media_ids` (tối đa 5 / tin), ảnh riêng tư, kiểm chủ sở hữu qua trait của `media` (như trả
  hàng, D-202); người mua tải qua endpoint tải lên dành cho người mua của `media`.
- **Câu trả lời mẫu** (macro): theo nút, kế thừa, có biến (`{customer_name}`, `{order_number}`…) điền lúc
  chèn; ngôn ngữ theo người mua.

### Gắn với bản ghi khác

`links`: danh sách `<kind>:<id>` (vd `orders.order:…`, `catalog.product:…`), kiểm qua sổ đối tượng
(D-123) lúc gắn — chỉ gắn bản ghi người gọi được xem. Trang đơn của nhân viên hiện "phiếu liên quan"
qua `GET /tickets?link=orders.order:<id>`. Module khác không đọc bảng của `support`.

### Email vào (tuỳ chọn, tắt mặc định)

Trả lời thư thông báo của phiếu thì vào lại phiếu: địa chỉ `Reply-To` mang token phiếu
(`support+<token>@…`); bộ nhận thư là trait `InboundMail` (bộ cài đặt thật — webhook của nhà cung cấp
email — là module riêng, làm khi có tài khoản; D-191). Thư không khớp phiếu nào thì bỏ, không tạo phiếu
mới (chống spam). Cắt phần trích dẫn thư cũ bằng quy tắc đơn giản; giữ bản gốc trong `raw_media_id`.

### Chuyển thành khiếu nại

`POST /tickets/{id}/escalate {kind: consumer_complaint}` (`support.tickets.manage`): gọi
`ComplaintIntake::open(tenant, ComplaintFromTicket { … })` của `compliance` NGOÀI transaction, rồi ghi
`case_id` và đóng phiếu `escalated` trong transaction của `support`. Gọi lại với cùng `Idempotency-Key`
không tạo hai vụ việc (`compliance` nhận `source_ref = "support.ticket:<id>"` duy nhất). Người mua chọn loại
"khiếu nại" trên form công khai thì đi thẳng sang `compliance` (không tạo phiếu).

### AI (D-109)

`POST /tickets/{id}/reply-suggestions` gọi `TextModel` với lịch sử phiếu (bỏ email, số điện thoại) và câu
trả lời mẫu liên quan; kết quả là **bản nháp** nhân viên sửa rồi gửi, tin nhắn mang `ai_assisted = true`
(bước 37 gắn nhãn). Không gọi trong transaction ghi. Tắt khi module `ai` chưa cấu hình.

### Kiểu và trait (phác thảo — tra API thật bằng skill `check-library-api`)

```rust
// support::api — cho module khác
#[async_trait] pub trait SupportIntake: Send + Sync {
    /// Tạo phiếu thay người mua (vd từ trang đơn vãng lai của `orders`). Trùng `reference` → phiếu cũ.
    async fn open(&self, tenant: &TenantContext, ticket: NewTicket) -> Result<TicketRef, Error>;
}
pub struct NewTicket { pub org_id: Uuid, pub requester: Requester, pub category: String,
    pub subject: String, pub body: String, pub links: Vec<String>, pub reference: String }
pub enum Requester { User(Uuid), VerifiedPhone(PhoneNumber), VerifiedEmail(String) }
pub struct TicketRef { pub id: Uuid, pub number: String, pub access_token: Option<String> }
```

`compliance::api` thêm (nếu chưa có) `ComplaintIntake::open(tenant, ComplaintFromTicket) -> CaseRef`.

### Bảng dữ liệu (mọi bảng có `tenant_id`, D-185)

| Bảng | Cột chính | Index / ràng buộc |
|---|---|---|
| `support_tickets` | `id`, `tenant_id`, `org_id`, `number` (đánh số của kernel, tiền tố theo nút), `requester_user_id` NULL, `requester_email` NULL, `requester_phone_hash` NULL, `requester_name`, `verified_at` NULL, `token_hash` NULL, `category`, `priority`, `status`, `resolution` NULL, `subject`, `channel` (`web/guest/order_lookup/staff/email`), `assignee_user_id` NULL, `team_id` NULL, `first_response_due_at` NULL, `resolve_due_at` NULL, `first_responded_at` NULL, `solved_at` NULL, `closed_at` NULL, `breached` BOOL, `case_id` NULL, `dispute_id` NULL, `follow_up_of` NULL, `escalated_to_platform` BOOL, `satisfaction` SMALLINT NULL (1–5), `satisfaction_note` NULL, `last_message_at`, `reference` NULL, `search_key`, `created_at`, `updated_at` | unique (`tenant_id`, `number`); unique (`tenant_id`, `reference`); index (`tenant_id`, `org_id`, `status`, `id`), (`tenant_id`, `assignee_user_id`, `status`, `id`), (`tenant_id`, `requester_user_id`, `id`), (`tenant_id`, `status`, `resolve_due_at`) |
| `support_messages` | `id`, `tenant_id`, `ticket_id`, `kind` (`public/internal/system`), `author_kind` (`customer/agent/system`), `author_user_id` NULL, `body`, `media_ids` text, `hidden_at` NULL, `hidden_by` NULL, `hidden_reason` NULL, `ai_assisted` BOOL, `created_at` | index (`tenant_id`, `ticket_id`, `id`) — chỉ thêm |
| `support_links` | `tenant_id`, `ticket_id`, `target` (`<kind>:<id>`), `created_at` | pk (`tenant_id`, `ticket_id`, `target`); index (`tenant_id`, `target`) |
| `support_teams` · `support_team_members` | nhóm theo nút; thành viên, cờ `accepting` | unique (`tenant_id`, `org_id`, `name`) |
| `support_policies` | `tenant_id`, `org_id`, `categories` JSON, `first_response_minutes` JSON (theo ưu tiên), `resolve_minutes` JSON, `auto_close_days`, `holidays` JSON, `guest_tickets` BOOL, `inbound_email` BOOL, `updated_at` | pk (`tenant_id`, `org_id`); kế thừa theo cây |
| `support_macros` | `id`, `tenant_id`, `org_id`, `name`, `bodies` JSON (theo ngôn ngữ), `created_by`, `updated_at` | index (`tenant_id`, `org_id`) |
| `support_routing_rules` | `id`, `tenant_id`, `org_id`, `match` JSON (loại, kênh), `team_id`, `position` | index (`tenant_id`, `org_id`, `position`) |

Cộng bảng `support_events` (outbox), `support_jobs`, `support_throttle`, `support_tokens` của kernel.

### API REST (`/api/v1/support`)

| Endpoint | Permission | Ghi chú |
|---|---|---|
| `POST /public/tickets` | công khai (chống spam) hay đăng nhập | 201; vãng lai trả `access_token` một lần |
| `POST /public/tickets/{id}/verify` | token phiếu | OTP email / SMS |
| `GET /public/tickets/{id}` · `POST /public/tickets/{id}/messages` · `POST /public/tickets/{id}/satisfaction` | token phiếu | không thấy ghi chú nội bộ; email / số che như D-202 |
| `GET /me/tickets` · `GET /me/tickets/{id}` · `POST /me/tickets/{id}/messages` · `POST /me/claim` | `support.request` (người mua) | |
| `GET /tickets` | `support.tickets.read` | lọc `status`, `assignee=me/none/<id>`, `team_id`, `org_id`, `category`, `priority`, `breached`, `link`, `q` (không dấu, D-202); `fetch_page` / `fetch_page_by` (`sort=due`) |
| `GET /tickets/{id}` · `POST /tickets/{id}/messages` | `…read` / `…reply` | `kind=internal` cho ghi chú; `Idempotency-Key` |
| `PATCH /tickets/{id}` | `support.tickets.reply` | trạng thái, ưu tiên, loại, phân công; `If-Match` |
| `POST /tickets/{id}/links` · `DELETE …/links/{target}` | `…reply` | kiểm qua sổ đối tượng |
| `POST /tickets/{id}/escalate` | `support.tickets.manage` | thành khiếu nại / lên sàn |
| `POST /tickets/{id}/reply-suggestions` | `…reply` | AI, bản nháp |
| `GET/PUT /policies/{org_id}` · `/teams` · `/macros` · `/routing-rules` | `support.settings.manage` | |
| `GET /reports/summary` | `support.tickets.read` | số phiếu, thời gian phản hồi / giải quyết trung vị, tỉ lệ quá hạn, hài lòng — theo nút, khoảng thời gian |

### Tool MCP

`list_tickets`, `get_ticket` (`read_only`), `reply_ticket`, `add_internal_note`, `update_ticket`,
`link_ticket`, `suggest_reply` (`read_only`, bản nháp). Cùng service, cùng permission (luật 8). Agent trả
lời người mua là lệnh ghi bình thường; khách muốn duyệt trước thì bật cho `reply_ticket` cơ chế
`approval_required` của P12 bước 51.

### Event

`support.ticket_created`, `support.ticket_status_changed`, `support.ticket_assigned`,
`support.message_added` (không mang nội dung — chỉ id, loại, tác giả), `support.ticket_breached`,
`support.ticket_escalated`, `support.satisfaction_recorded`. Không mang email, số điện thoại, token.

Nghe: `privacy::USER_DELETED` (ẩn danh hoá người mua, giữ phiếu cho thống kê), `orders.order_*` không cần.

### Job và lịch

`support.sla_check` (mỗi phút: đánh dấu quá hạn, phát event, đưa vào hộp việc), `support.auto_close`
(hằng ngày), `support.retention` (xoá phiếu đóng quá `AUTOXIDE_SUPPORT_RETENTION_DAYS`, mặc định 730 ngày
— dữ liệu có ngày hiệu lực khi luật đổi).

### Cấu hình

`AUTOXIDE_SUPPORT_RETENTION_DAYS` (730), `AUTOXIDE_SUPPORT_GUEST_PER_HOUR` (5 / IP băm),
`AUTOXIDE_SUPPORT_MAX_BODY_CHARS` (10000), `AUTOXIDE_SUPPORT_TOKEN_DAYS_AFTER_CLOSE` (30). Chính sách kinh
doanh (loại, SLA, giờ làm, tự đóng) là dữ liệu theo nút, không phải biến môi trường.

### Permission

`support.request` (người mua), `support.tickets.read`, `support.tickets.reply`, `support.tickets.manage`,
`support.settings.manage`. Tính năng gói (bước 35): `support`, `support.sla`, `support.ai`.

### Mã lỗi (kèm bản dịch `vi`)

`support.ticket_not_found` (404), `support.verification_required` (403 — vãng lai chưa xác thực OTP),
`support.ticket_closed` (409 — tạo phiếu mới), `support.invalid_transition` (409),
`support.assignee_out_of_scope` (422), `support.link_not_found` (422), `support.attachment_not_owned` (422),
`support.category_unknown` (422), `support.already_escalated` (409), `support.guest_tickets_disabled` (403).

## Thứ tự làm

1. Bảng, máy trạng thái, tạo / trả lời cho người dùng có tài khoản và nhân viên; test.
2. Vãng lai (OTP, token phiếu, claim), tạo từ token xem đơn của bước 45 (`SupportIntake`).
3. SLA theo giờ làm, phân công, nhóm, quy tắc; job; hộp việc (bước 51).
4. Đính kèm, liên kết bản ghi, câu trả lời mẫu, chuyển thành khiếu nại, hài lòng.
5. Tool MCP, AI gợi ý, báo cáo; email vào (trait + bộ giả); tài liệu `docs/SUPPORT.md` (EN + VI).

## Test bắt buộc (cả hai database)

`vang_lai_phai_xac_thuc_truoc_tin_thu_hai`, `token_phieu_chi_mo_dung_mot_phieu`,
`nguoi_mua_khong_thay_ghi_chu_noi_bo`, `nhan_vien_nhanh_khac_khong_thay_phieu`,
`nguoi_ban_tren_san_chi_thay_phieu_gian_hang_minh`, `dong_ho_sla_dung_khi_cho_nguoi_mua`,
`sla_tinh_theo_gio_lam_viec_va_ngay_nghi`, `chia_vong_khong_giao_cho_nguoi_khong_co_quyen`,
`chuyen_khieu_nai_goi_lai_khong_tao_hai_vu_viec`, `tin_nhan_khong_sua_duoc_chi_an_co_ly_do`,
`phan_cong_dong_thoi_mot_ben_thang` (`If-Match`), `xoa_tai_khoan_an_danh_hoa_giu_thong_ke`,
`tool_mcp_cung_permission_nhu_rest`, `goi_y_ai_chi_la_ban_nhap` (`FakeTextModel`).

## Tiêu chí xong

Mọi mục bước 53 trong ROADMAP `[x]`; gate xanh trên cả hai database; `docs/openapi.json`, `.env.example`
sinh lại; hướng dẫn EN + VI; hộp việc có nguồn `support.tickets`; không có đường nào tạo khiếu nại hai
nơi.

## Chỗ dễ sai

- Làm lại khiếu nại trong `support` — khiếu nại có hạn luật định ở `compliance`; phiếu chỉ chuyển sang.
- Đồng hồ SLA tính theo giờ thật thay vì giờ làm của nút; quên dừng khi chờ người mua.
- Lộ số điện thoại / email của người mua cho nhân viên nhánh khác, hay trong event.
- Thư thông báo của phiếu là tin giao dịch: không đi qua sổ đồng ý quảng cáo, nhưng không được kèm quảng cáo.
- Email vào tạo phiếu mới từ thư lạ → thành kênh spam.

## Căn cứ pháp lý

| Nội dung | Văn bản | Ghi chú |
|---|---|---|
| Tiếp nhận, giải quyết khiếu nại của người tiêu dùng; thời hạn phản hồi | Luật Bảo vệ quyền lợi người tiêu dùng 19/2023/QH15; NĐ 55/2024/NĐ-CP | Phần có hạn luật định đi qua `compliance` — cần đối chiếu văn bản gốc |
| Dữ liệu cá nhân trong phiếu (tối thiểu hoá, thời hạn lưu, xoá theo yêu cầu) | Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15; NĐ 356/2025 | Thời hạn lưu mặc định 730 ngày cần người có chuyên môn xác nhận |
| Thông tin liên hệ, kênh hỗ trợ công bố trên website TMĐT | NĐ 52/2013 sửa bởi NĐ 85/2021 (thay bởi Luật TMĐT 122/2025 và văn bản hướng dẫn) | Cần đối chiếu văn bản đang hiệu lực |

## Agent làm bước này tự quyết

Thuật toán cắt trích dẫn thư; danh sách loại mặc định; có tách `support_teams` hay dùng nhóm của
`organizations`; mặc định SLA (khuyến nghị phản hồi đầu 8 giờ làm việc, giải quyết 3 ngày làm việc). Ghi
vào `docs/DECISIONS.md`.
