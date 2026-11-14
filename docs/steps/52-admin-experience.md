# Bước 52 — Trải nghiệm quản trị nâng cao

> Lộ trình: `docs/ROADMAP.md` bước 52 · Thiết kế: `docs/research/ui-design.md` mục 5 · Quyết định:
> D-109 (AI chỉ thành bản nháp), D-200, D-201, D-232 (quản trị trên điện thoại)
> Đã gộp các quyết định: D-202, D-224 và đối chiếu thiết kế 2 (A-c10, A-c12) (D-228).
> Mã `A-…` trỏ tới `docs/research/ui-trace/` (nguồn: `docs/prompts/ui-trace-roadmap.md`); `ActorKind` đủ bảy
> giá trị có nguồn `docs/prompts/ui-phase2-api.md`, màn tương ứng ở `docs/research/ui-traceability.md`.

## Mục tiêu

Những năng lực quản trị mà các sản phẩm hàng đầu có và nhà phát triển, nhà vận hành, AI agent dùng hằng
ngày, sau khi bước 51 đã có menu, hộp việc, tìm kiếm, view, thao tác hàng loạt. Nhân viên quản trị được trên
điện thoại: menu có gợi ý cho thanh đáy, và bản tóm tắt cuối ngày tự gửi (D-232).

## Bắt đầu khi

SAU bước 51. Mục AI cần module `ai` (bước 18) và P7 của bước 51 (schema bộ lọc). Mục 6 cần P1, P2, P4 của bước
51 và `notifications` (bước 37); phần cảnh báo bất thường của bản tóm tắt chỉ có khi bước 58 đã làm (D-232).

## Thiết kế

### 1. Bàn kỹ thuật

- Nhật ký request theo khách (bảng `kernel_request_log`, cấp khách hàng, có hạn lưu là cấu hình, mặc định
  14 ngày, xoá bằng job): thời điểm, API key hay người dùng (`api_key_id`), `actor_kind`, phương thức,
  **đường dẫn mẫu** (không có ID thật), status, `code` lỗi, thời gian xử lý, `Idempotency-Key`, `approval_id`,
  `request_id`, `trace_id` (nối với OTLP, A-c10). Không lưu thân request, header xác thực, query có dữ liệu cá
  nhân. Ghi bất đồng bộ, không làm chậm request; đo trước / sau.
- `actor_kind` dùng `ActorKind` của kernel (D-224, P12 bước 51) — đủ bảy giá trị `human`, `agent`, `service`,
  `system`, `support`, `app`, `scim`, không phải tập con.
- **Nhật ký lệnh của agent (D-202).** Lệnh gọi qua MCP ghi `mcp:<tên tool>` (vd `mcp:comments.answer`) vào cột
  đường dẫn mẫu, phương thức `MCP`; **không ghi tham số** của tool (có thể chứa dữ liệu cá nhân). Ghi ở lớp
  MCP của kernel (`mcp.rs`), một dòng cho mỗi lần gọi tool, kể cả khi tool trả lỗi. Lệnh nhận
  `202 approval_required` (P12 bước 51): dòng nhật ký giữ `approval_id` để giao diện nối tới yêu cầu duyệt;
  khi lệnh được duyệt và chạy, dòng mới ghi cùng `approval_id`.
- `GET /api/v1/kernel/devtools/requests` lọc theo `actor_kind`, `api_key_id`, `path` (chính xác, vd
  `mcp:orders.refund`), `status`, khoảng thời gian; phân trang bằng `fetch_page`, index (`tenant_id`,
  `actor_kind`, `id`) và (`tenant_id`, `api_key_id`, `id`). Tool MCP `kernel.request_log`
  (`kernel.devtools.read`, `read_only`) như REST (D-202).
- Webhook: danh sách lần giao (module `webhooks` đã có bảng giao), lọc theo trạng thái, gửi lại một lần
  giao: `POST /api/v1/webhooks/deliveries/{id}/redeliver` (A-c10).
- Event theo bản ghi: `GET /api/v1/kernel/devtools/events?object=<kind>:<id>` — **đọc từ audit**, vì outbox
  bị dọn sau khi giao (A-c10).
- Dead-letter queue: chạy lại có audit.
- Permission: đọc `kernel.devtools.read`; ghi (gửi lại lần giao, chạy lại job) `kernel.devtools.manage`
  (A-c10). Tool MCP tương ứng.

### 2. Hoàn tác an toàn

- Thao tác lưu trữ / ẩn của module có `restore` tương ứng; giao diện hiện "Hoàn tác" vài giây.
- Lệnh ghi lớn (thao tác hàng loạt, nhập CSV, đổi giá hàng loạt) nhận `dry_run=true`: trả danh sách thay
  đổi sẽ xảy ra, không ghi gì, không phát event.

### 3. AI trong quản trị

- `POST /api/v1/ai/filters/parse {resource, text}` → `params` đã kiểm theo schema bộ lọc (P7 bước 51) và
  phần không hiểu. Không bao giờ tự chạy lệnh ghi.
- Trợ lý: `POST /api/v1/ai/assistant/messages {page, text}` (A-c12) chạy tool MCP với **đúng quyền của người
  gọi**, chỉ tool `read_only` khi khách chưa bật ghi; lệnh ghi chỉ ra bản nháp hay `dry_run` chờ người duyệt
  (D-109); lệnh rủi ro đi qua yêu cầu duyệt của P12 bước 51. Trả lời kèm danh sách tool đã gọi để người dùng
  kiểm.

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

### 6. Quản trị trên điện thoại (D-232)

Giao diện thanh đáy thuộc repo giao diện; core chỉ cho dữ liệu.

- **Gợi ý `mobile` trên sổ điều hướng** (P1 bước 51): `NavItem` thêm `mobile: MobileHint { placement, order }`,
  `placement` = `primary` (thanh đáy) / `secondary` (menu "Thêm") / `hidden` (không hiện trên điện thoại). Module
  khai cùng chỗ khai mục; không khai thì `secondary` với `order` = `order` của mục — trường có mặc định (hàm dựng
  của `NavItem`), module cũ không phải sửa (như D-038).
- `GET /api/v1/kernel/navigation?org_id&device=mobile` (`device` = `desktop` mặc định / `mobile`; giá trị khác → 422
  lỗi kiểm đầu vào): lọc permission, module bật, tính năng **như cũ**, rồi mỗi mục thêm `mobile: {placement,
  order}`, sắp theo `placement` rồi `order`, bỏ mục `hidden`. Tối đa **5** mục `primary`: lấy 5 mục `primary` có
  `order` nhỏ nhất trong số mục **hiện và `available`**, phần còn lại (kể cả mục `locked`) thành `secondary` — không
  lỗi lúc khởi động, vì tập module bật khác nhau theo khách. `device=desktop` trả đúng như trước. `ETag` gồm cả
  `device`. Tool MCP `kernel.navigation` nhận thêm `device`.
- **Tóm tắt cuối ngày** cho nhân viên — module `notifications` sở hữu (gửi tin, giữ tuỳ chọn của người nhận):
  - Người dùng tự bật (mặc định **tắt**), chỉ tài khoản `realm = staff`, theo từng khách: `GET` / `PUT
    /api/v1/notifications/me/digest {enabled, local_time, time_zone, org_id?, skip_when_empty}` (đăng nhập; không
    permission riêng). `local_time` dạng `HH:MM`; `time_zone` IANA (giao diện điền sẵn `auth_user.time_zone` của bước
    34); `org_id` thu phạm vi số liệu về một nút (bỏ = mọi nút người đó có quyền; nút ngoài phạm vi → 403
    `org_out_of_scope`); `skip_when_empty` (mặc định `true`) = không gửi khi không có gì đáng báo. Kênh (email,
    đẩy, trong ứng dụng) chọn bằng tuỳ chọn theo loại tin sẵn có của `notifications` (`notifications_preferences`).
  - Nội dung, tính **với quyền hiện tại của người nhận lúc gửi** (như `run_as` của bước 58; mất quyền thì phần đó
    biến mất, không còn membership ở khách thì tự tắt): số liệu tóm tắt P4 trong phạm vi người đó xem được (cùng
    hàm gom của `GET /kernel/summary`, kèm `definition`); số việc trong hộp việc P2 theo `severity` (`blocking`,
    `attention`, `info` — cùng hàm của `/kernel/work/counts`); cảnh báo bất thường của bước 58 đã gửi cho người đó
    trong 24 giờ qua, nếu có. Nguồn quá hạn thời gian hay lỗi thì ghi "chưa lấy được", như `incomplete` của P2.
    "Không có gì đáng báo" = không có việc `blocking` / `attention` và không có cảnh báo bất thường.
  - Cảnh báo bất thường lấy từ tin `notifications` đã gửi, không gọi `automations` (đăng ký sau): `NotificationKind`
    thêm `in_digest: bool` (mặc định `false`); bước 58 đặt `true` cho loại tin cảnh báo bất thường.
  - Là tin vận hành cho nhân viên, không phải quảng cáo: loại `notifications.daily_digest`, `Category::Transactional`
    (không qua sổ đồng ý quảng cáo, không chèn nội dung quảng cáo), kèm link tắt bản tóm tắt. Căn cứ: NĐ
    91/2020/NĐ-CP (chống tin nhắn rác) áp cho tin quảng cáo, không cho tin giao dịch — cần đối chiếu văn bản gốc.
  - **Một lần mỗi ngày, chịu job chạy lại:** job `notifications.digest_tick` (5 phút một lần mỗi database,
    `NO_TENANT`, unique_key `digest_tick:<khe 5 phút>`) lấy dòng `enabled` có `next_run_at <= now`, mỗi dòng trong
    một transaction: xếp job `notifications.send_digest` (unique_key `<tenant>:<user>:<ngày địa phương>`) và dời
    `next_run_at` sang giờ đó của ngày địa phương kế tiếp. `send_digest` chỉ đọc rồi gọi `Notifier::notify` với
    `dedupe_key = digest:<user>:<ngày địa phương>` (unique của `notifications_messages` chặn tin thứ hai), rồi ghi
    `last_sent_on`. Đổi giờ bằng `PUT` thì tính lại `next_run_at`; khoá theo ngày giữ một tin mỗi ngày.
  - Giờ địa phương tính bằng múi giờ IANA (cùng công cụ múi giờ của bước 34 / 58), không cộng độ lệch cố định:
    giờ bị bỏ qua khi đổi giờ mùa hè thì gửi ở phút hợp lệ kế tiếp; giờ lặp lại thì gửi ở lần đầu.

| Bảng (`notifications`) | Cột | Khoá, index |
|---|---|---|
| `notifications_digest_settings` | `tenant_id`, `user_id`, `enabled` (mặc định `false`), `local_time`, `time_zone`, `org_id` NULL, `skip_when_empty` (mặc định `true`), `next_run_at` NULL, `last_sent_on` (ngày địa phương) NULL, `updated_at` | PK (`tenant_id`, `user_id`); sweep index (`next_run_at`) |

Mã lỗi (bản dịch ở `locales/vi.json` của `notifications`):

| Mã | HTTP | Câu tiếng Việt |
|---|---|---|
| `notifications.digest_staff_only` | 403 | Tóm tắt cuối ngày chỉ dành cho tài khoản nhân viên. |
| `notifications.digest_invalid_time` | 422 | Giờ gửi phải dạng `HH:MM` và múi giờ phải là tên IANA hợp lệ (vd `Asia/Ho_Chi_Minh`). |

## Test bắt buộc

Trên cả hai database:

- `request_log_khong_luu_than_va_token`, `request_log_het_han_bi_xoa`, `dry_run_khong_ghi_khong_phat_event`,
  `restore_tra_lai_trang_thai_truoc`, `ai_filter_parse_chi_tra_tham_so_hop_le` (dùng `FakeTextModel`),
  `seller_badge_tinh_tu_so_lieu_that`.
- Nhật ký lệnh của agent (D-202): `lenh_mcp_ghi_dung_ten_tool`, `tham_so_tool_khong_lot_vao_nhat_ky`,
  `loc_nhat_ky_theo_actor_kind_va_key`, `lenh_cho_duyet_giu_approval_id`.
- Quản trị trên điện thoại (D-232): `dieu_huong_mobile_toi_da_5_muc_primary`,
  `dieu_huong_mobile_van_loc_permission_va_tinh_nang`, `dieu_huong_mobile_bo_muc_hidden`,
  `dieu_huong_desktop_khong_doi`, `tom_tat_mac_dinh_tat`, `tom_tat_chi_nhan_vien_bat_duoc`,
  `tom_tat_chi_gom_so_lieu_trong_pham_vi_nguoi_nhan`, `tom_tat_dem_viec_theo_muc_gap`,
  `tom_tat_khong_gui_khi_khong_co_gi_dang_bao`, `tom_tat_gui_dung_gio_theo_mui_gio_nguoi_dung`,
  `tom_tat_chay_lai_job_khong_gui_hai_lan`; đồng thời: `hai_job_gui_tom_tat_cung_ngay_chi_mot_tin` (hai worker
  chạy `send_digest` cùng người, cùng ngày: đúng một tin mỗi kênh).

## Agent làm bước này tự quyết

Hạn lưu mặc định của nhật ký request; ghi nhật ký bằng outbox hay hàng đợi trong bộ nhớ có giới hạn (đo
trước); danh sách chỉ số sức khoẻ người bán đầu tiên; mục điều hướng nào của module đang có là `primary` trên
điện thoại (D-232). Ghi vào `docs/DECISIONS.md`.
