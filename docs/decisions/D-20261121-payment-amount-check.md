## D-20261121-payment-amount-check · `plans` và `bookings` so số thực thu của `ledger.payment_succeeded` với số phải trả — 2026-11-21 — agent quyết

Chủ đề: Thương mại

Quyết định: `ledger` có hành động đối soát `accept_received` — ghi nhận khoản đối tác thu được kể cả khi khác số phải
thu, và phát `ledger.payment_succeeded` với `amount` là **số thực thu**. Bên nghe không còn được coi event đó là "đã trả
đủ":

1. **`plans`**: kỳ thanh toán chỉ thành `paid` khi `amount` cùng loại tiền và không nhỏ hơn số của kỳ. Thiếu (hay khác
   loại tiền) thì kỳ vẫn `open` (khách trả nốt qua lượt thu mới, hoặc nhà vận hành thu tay bằng `mark-paid`), ghi log
   cảnh báo; module không có mô hình "đã trả một phần" nên không ghi phần đã trả. Lỗi nội bộ
   `plans.billing_period_underpaid` chỉ dùng giữa service và bộ nghe.
2. **`bookings`**: cọc thu thiếu thì lịch không được xác nhận; số đã thu được hoàn lại qua job hoàn cọc sẵn có
   (`refund_pending` với `refund_minor` = số thực thu); lịch vẫn `held` tới khi hết hạn giữ rồi trả chỗ. Tiền về muộn
   (lịch đã hết hạn / huỷ) hoàn đúng số thực thu thay vì số cọc. Khác loại tiền (không xảy ra với lượt thu `bookings`
   tự tạo) thì không tự làm gì, để người xử lý ở `ledger`.
3. Event không mang `amount` (bản cũ trong outbox) được coi như thu đủ — giữ hành vi trước.

Lý do: không để khoản thu thiếu mở quyền dùng gói hay giữ chỗ như đã trả đủ; dùng lại trạng thái và job có sẵn của mỗi
module thay vì thêm mô hình trả một phần.
