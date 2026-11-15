# Bước 58 — Tự động hoá quy trình (`automations`)

> Lộ trình: `docs/ROADMAP.md` bước 58 · Quyết định: D-220, D-222, D-229 (cảnh báo bất thường) · Đặc tả liên quan: `docs/steps/51-ui-contracts.md`
> (P12 lệnh chờ duyệt, P15 job API), `docs/steps/40-core-capabilities.md` (MCP), `docs/steps/54-authorization.md`

## Mục tiêu

Chủ cửa hàng (và AI agent) tự đặt luật "khi … thì …" mà không viết code, như Shopify Flow: "đơn trên 5 triệu
thì gắn nhãn VIP và báo quản lý", "tồn dưới 10 thì tạo nháp đơn nhập", "khách đánh giá 1 sao thì mở phiếu hỗ
trợ". Luật chạy **bên trong** Autoxide, mỗi hành động gọi đúng tool MCP đã có — không có đường thứ hai làm
được điều REST / MCP không làm được (luật kiến trúc 8).

## Bắt đầu khi

- Bước 51 P12 (lệnh chờ duyệt) và P15 (job API); bước 34 (quyền theo nút); bước 54 P4 (key chi tiết) nếu
  đã có. Mọi tool MCP của module đang có dùng được ngay.

## Thiết kế

### Luật

`automations_rules`: `id`, `tenant_id`, `org_id` (phạm vi), `name`, `trigger`, `conditions` JSON, `actions`
JSON, `enabled`, `run_as` (người tạo luật — hành động chạy **với quyền hiện tại** của người đó; người đó mất
quyền thì luật tự tắt, báo cho quản trị), `version`, `created_by`, `updated_at`.

- **Kích hoạt (`trigger`)**: một loại event đã khai trong sổ event (vd `orders.order_placed`), hoặc lịch
  (`cron` đơn giản: mỗi giờ / ngày / tuần, theo múi giờ của nút), hoặc gọi tay / từ tool MCP, hoặc **số liệu bất
  thường** (dưới).
- **Cảnh báo bất thường (D-229)**: kích hoạt `metric` đọc một số liệu tóm tắt module đã khai (`SummaryMetric`, bước
  51 P4 — vd doanh thu theo ngày, số đơn, tỉ lệ đơn COD giao thất bại, số ngày còn đủ hàng của biến thể theo tốc
  độ bán 14 ngày của `inventory`) theo lịch, so với **ngưỡng cố định** hay **mức nền** (trung bình cùng thứ trong
  4 tuần trước) bằng toán tử `drop_pct` / `rise_pct` / `below` / `above`. Thống kê đơn giản, tất định, giải thích
  được ("doanh thu hôm nay 12 triệu, thấp hơn 45% so với trung bình 4 thứ Hai trước: 21,8 triệu"); không mô hình
  học máy. Mỗi luật một lần báo mỗi kỳ (khoá theo khe thời gian). Mẫu luật dựng sẵn: doanh thu tụt, sắp hết hàng
  theo tốc độ bán, tỉ lệ bom hàng tăng, phiếu hỗ trợ quá hạn tăng — hành động mặc định là thông báo cho người tạo
  luật và đưa vào hộp việc.
- **Điều kiện**: cây `all` / `any` / `not` trên trường của payload event (đường dẫn JSON), toán tử cố định
  (`eq`, `ne`, `gt`, `gte`, `lt`, `lte`, `in`, `contains`, `exists`); kiểu kiểm theo JSON Schema payload mà
  module khai cho event (thêm `EventInfo::payload_schema`, mặc định không có → chỉ so chuỗi). Không biểu
  thức tự do, không mã — kiểm được lúc lưu, giải thích được.
- **Hành động**: danh sách tuần tự, mỗi bước là một tool MCP (`tool`, `arguments` có thể lấy giá trị từ
  payload bằng `{{ event.order.id }}` qua bộ dựng mẫu của kernel — D-206), cộng vài hành động có sẵn của module
  này: `wait` (tối đa 30 ngày, bằng job hẹn giờ), `stop_if`.
- Hành động "tặng voucher" (D-222) gọi tool `promotions.grant_voucher` (`source = automation`, `source_ref =
  automation:<run_id>:<step>`) — chạy lại không cấp thêm; vẫn kiểm quyền của `run_as`.
- Lưu luật thì kiểm: event / tool có thật, người tạo có permission của **mọi** tool trong luật, tham số khớp
  JSON Schema đầu vào của tool (có thể kiểm sau khi điền mẫu bằng dữ liệu mẫu). Lỗi gom một lần (D-083).

### Chạy

- Nghe mọi event được dùng làm kích hoạt (`Subscription` theo tên, chịu nhận lại — khoá chống chạy trùng
  `(rule_id, event_id)`), xếp job `automations.run` (payload chỉ id) — không chạy trong transaction của event.
- Mỗi lần chạy ghi `automations_runs` (`rule_id`, `rule_version`, `event_id`, `status`
  `running/waiting/succeeded/failed/awaiting_approval/skipped`, `steps` JSON: từng hành động, thời gian, kết quả
  rút gọn, lỗi `code`), giữ `AUTOXIDE_AUTOMATIONS_RUN_RETENTION_DAYS` (mặc định 90).
- Hành động là lệnh rủi ro (tool `destructive`, `money`, hay đánh dấu `requires_approval_for_agent`) thì tạo
  yêu cầu duyệt (P12) và chờ; người duyệt thấy luật nào, event nào sinh ra.
- Chống vòng lặp: hành động do luật gây ra mang `Actor.automation = rule_id`; event sinh từ đó không kích hoạt
  lại cùng luật, và độ sâu chuỗi luật tối đa 3. Giới hạn số lần chạy mỗi luật mỗi giờ (cấu hình, mặc định 1000);
  vượt thì tạm dừng luật và báo.
- `Idempotency-Key` của mỗi hành động = `automation:<run_id>:<step>` — chạy lại sau lỗi không làm hai lần.

### API, MCP

- REST `/api/v1/automations`: `GET/POST /rules`, `GET/PATCH/DELETE /rules/{id}` (`If-Match`), `POST
  /rules/{id}/test {event_sample}` (chạy thử khô: tính điều kiện, điền mẫu, kiểm quyền, **không** gọi tool),
  `POST /rules/{id}/run` (chạy tay), `GET /runs?rule_id&status`, `GET /runs/{id}`, `POST /runs/{id}/retry`.
  `GET /catalog` liệt kê kích hoạt (event + schema) và hành động (tool + schema) người gọi dùng được.
- Mẫu luật dựng sẵn do module khai (`Module::automation_templates`, mặc định rỗng).
- MCP: `list_rules`, `get_rule`, `test_rule` (`read_only`), `create_rule`, `update_rule` — tạo / sửa luật
  bằng key `agent` thì luật ở trạng thái **chờ duyệt** tới khi người có quyền bật (P12). AI gợi ý luật từ câu
  lời thường chỉ là bản nháp (D-109).
- Permission: `automations.read`, `automations.manage`, `automations.run`. Tính năng gói `automations`, hạn
  mức số luật và số lần chạy mỗi tháng (bước 35).
- Event: `automations.rule_changed`, `automations.run_failed`, `automations.rule_paused`.
- Mã lỗi: `automations.trigger_unknown`, `automations.tool_unknown`, `automations.permission_missing`
  (liệt kê tool thiếu quyền), `automations.arguments_invalid`, `automations.loop_detected`,
  `automations.rule_paused` (409).

### Bảng dữ liệu (có `tenant_id`)

`automations_rules`, `automations_runs`, `automations_run_keys` (khoá chống chạy trùng `(tenant_id, rule_id,
event_id)`), cùng `automations_events`, `automations_jobs`.

## Test bắt buộc (cả hai database)

`luu_luat_kiem_quyen_moi_tool`, `nguoi_tao_mat_quyen_thi_luat_tu_tat`, `nhan_lai_event_khong_chay_hai_lan`,
`hanh_dong_rui_ro_cho_duyet`, `luat_khong_tu_kich_hoat_lai_minh`, `chay_lai_sau_loi_khong_lam_hai_lan`,
`chay_thu_kho_khong_goi_tool`, `wait_hen_gio_roi_chay_tiep`, `vuot_so_lan_chay_tam_dung_luat`,
`luat_cua_nut_khac_khong_thay`, `agent_tao_luat_thi_cho_duyet`, `so_lieu_tut_so_voi_muc_nen_thi_bao`,
`mot_ky_chi_bao_mot_lan`, `so_lieu_thieu_du_lieu_nen_khong_bao_sai`.

## Chỗ dễ sai

- Cho hành động làm điều tool MCP không làm được — luôn đi qua tool, cùng permission.
- Chạy với quyền của "hệ thống" thay vì người tạo luật — leo thang quyền.
- Ngôn ngữ điều kiện tự do — không kiểm được, không chạy giống nhau, khó giải thích.
- Quên chống vòng lặp giữa hai luật.

## Agent làm bước này tự quyết

Cú pháp lịch; giới hạn độ dài luật; mẫu luật mặc định cho từng mô hình kinh doanh. Ghi vào `docs/DECISIONS.md`.
