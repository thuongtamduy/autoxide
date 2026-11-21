## D-20261121-money-low-fixes · Thu hồi thưởng giới thiệu khi hoàn tiền; thẻ quà tặng đã gắn chỉ chủ dùng; khớp biến động số dư theo lượt nhận — 2026-11-21 — agent quyết

Chủ đề: Thương mại

Quyết định:

1. **Thưởng giới thiệu** (`customers`, đặc tả 47 mục 5): đơn đủ điều kiện bị hoàn tới mức tiền hàng còn lại dưới
   `min_first_order_minor` của quy tắc đang hiệu lực lúc thưởng → lượt giới thiệu chuyển `revoked`, điểm thưởng hai bên
   bị thu hồi như điểm tích của đơn khi hoàn (phần chưa dùng; phần đã tiêu không thu được). Voucher đã cấp qua
   `promotions` không thu hồi (việc của `promotions`, voucher dùng một lần và tự hết hạn). Chọn thu hồi thay vì giữ
   thưởng chờ hết hạn đổi trả: thưởng đã cấp lúc đơn `completed`, tức đã qua hạn đổi trả thường; chỉ còn hoàn ngoại lệ.
2. **Thẻ quà tặng**: thẻ chưa gắn chủ là thẻ vô danh (ai có mã dùng được — như thẻ cào, đúng thiết kế); thẻ đã gắn vào
   tài khoản (`POST /me/gift-cards/attach`) chỉ chủ dùng và gắn được; người khác biết mã nhận
   `customers.gift_card_not_found` như mã sai (không lộ số dư).
3. **Biến động số dư VietQR** (`ledger`): trước khi ghi đã thu, biến động "nhận" thanh toán bằng cách ghi `payment_id`
   lên chính nó trong một transaction có khoá dòng thanh toán. Thanh toán đã xong hay đã có biến động khác nhận →
   biến động này vào hàng chờ `duplicate_payment`. Hai lần chuyển cùng mã tới cùng lúc: một lần ghi đã thu, lần kia
   luôn vào hàng chờ (trước đây có thể mất mà không báo). Thêm index (`tenant_id`, `payment_id`) cho bảng biến động.

Lý do: ba chỗ tiền có thể đi sai mà không ai thấy — thưởng cho đơn đã hoàn, số dư thẻ của người khác, khoản chuyển thừa
biến mất. Cách sửa chạy được trên cả SQLite và Postgres (khoá dòng của kernel, câu cập nhật có điều kiện).

Đã cân nhắc:
- Giữ thưởng giới thiệu chờ thêm một kỳ sau `completed`: chậm thưởng cho mọi người vì vài đơn ngoại lệ.
- Bắt mọi thẻ quà tặng phải gắn chủ mới dùng: đổi cách dùng thẻ quà tặng giấy / thẻ cào mà đặc tả 47 cho phép.
- Unique (`tenant_id`, `payment_id`) trên biến động: biến động "trùng" cũng cần giữ `payment_id` để người xử lý biết nó
  thuộc thanh toán nào.
