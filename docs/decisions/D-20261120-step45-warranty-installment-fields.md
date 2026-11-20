## D-20261120-step45-warranty-installment-fields · Bảo hành theo serial hay theo dòng mở khi giao xong, trả góp chỉ qua đối tác của `ledger`, ô người mua nhập lúc đặt hàng, hạn của người mua và cách trả trên giỏ — 2026-11-20 — agent quyết

Chủ đề: Thương mại

Bối cảnh: các mục "Trả góp qua đối tác … bảo hành", "Ô người mua nhập lúc đặt hàng" và một phần "Website bán hàng" của
bước 45 (D-187, D-194, D-235, D-232). Đặc tả để agent quyết: lúc mở bảo hành, cách nhận diện bảo hành theo đơn khi sản
phẩm không theo serial, trạng thái của phiếu bảo hành, cách chọn đối tác trả góp.

Quyết định:

1. **Bảo hành mở khi đơn giao xong** (`orders.order_delivered`), hạn tính từ `delivered_at` cộng `warranty_months` chụp
   trong dòng đơn lúc đặt (catalog đổi sau không ảnh hưởng đơn đã bán). Sản phẩm theo serial: nhân viên ghi serial lúc
   chuẩn bị hàng (`POST /{id}/lines/{line_id}/serials`, tối đa bằng số lượng, gửi lại không thêm), mỗi serial một bảo
   hành; không có serial thì một bảo hành cho cả dòng (khoá `serial_key = "-"`). Unique (dòng, `serial_key`) và chèn `ON
   CONFLICT DO NOTHING`: event giao lại không mở hai lần. Ghi serial sau khi đã giao thì thay bảo hành theo dòng bằng bảo
   hành theo serial. Job ngày `orders.expire_warranties` đánh dấu `expired`; tiếp nhận kiểm cả `ends_at` nên không phụ
   thuộc job.
2. **Tra cứu** theo serial hay số điện thoại người mua — số chỉ so qua khoá liên hệ theo khách (`SecretKey::phone_key`,
   D-20261120-step45-guest-orders), không lưu số; phạm vi theo `orders.warranty.manage` ở nút bán. Người mua xem
   `GET /me/warranties`.
3. **Phiếu bảo hành** `orders_warranty_claims`: số `BH<năm>-<số>` qua `numbering` của kernel; trạng thái `received →
   inspecting → repairing | replacing → ready → returned`, `rejected` từ `received` / `inspecting`; chuyển bằng
   `UPDATE … WHERE status = <cũ>` (hai người bấm cùng lúc: một người thắng, người kia 409), gọi lại cùng đích là no-op;
   event `orders.warranty_claim_changed`. Hạn xử lý theo cài đặt `orders.warranty_claim_days` của nút (mặc định 15 ngày —
   dữ liệu, không viết cứng).
4. **Trả góp chỉ qua đối tác có giấy phép**: `payment_method = installment` kèm `{partner, months}` (`months` thuộc 3, 6,
   9, 12, 18, 24); lượt thu tạo qua `Ledger::create_payment` với `provider = partner` — `orders` không tính lãi, không
   duyệt hồ sơ vay, không giữ tiền (D-187). Kế hoạch trả góp lưu trên dòng thanh toán (`orders_payments.installment`).
   Bộ nối với công ty tài chính thật là module riêng, cần secret thật (ngoài phạm vi, đặc tả 45).
5. **Ô người mua nhập** (D-235): `orders.order` khai đích trường tự định nghĩa với mức `internal` và `buyer_input`;
   checkout kiểm bằng `CustomFields::validate_buyer_input` (kernel thêm hàm này: chỉ trường `buyer_input`, khoá khác là
   lỗi, trường `internal` bắt buộc không chặn người mua) và gộp lỗi vào cùng một 422 của checkout. Giá trị lưu ở
   `orders_orders.custom_fields`, trả trong đơn, và đưa vào tham số `fields` của thư "đã nhận đơn" / "đã xác nhận".
6. **Website**: đơn của người mua trả sẵn `buyer_cancel_until`, `buyer_can_cancel`, `return_until` (tính từ chính sách
   của nút); giỏ trả `payment_methods` theo đúng điều kiện checkout kiểm (COD khi giỏ cần giao và nút bật COD — chống bom
   hàng theo người mua chỉ kiểm lúc đặt, khi có số điện thoại); thống kê đánh giá thêm `by_variant` (một câu `GROUP BY`
   trên index sẵn có). Số điểm tối đa dùng được chờ bước 47 (`customers`); khối của trang chờ `Module::blocks` của bước 67.

Căn cứ: Luật Bảo vệ quyền lợi người tiêu dùng 19/2023/QH15 (hiệu lực 01/7/2024) và NĐ 55/2024/NĐ-CP — trách nhiệm bảo
hành của người bán, thời hạn sửa chữa / thay thế — **cần đối chiếu điều cụ thể** (nên hạn xử lý là cài đặt của khách);
Luật Các tổ chức tín dụng 32/2024/QH15: cho vay tiêu dùng / trả góp chỉ do tổ chức được cấp phép — core chỉ chuyển người
mua sang đối tác; Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15: tra bảo hành theo số điện thoại dùng khoá băm theo khách, không
lưu số — cần người có chuyên môn pháp lý xác nhận.

Hệ quả: `catalog` cần chụp đúng `warranty_months` / `serial_tracked` (đã có trong `VariantSnapshot`); bước 47 thêm số
điểm vào báo giá giỏ; bước 67 khai khối `orders.*`.
