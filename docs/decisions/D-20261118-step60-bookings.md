## D-20261118-step60-bookings · Module `bookings`: ô giờ 5 phút, giờ bắt đầu theo lưới 15 phút, "nhân viên bất kỳ", cache giờ trống bằng HTTP, cọc qua `ledger`, nhắc lịch qua `notifications` — 2026-11-18 — agent quyết

Chủ đề: Thương mại

Bối cảnh: đặc tả bước 60 (`docs/steps/60-bookings.md`, D-221) để agent tự quyết độ mịn ô giờ, cách cache giờ trống và
có cho chọn "nhân viên bất kỳ" không. Khi làm còn vài chỗ đặc tả chưa nói.

Quyết định:

1. **Ô giờ 5 phút.** `bookings_slots` giữ một dòng cho mỗi tài nguyên × 5 phút bị chiếm (khoá duy nhất
   `tenant_id, resource_id, slot_start`); khoảng chiếm gồm cả thời gian đệm trước / sau, làm tròn ra ngoài theo lưới 5
   phút UTC. Một lịch 60 phút + 10 phút đệm là 14 dòng — rẻ, và chạy như nhau trên hai database.
2. **Giờ bắt đầu được chào theo lưới 15 phút giờ địa phương** của nút (`organizations.timezone`, mặc định UTC). Đặt
   giờ lệch lưới thì vẫn nhận nếu nằm trong giờ làm và còn trống — lưới chỉ để danh sách giờ trống gọn.
3. **"Nhân viên bất kỳ" được phép:** không nêu `resource_id` thì hệ thống chọn nhân viên làm được dịch vụ, còn trống,
   theo thứ tự ID (ổn định, dễ đoán). Giờ trống không nêu nhân viên thì gộp mọi nhân viên.
4. **Cache giờ trống bằng HTTP** (`Cache-Control: public, max-age=15`, `Vary: Autoxide-Tenant`), không cache trong bộ nhớ
   máy chủ: không phải xoá cache khi có người đặt, nhiều máy chủ không lệch nhau; đặt chỗ vẫn kiểm lại dưới khoá duy
   nhất nên giờ cũ trong cache không gây đặt trùng.
5. **Tài nguyên:** nhân viên (`staff`, có thể gắn `user_id` của bước 34) cần lịch làm việc mới có giờ trống; phòng /
   thiết bị không có lịch thì coi như luôn mở, có lịch thì theo lịch. Lịch là khung giờ lặp theo thứ trong tuần, giờ
   địa phương; ngoại lệ (`closed` nghỉ / `open` ca thêm) lưu theo UTC.
6. **Lớp học sức chứa N:** dòng `bookings_sessions` (dịch vụ, tài nguyên, giờ bắt đầu) chiếm ô giờ một lần khi người đầu
   tiên đặt; chỗ ngồi đếm bằng `UPDATE … SET booked = booked + n WHERE booked + n <= capacity`.
7. **Cọc qua `ledger`** (D-187): đặt có cọc → `held` (mặc định 10 phút, cài đặt theo nút `bookings.hold_minutes`), tạo
   lượt thu bằng `dyn Ledger::create_payment` sau khi transaction ghi xong, `ledger.payment_succeeded` → `confirmed`. Hết
   hạn giữ (job `bookings.expire_holds`) thì trả chỗ; tiền về sau khi đã hết hạn thì hoàn đủ. Hoàn cọc khi huỷ / vắng
   mặt đi qua job `bookings.refund_deposit` (gọi `dyn Ledger::refund` với `reference` cố định — chạy lại không hoàn
   hai lần). Không có `ledger` thì dịch vụ có cọc báo `bookings.payments_unavailable`.
8. **Chính sách huỷ theo nút** (`bookings.free_cancel_hours` mặc định 24, `bookings.late_cancel_retain_percent` và
   `bookings.no_show_retain_percent` mặc định 100, `bookings.require_approval` mặc định tắt). Khách huỷ được tới giờ
   bắt đầu (sau đó `bookings.too_late_to_cancel`); trong hạn miễn phí hoàn đủ cọc, sau hạn giữ lại phần trăm theo chính
   sách. Cửa hàng huỷ thì luôn hoàn đủ. Khách đổi giờ chỉ trong hạn huỷ miễn phí (`bookings.too_late_to_change`).
9. **Lịch do cửa hàng tạo** (`POST /bookings`, khách gọi điện, khách đến thẳng) không giữ tạm và không thu cọc: xác nhận
   ngay.
10. **Nhắc lịch** qua `dyn Notifier` (tin giao dịch `bookings.reminder`) 24 giờ và 2 giờ trước; lịch đặt trong vòng 24
    giờ chỉ nhận lần 2 giờ. Gửi trước (khoá chống trùng `bookings.reminder:<id>:<mốc>:<giờ bắt đầu>`), đánh dấu sau:
    job chạy lại không gửi lần hai, đổi giờ thì nhắc lại theo giờ mới. `notifications` chưa có tệp đính kèm, nên `.ics`
    là endpoint tải về (`/bookings/{id}/calendar.ics`, `/me/bookings/{id}/calendar.ics`) và link trong tin trỏ tới đó.
11. **Khách vãng lai** xác nhận số điện thoại bằng mã một lần (6 số, băm có pepper riêng của khách, 10 phút, tối đa 5
    lần sai, giới hạn số mã theo số điện thoại và IP) gửi qua `dyn SmsSender`, rồi đặt / huỷ bằng mã đó. Khách có tài
    khoản đặt không cần mã và xem / huỷ ở `/me/bookings`.
12. **Dịch vụ là bản ghi riêng của `bookings`** (bước 41 `catalog` chưa merge): sau này gắn sản phẩm của `catalog` thì chỉ
    giữ ID, không foreign key.

Căn cứ pháp lý — **cần đối chiếu văn bản gốc / người có chuyên môn pháp lý**: Luật Bảo vệ quyền lợi người tiêu dùng
19/2023/QH15 (hiệu lực 01/07/2024) và NĐ 55/2024/NĐ-CP — giá, tiền cọc, điều kiện huỷ phải hiện trước khi đặt (giờ
trống trả kèm chính sách); NĐ 52/2024/NĐ-CP — tiền cọc qua tổ chức cung ứng dịch vụ thanh toán có giấy phép (D-187);
Luật Khám bệnh, chữa bệnh 15/2023/QH15 — `bookings` không lưu thông tin sức khoẻ, ghi chú chỉ là chữ ngắn (500 ký tự).

Đã cân nhắc: ràng buộc loại trừ (`EXCLUDE USING gist`) — chỉ Postgres có; ô 15 phút — ít dòng hơn nhưng không tả được
đệm 5 / 10 phút; cache trong bộ nhớ — phải xoá khi đặt và lệch giữa nhiều máy chủ; thu cọc cả với lịch do cửa hàng tạo
— cửa hàng tự quyết khi nhận khách trực tiếp, chưa cần.

Hệ quả: thay đổi về `organizations.timezone` có hiệu lực ngay cho giờ trống và lịch làm việc (giờ đã đặt giữ nguyên
thời điểm UTC). Bảng ô giờ lớn dần theo số lịch: lịch đã qua giữ ô giờ, việc dọn để khi đo thấy cần.
