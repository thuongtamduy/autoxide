# Đặt lịch dịch vụ (module `bookings`)

Module `bookings` nhận đặt lịch cho dịch vụ: spa, salon, phòng khám, studio, lớp học, sân thể thao. Khách chọn
dịch vụ, nhân viên (hay ai còn trống) và giờ trống; cửa hàng quản lý nhân viên, phòng và thiết bị, giờ làm việc, ngày
nghỉ, nhắc lịch, huỷ và vắng mặt theo chính sách của mình, và có thể giữ chỗ bằng tiền cọc trả qua đối tác thanh toán
có giấy phép. Không cần các module bán hàng.

English: [`BOOKINGS.md`](BOOKINGS.md).

## Permission

| Permission                 | Cho phép                                                                           |
| -------------------------- | ---------------------------------------------------------------------------------- |
| `bookings.services.manage` | Dịch vụ, nhân viên, phòng, thiết bị, lịch tuần, ngày nghỉ và ca thêm                |
| `bookings.manage`          | Mọi lịch của đơn vị: đặt hộ khách, xác nhận, check-in, vắng mặt, huỷ, đổi giờ       |
| `bookings.read`            | Xem mọi lịch của đơn vị, không sửa được                                            |
| `bookings.staff`           | Lịch của chính mình (tài nguyên gắn với tài khoản): check-in, xong, vắng mặt        |

Cả bốn theo đơn vị tổ chức (cửa hàng, chi nhánh). Khách không cần permission: người dùng đã đăng nhập tự đặt cho mình
và xem lịch của mình ở `GET /api/v1/bookings/me/bookings`. Khách vãng lai đặt bằng mã gửi tới điện thoại.

## Thiết lập

1. **Múi giờ.** Giờ làm việc theo giờ địa phương của đơn vị: đặt `organizations.timezone` (vd `Asia/Ho_Chi_Minh`) ở
   đơn vị hay đơn vị cha. Chưa đặt thì UTC.
2. **Nhân viên, phòng, thiết bị** — `POST /api/v1/bookings/resources` với `kind` là `staff`, `room` hay `equipment`.
   Gắn nhân viên với tài khoản (`user_id`) để họ thấy lịch của mình.
3. **Lịch tuần** — `PUT /api/v1/bookings/resources/{id}/schedule` với các khung như
   `{"weekday": 1, "start_minute": 540, "end_minute": 1020}` (thứ Hai 09:00–17:00 giờ địa phương). Nhân viên không có
   lịch thì không bao giờ trống; phòng và thiết bị không có lịch thì luôn mở.
4. **Ngày nghỉ và ca thêm** — `POST /api/v1/bookings/exceptions` với `kind` là `closed` (ngày lễ, nghỉ phép) hay `open`
   (ca thêm), `starts_at`, `ends_at` (UTC).
5. **Dịch vụ** — `POST /api/v1/bookings/services`: `duration_minutes`, `buffer_before_minutes` (chuẩn bị) và
   `buffer_after_minutes` (dọn dẹp), `price`, `deposit` (`none`, `percent` theo giá, hay `amount` cố định),
   `capacity` (1 cho cắt tóc, nhiều hơn cho lớp học), `performers` (ai làm được) và `required` (phòng, máy mà lịch
   nào cũng cần).

Thời lượng và thời gian đệm theo bước 5 phút.

## Chính sách theo đơn vị

Cài đặt của cây tổ chức, kế thừa xuống dưới:

| Cài đặt                                | Mặc định | Nghĩa                                                                 |
| -------------------------------------- | -------- | --------------------------------------------------------------------- |
| `bookings.free_cancel_hours`           | 24       | Huỷ trước giờ bắt đầu ít nhất chừng này giờ: hoàn đủ cọc               |
| `bookings.late_cancel_retain_percent`  | 100      | Phần trăm cọc giữ lại khi huỷ muộn hơn                                 |
| `bookings.no_show_retain_percent`      | 100      | Phần trăm cọc giữ lại khi khách vắng mặt                               |
| `bookings.require_approval`            | false    | Lịch không cọc chờ cửa hàng xác nhận                                   |
| `bookings.hold_minutes`                | 10       | Số phút giữ chỗ trong lúc khách trả cọc                                |

Câu trả lời giờ trống nhắc lại các điều kiện này, để cửa hàng hiện giá, tiền cọc và điều kiện huỷ trước khi khách đặt,
như luật bảo vệ người tiêu dùng yêu cầu.

## Giờ trống

`GET /api/v1/bookings/public/availability?service_id=…&from=…&to=…` (không cần đăng nhập; `resource_id` và `org_id`
không bắt buộc; tối đa 31 ngày một lần) liệt kê giờ bắt đầu còn trống mỗi 15 phút giờ địa phương: lịch tuần cộng ca
thêm, trừ ngày nghỉ, trừ những gì đã đặt kèm thời gian đệm. Mỗi giờ có `starts_at` (UTC), `local_start` (kèm độ lệch
múi giờ của đơn vị) và `seats_left`. Câu trả lời được cache 15 giây, nên giờ đã hiện có thể vừa bị người khác đặt.

## Đặt lịch

- **Khách** — `POST /api/v1/bookings/public/bookings` với `service_id`, `starts_at` (lấy từ giờ trống), `resource_id`
  (không nêu: nhân viên đầu tiên còn trống), `customer` (`name`, `phone`, `email`).
  - Không cọc: `confirmed` ngay, hay `pending` khi đơn vị duyệt lịch (`POST /bookings/{id}/confirm`).
  - Có cọc: `held` trong `bookings.hold_minutes` phút; câu trả lời có `checkout` (`checkout_url`, `qr_payload`) của đối
    tác thanh toán. Đối tác báo đã thu thì lịch thành `confirmed`. Lịch giữ mà chưa trả được trả chỗ mỗi phút, giờ đó
    trống lại; tiền về sau khi đã hết giữ thì được hoàn đủ.
- **Khách vãng lai** — trước hết `POST /api/v1/bookings/public/phone-codes` với `phone`; gửi `challenge_id` và 6 chữ
  số nhận qua SMS làm `phone_code` trong lệnh đặt. Mỗi mã dùng một lần, trong 10 phút. Khách vãng lai huỷ bằng mã mới ở
  `POST /api/v1/bookings/public/bookings/{id}/cancel`.
- **Cửa hàng** — `POST /api/v1/bookings/bookings` (khách gọi điện, khách đến thẳng): xác nhận ngay, không cọc.

Hai người không bao giờ được cùng một giờ: mỗi lịch giữ các ô 5 phút của từng nhân viên, phòng, máy nó dùng, dưới một
khoá duy nhất, trong một transaction. Request thứ hai nhận `409 bookings.slot_taken` — như nhau trên SQLite và
PostgreSQL. Lớp học đếm chỗ bằng một câu cập nhật có điều kiện và báo `409 bookings.capacity_full` khi đủ người.

## Sau khi đặt

| Việc | Ai | Endpoint |
|---|---|---|
| Check-in, xong | cửa hàng, nhân viên của lịch | `POST /api/v1/bookings/bookings/{id}/check-in`, `/complete` |
| Vắng mặt (sau giờ bắt đầu) | cửa hàng, nhân viên của lịch | `POST /api/v1/bookings/bookings/{id}/no-show` — giữ `bookings.no_show_retain_percent` cọc |
| Huỷ | cửa hàng (lúc nào cũng được, hoàn đủ), khách (tới giờ bắt đầu, theo chính sách) | `POST /api/v1/bookings/bookings/{id}/cancel` |
| Đổi giờ | cửa hàng (lúc nào cũng được), khách (trong hạn huỷ miễn phí) | `POST /api/v1/bookings/bookings/{id}/reschedule` |
| Tệp lịch | cửa hàng, nhân viên, khách | `GET /api/v1/bookings/bookings/{id}/calendar.ics`, `GET /api/v1/bookings/me/bookings/{id}/calendar.ics` |

Tiền hoàn đi qua đối tác thanh toán, không bao giờ từ tiền của nền tảng (`ledger`), với mã tham chiếu cố định: hoàn
lại lần nữa không trả hai lần.

## Nhắc lịch

Khi có module `notifications`, khách nhận tin nhắc giao dịch (`bookings.reminder`) 24 giờ và 2 giờ trước giờ bắt đầu,
trên các kênh họ có (hộp thư trong ứng dụng, email, SMS, Zalo ZNS, thông báo đẩy). Lịch đặt trong vòng 24 giờ chỉ nhận
lần 2 giờ. Một lần nhắc không bao giờ gửi hai lần, kể cả khi job chạy lại; lịch đổi giờ được nhắc lại theo giờ mới.
Tin nhắc có link tới tệp lịch của lịch đặt.

## Cho AI agent (MCP)

| Tool | Permission |
|---|---|
| `bookings.availability` | — (chỉ đọc) |
| `bookings.list_bookings` | `bookings.read` hay `bookings.manage` (chỉ đọc) |
| `bookings.create_booking` | `bookings.manage` |
| `bookings.reschedule_booking` | `bookings.manage` |
| `bookings.cancel_booking` | `bookings.manage` |

## Lỗi

| Mã | Trạng thái | Nghĩa |
|---|---|---|
| `bookings.slot_taken` | 409 | Giờ vừa bị đặt; hỏi lại giờ trống |
| `bookings.capacity_full` | 409 | Lớp đã đủ người |
| `bookings.outside_hours` | 422 | Ngoài giờ làm của nhân viên hay phòng |
| `bookings.too_late_to_cancel` | 409 | Lịch đã bắt đầu |
| `bookings.too_late_to_change` | 409 | Khách chỉ đổi giờ được trong hạn huỷ miễn phí |
| `bookings.invalid_status` | 409 | Không làm được ở trạng thái hiện tại của lịch |
| `bookings.code_invalid` | 422 | Mã của khách vãng lai sai, đã dùng hay hết hạn |
| `bookings.payments_unavailable` | 503 | Dịch vụ cần cọc nhưng chưa thiết lập thanh toán |

## Ghi chú pháp lý

Cần đối chiếu văn bản gốc / người có chuyên môn pháp lý: luật bảo vệ quyền lợi người tiêu dùng (Luật 19/2023/QH15, NĐ
55/2024) yêu cầu hiện giá, tiền cọc và điều kiện huỷ trước khi đặt; tiền cọc đi qua tổ chức cung ứng dịch vụ thanh toán
có giấy phép (NĐ 52/2024); phòng khám có quy định riêng về hồ sơ bệnh án (Luật 15/2023/QH15) — `bookings` không lưu
thông tin sức khoẻ, ghi chú chỉ là chữ ngắn.
