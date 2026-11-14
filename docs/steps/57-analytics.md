# Bước 57 — Phân tích truy cập và hành vi (`analytics`)

> Lộ trình: `docs/ROADMAP.md` bước 57 · Quyết định: D-220 · Đặc tả liên quan: `docs/steps/37-compliance.md`
> (sổ đồng ý, căn cứ xử lý), `docs/steps/41-catalog.md` (bán chạy, thịnh hành), `docs/steps/51-ui-contracts.md`
> (số liệu tóm tắt P4)

## Mục tiêu

Chủ website biết: bao nhiêu người xem, xem trang nào, sản phẩm nào, đến từ đâu (tìm kiếm, mạng xã hội, quảng
cáo có UTM), và phễu "xem sản phẩm → thêm giỏ → bắt đầu thanh toán → mua" rơi ở bước nào — ngay trong
Autoxide, **không gửi dữ liệu người đọc cho bên thứ ba**, mặc định **không cookie**, chỉ giữ số đã gộp.
Theo dõi từng người (đường đi của một khách) chỉ khi người đó đồng ý.

## Bắt đầu khi

- Bước 37 xong (sổ đồng ý `ConsentLedger`, sổ căn cứ xử lý). Bước 33 (khách), 34 (nút, website theo tên miền).
- Song song được với 41–49. Module khác khai loại sự kiện của mình (vd `catalog` khai `product_viewed`).

## Phạm vi

Làm: thu sự kiện từ website (endpoint công khai) và từ máy chủ (event của module); gộp theo giờ / ngày; phiên
ẩn danh không cookie; nguồn truy cập và UTM; phễu; số liệu cho trang tổng quan; API báo cáo; theo dõi theo
người khi có đồng ý; xoá theo hạn lưu.

Không làm: bản đồ nhiệt, ghi lại phiên (session replay), dấu vân tay trình duyệt (fingerprinting — **cấm**),
gửi dữ liệu cho mạng quảng cáo, thử nghiệm A/B (để sau).

## Thiết kế

### Thu sự kiện

- `POST /api/v1/analytics/public/events` (công khai, theo website suy từ tên miền; gom tối đa 20 sự kiện mỗi
  lần; `sendBeacon` được): `{type, path, referrer, utm: {source, medium, campaign, term, content}, object:
  "<kind>:<id>"?, props?}`. Kiểu sự kiện do module khai (`Module::analytics_events` → `{key, props_schema}`,
  mặc định rỗng — D-038); sự kiện lạ hay `props` sai schema bị bỏ, không lỗi (chống rác, không làm hỏng trang).
- Sự kiện phía máy chủ (đơn đặt, thanh toán xong, đăng ký) lấy từ event sẵn có của module qua đăng ký nhận —
  không tin số đơn do trình duyệt gửi.
- Chống bot: bỏ user agent bot đã biết, giới hạn tần suất theo IP băm (`throttle`), bỏ sự kiện không có tên
  miền khớp website.

### Phiên ẩn danh không cookie (mặc định)

- Khoá phiên = HMAC(khoá ngày xoay mỗi 24 giờ, `website_id` + IP băm theo khối mạng + user agent rút gọn).
  Khoá ngày sinh từ `SecretKey::derive("analytics.day.<ngày>")` và **bị xoá sau ngày đó** → không nối được hai
  ngày, không truy ngược ra IP. Không lưu IP, không lưu user agent đầy đủ.
- Đếm khách duy nhất theo ngày; không có "khách quay lại" khi không có đồng ý.

### Theo dõi theo người (chỉ khi đồng ý)

- Người dùng đã đăng nhập và đã đồng ý mục đích `analytics.personal` (sổ đồng ý của bước 37): sự kiện gắn
  `user_id`, giữ đường đi để `customers` (bước 47) dựng "sản phẩm vừa xem", phân khúc theo hành vi. Rút đồng ý
  → ngừng ngay, xoá phần đã gắn người (giữ số gộp).

### Gộp và báo cáo

- Bảng thô `analytics_events` giữ ngắn (`AUTOXIDE_ANALYTICS_RAW_RETENTION_DAYS`, mặc định 7), job gộp mỗi giờ
  vào `analytics_daily` (theo website, ngày, trang, nguồn, UTM, loại sự kiện, đối tượng) và `analytics_funnel_daily`.
  Báo cáo **chỉ đọc bảng gộp** (nhanh trên bảng lớn, D-190), giữ `AUTOXIDE_ANALYTICS_RETENTION_DAYS` (mặc định 790
  — 2 năm + 2 tháng để so cùng kỳ).
- `GET /api/v1/analytics/overview?site&from&to` (khách, lượt xem, tỉ lệ thoát, nguồn hàng đầu),
  `/pages`, `/sources`, `/campaigns`, `/objects?kind=catalog.product` (xem nhiều nhất), `/funnels/{key}`
  (module khai phễu: `Module::funnels`, vd `orders.checkout`).
- Số liệu tóm tắt cho trang tổng quan (P4 bước 51) và cột "thịnh hành" cho `catalog` (bước 41) qua `api`.
- Mọi API theo phạm vi nút (website của nút, bước 34); permission `analytics.read`.

### Bảng dữ liệu (mọi bảng có `tenant_id`)

| Bảng | Cột chính |
|---|---|
| `analytics_events` | `id`, `tenant_id`, `site_org_id`, `occurred_at`, `type`, `session_key`, `user_id` NULL (chỉ khi đồng ý), `path`, `referrer_host`, `utm_*`, `object`, `props` JSON — index (`tenant_id`, `occurred_at`) |
| `analytics_daily` | `tenant_id`, `site_org_id`, `day`, `dimension` (`page`/`source`/`campaign`/`object`/`type`), `value`, `views`, `visitors`, `events` — khoá (`tenant_id`, `site_org_id`, `day`, `dimension`, `value`) |
| `analytics_funnel_daily` | `tenant_id`, `site_org_id`, `day`, `funnel`, `step`, `sessions` |

### Tool MCP, event, cấu hình, permission, lỗi

- MCP: `analytics.overview`, `analytics.top_pages`, `analytics.top_objects`, `analytics.funnel` (`read_only`).
- Không phát event cho từng lượt xem. Event `analytics.daily_rolled_up` cho `reporting` (bước 38) nếu cần.
- Cấu hình: hai hạn lưu ở trên, `AUTOXIDE_ANALYTICS_ENABLED` (mặc định `true`), `AUTOXIDE_ANALYTICS_EVENTS_PER_MINUTE`
  (theo IP băm, mặc định 120).
- Permission `analytics.read`, `analytics.manage` (phễu, loại trừ IP nội bộ). Tính năng gói `analytics`.
- Mã lỗi: `analytics.site_unknown` (404), `analytics.range_too_long` (422, quá 400 ngày một lần).

## Test bắt buộc (cả hai database)

`khong_luu_ip_va_user_agent_day_du`, `khoa_phien_khong_noi_duoc_hai_ngay`, `su_kien_la_bi_bo_khong_loi`,
`bot_bi_loai`, `gop_theo_ngay_khop_su_kien_tho`, `bao_cao_chi_doc_bang_gop`, `theo_doi_theo_nguoi_chi_khi_dong_y`,
`rut_dong_y_xoa_phan_gan_nguoi`, `website_cua_nut_khac_khong_thay_so_lieu`, `het_han_thi_xoa_su_kien_tho`.

## Căn cứ pháp lý

| Nội dung | Văn bản | Ghi chú |
|---|---|---|
| Thu thập dữ liệu hành vi gắn với người cần căn cứ / đồng ý; tối thiểu hoá | Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15; NĐ 356/2025 | **Cần đối chiếu văn bản gốc**: số gộp ẩn danh có là dữ liệu cá nhân không, có cần banner đồng ý khi không cookie không |
| Quảng cáo và thư quảng cáo dựa trên hành vi | NĐ 91/2020/NĐ-CP | Phân khúc theo hành vi chỉ dùng cho tin quảng cáo khi đã đồng ý (bước 37) |

## Agent làm bước này tự quyết

Danh sách bot; cách cắt user agent rút gọn; mặc định tỉ lệ thoát tính thế nào; có dùng `reporting` (38) để gộp
xuyên database không. Ghi vào `docs/DECISIONS.md`.
