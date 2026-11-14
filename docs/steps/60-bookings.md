# Bước 60 — Đặt lịch dịch vụ (`bookings`)

> Lộ trình: `docs/ROADMAP.md` bước 60 · Quyết định: D-221 · Đặc tả liên quan: 34 (nút, giờ mở cửa), 36 (thanh
> toán, cọc qua đối tác), 37 (thông báo)

## Mục tiêu

Spa, salon, phòng khám, studio, lớp học, sân thể thao: khách chọn dịch vụ, nhân viên, giờ còn trống và đặt
chỗ; cửa hàng quản lý lịch nhân viên, phòng / thiết bị, giờ nghỉ, nhắc lịch, huỷ và đổi giờ theo chính sách, giữ
chỗ bằng tiền cọc qua đối tác có giấy phép. Không cần phần bán hàng (41–49) để dùng.

## Bắt đầu khi

Bước 34 (nút, giờ mở cửa, múi giờ) và 36 (thanh toán qua `ledger`) xong; 37 cho thông báo. Song song với 41–49.

## Thiết kế

- **Dịch vụ** (`bookings_services`: tên đa ngôn ngữ, thời lượng, thời gian đệm trước / sau, giá + tiền tệ,
  `deposit` (không / phần trăm / số tiền), sức chứa (1 cho cắt tóc, N cho lớp học), nhân viên làm được, tài
  nguyên cần (phòng, máy)).
- **Lịch làm việc:** `bookings_resources` (nhân viên — `user_id` của bước 34 — hay phòng / thiết bị),
  `bookings_schedules` (khung giờ lặp theo thứ, theo múi giờ của nút), `bookings_exceptions` (nghỉ phép, ngày lễ,
  ca thêm).
- **Tính giờ trống:** hàm thuần trong Rust (lịch − ngoại lệ − lịch đã đặt − đệm), kết quả theo múi giờ của nút,
  lưu UTC. `GET /public/bookings/availability?service_id&org_id&from&to&resource_id?` có cache ngắn.
- **Đặt chỗ không chồng giờ:** bảng `bookings_slots` (một dòng cho mỗi tài nguyên × khoảng 5 phút bị chiếm, khoá
  duy nhất (`tenant_id`, `resource_id`, `slot_start`)); đặt = chèn mọi ô trong một transaction, trùng khoá thì
  báo `bookings.slot_taken` — chạy như nhau trên hai database, không cần ràng buộc loại trừ chỉ Postgres có.
  Lớp học sức chứa N: đếm bằng `UPDATE … SET booked = booked + 1 WHERE booked < capacity`.
- **Giữ tạm và cọc:** giữ chỗ `held` 10 phút trong lúc thanh toán cọc qua `ledger` (D-187); hết hạn trả chỗ (job).
  Không cọc → `confirmed` ngay hay chờ cửa hàng duyệt (chính sách).
- **Huỷ / đổi / vắng mặt:** chính sách theo nút (hạn huỷ miễn phí, phần cọc giữ lại khi huỷ muộn hay vắng mặt);
  hoàn cọc qua `ledger`. Cửa hàng đánh dấu `no_show`.
- **Nhắc lịch** qua `notifications` (tin giao dịch): 24 giờ và 2 giờ trước; tệp `.ics` đính kèm.
- Khách vãng lai đặt bằng số điện thoại + OTP (như tra đơn vãng lai bước 45); khách có tài khoản xem / huỷ trong
  `/me/bookings`.

### API, MCP, quyền, lỗi

- REST `/api/v1/bookings`: `services`, `resources`, `schedules`, `exceptions`, `bookings` (lọc theo ngày, nhân
  viên, trạng thái), `POST /bookings/{id}/check-in|no-show|cancel|reschedule`; công khai `availability`,
  `POST /public/bookings`; `/me/bookings`.
- MCP: `availability`, `list_bookings` (`read_only`), `create_booking`, `reschedule_booking`, `cancel_booking`.
- Permission: `bookings.manage`, `bookings.staff` (nhân viên xem lịch của mình), `bookings.services.manage`.
- Event: `bookings.booking_created`, `…_confirmed`, `…_cancelled`, `…_rescheduled`, `…_no_show`, `…_completed`.
- Lỗi: `bookings.slot_taken` (409), `bookings.outside_hours` (422), `bookings.too_late_to_cancel` (409),
  `bookings.capacity_full` (409).

## Test bắt buộc (cả hai database)

`hai_nguoi_dat_cung_gio_chi_mot_thanh_cong`, `lop_hoc_khong_vuot_suc_chua`, `gio_trong_tru_nghi_va_dem`,
`giu_tam_het_han_tra_cho`, `huy_muon_giu_coc_theo_chinh_sach`, `doi_mui_gio_hien_dung_gio_dia_phuong`,
`nhac_lich_khong_gui_hai_lan`.

## Căn cứ pháp lý

| Nội dung | Văn bản | Ghi chú |
|---|---|---|
| Thông tin dịch vụ, điều kiện huỷ, tiền cọc minh bạch trước khi đặt | Luật Bảo vệ quyền lợi người tiêu dùng 19/2023/QH15; NĐ 55/2024 | Cần đối chiếu văn bản gốc |
| Tiền cọc qua đối tác thanh toán có giấy phép | NĐ 52/2024/NĐ-CP; D-187 | Core không giữ tiền |
| Dịch vụ y tế (phòng khám) có quy định riêng về hồ sơ bệnh án | Luật Khám bệnh, chữa bệnh 15/2023/QH15 | `bookings` không lưu thông tin sức khoẻ; ghi chú chỉ là chữ ngắn |

## Agent làm bước này tự quyết

Độ mịn ô giờ (5 phút); cache giờ trống; có cho chọn "nhân viên bất kỳ" không. Ghi vào DECISIONS.
