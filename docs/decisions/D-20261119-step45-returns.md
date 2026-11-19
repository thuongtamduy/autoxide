## D-20261119-step45-returns · Đổi trả và hoàn tiền của `orders`: một đường hoàn, tiền hoàn tính lúc nhận hàng, bút toán đảo — 2026-11-19 — agent quyết

Chủ đề: Thương mại

Bối cảnh: mục 6 của bước 45 (đổi trả và hoàn tiền, D-209, D-222, D-223). Trước mục này chỉ huỷ đơn đã thu mới hoàn
(gọi thẳng `Ledger::refund`, D-20261118-step45-lifecycle mục 3). Đặc tả 43 mục "Trả hàng và huỷ" cho sẵn hàm thuần
`promotions::api::returns::{line_refund, order_clawback}`.

Quyết định:

1. **Một đường hoàn tiền cho mọi nguồn** (`refunds.rs`): huỷ đơn, trả hàng, nhân viên hoàn tay (`POST /{id}/refunds`, tool
   `refund_order`, permission `orders.refund`), tranh chấp của bước 46 (`OrderRefunds`, nay được provide). Lượt hoàn ghi
   vào `orders_refunds` trong transaction khoá dòng đơn: tổng các lượt chưa hỏng không vượt tiền đã thu, mỗi lượt lấy từ
   **một** khoản thanh toán và không vượt phần còn lại của khoản đó (hoàn nhiều khoản thì gửi nhiều lệnh). Cùng
   `reference` + cùng nội dung → lượt cũ (200).
2. **Qua cổng hay `manual`:** khoản thu qua `ledger` thì gọi `Ledger::refund` (mã `orders:refund:<id>`); cổng hoàn ngay →
   xong, báo sau → nghe `ledger.payment_refunded` (`Subscription::optional`). Sổ từ chối hẳn (4xx) → lượt hoàn `failed`,
   trả lại phần đã giữ chỗ; lỗi tạm (5xx) giữ `pending`, gửi lại cùng `reference` sẽ gọi lại cổng. Khoản không qua cổng
   (COD, tiền mặt) → `manual`: người bán tự trả tiền cho người mua, lượt hoàn ghi nhận việc đó. Core không giữ tiền của
   người mua (D-187). Cổng không có event "hoàn thất bại" nên lượt `pending` mà cổng từ chối về sau chỉ thấy được qua
   `GET /{id}/refunds` — thêm khi `ledger` có event đó.
3. **Bút toán:** xong một lượt hoàn thì `orders.refund_completed` (cùng transaction); `orders` nghe lại event của mình để
   ghi sổ. `manual`: Nợ `receivable.customer` / Có `cash.on_hand`. Đơn đã ghi doanh thu (đã giao, tự thu / kênh thu): bút
   toán đảo Nợ `revenue.sales`, `tax.vat_output`, `revenue.shipping` (chia số hoàn theo tỉ lệ của bút toán doanh thu bằng
   `Money::allocate`) / Có `receivable.customer`. Hoàn trước khi giao: bút toán đảo ghi khi ghi doanh thu. Mã tham chiếu
   `orders:<đơn>:refund:<lượt>:paid|revenue` — giao lại event không ghi hai lần. Cách hạch toán hàng bán bị trả lại (vd
   TK 5213 hay ghi giảm 511) là dữ liệu của `ledger` — **cần đối chiếu** TT 99/2025, TT 133/2016.
4. **Yêu cầu trả** (`orders_returns`, số `R<năm>-000001`): chỉ đơn `delivered` trong `orders.return_window_days` sau khi
   giao (đơn `completed` là đã hết hạn); lý do thuộc cài đặt kế thừa mới `orders.allowed_return_reasons` (mặc định cả sáu
   mã của `orders.return_reason`) và đang bật trong danh sách lý do; chỉ dòng hàng cần giao, không phải quà; số lượng
   không vượt phần chưa trả và chưa nằm trong yêu cầu mở khác (kiểm lại trong transaction khoá dòng đơn). Mọi lỗi đầu vào
   báo một lần (D-083). Yêu cầu mở giữ một dòng `orders_completion_holds` (`return:<id>`) — đơn không hoàn tất tới khi
   yêu cầu bị từ chối, đã hoàn tiền hay đóng.
5. **Các bước của nhân viên** (`orders.returns.manage`, `POST /returns/{id}/transitions`, tool `decide_return`):
   `approve`, `reject` (mã của `orders.return_rejected`), `receive` (từng dòng: số nhận, `restock` / `damaged`; nhận tại
   cửa hàng được bỏ qua bước duyệt), `refund`, `close`. Gửi lại đúng bước hiện tại trả kết quả hiện có (agent retry).
   `receive` cộng `returned_quantity` có chặn trên (không trả quá số mua kể cả khi chạy song song), phát
   `orders.return_received` đúng hình dạng `inventory` đọc (`warehouse_org_id` một kho cho cả lần nhận — mặc định kho
   đã giao, vì `inventory` chống trùng theo `return:<id>`), và **tính tiền hoàn lúc này** rồi lưu cùng yêu cầu
   (`refund_minor`, `breakdown`) — lần trả sau biết phần đã thu / đã trừ. Thêm event `orders.return_status_changed` cho
   thông báo người mua.
6. **Tiền hoàn** (`return_refund.rs`, hàm thuần): hàng theo `line_refund` (đơn vị cuối nhận phần dư: nhiều lần trả cộng
   lại đúng tiền đã trả); phí giao chỉ khi lý do là lỗi của người bán (`damaged`, `wrong_item`, `not_as_described`,
   `missing_parts`), chia theo đơn vị hàng cần giao (không tính quà) cùng cách; thu lại khoản giảm lớp `order` theo
   `clawback_on_return` đã chụp, tính theo tổng sau lần trả trừ phần đã thu trước (trả tiếp món đã bị thu thì được trả
   lại); quà: chương trình mất điều kiện (không còn món nào, hay đơn còn lại dưới `min_order`) — `return_gift` thì dòng
   quà được thêm vào yêu cầu lúc tạo, quà không gửi lại hay `deduct_gift_value` thì trừ giá trị quà một lần, `keep` thì
   không. Giá trị quà là giá niêm yết lúc đặt, nay chụp vào `compare_at` của dòng quà. Không bao giờ đòi thêm tiền người
   mua (tiền hoàn tối thiểu 0). Trả hàng một phần không trả voucher về ví (đặc tả 43); huỷ trọn đơn trả qua
   `Promotions::release` như trước.
7. **Hoá đơn điều chỉnh / thay thế** (NĐ 123/2020 sửa bởi NĐ 70/2025) làm ở mục hoá đơn điện tử, khi nghe
   `orders.return_received` / `orders.refund_completed` — event đã mang đủ dòng và số tiền.

Hệ quả: bảng `orders_refunds`, `orders_returns` (migration `m0007_returns`); `GET /{id}/refunds`, `GET /me/returns/{id}`;
tool `refund_order`, `list_returns`, `decide_return`. Cách gửi trả và phí, ảnh kèm, xem trước tiền hoàn thuộc mục "Trả
hàng phía người mua".
