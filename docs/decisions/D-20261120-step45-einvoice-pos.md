## D-20261120-step45-einvoice-pos · Hoá đơn điện tử, thư trạng thái và bán tại quầy của `orders`: `invoices` không ghi doanh thu lần hai, hoá đơn điều chỉnh qua trait, ca quầy hai người duyệt — 2026-11-20 — agent quyết

Chủ đề: Thương mại

Bối cảnh: mục "Hoá đơn điện tử …" và "Bán tại quầy (POS)" của bước 45 (D-215, D-222, D-227, D-340). Code thật lệch đặc
tả ở hai chỗ: `invoices` (bước 36) tự ghi doanh thu khi phát hành chứng từ bán hàng — trong khi `orders` đã ghi doanh
thu khi đơn giao xong (D-20261118-step45-lifecycle) — và `dyn Invoices` chưa có cách lập hoá đơn điều chỉnh từ module
khác (chỉ có REST `/documents/{id}/adjust` cần `invoices.correct`).

Quyết định:

1. **Cờ `booked_by_source` của chứng từ** (`invoices`, migration `m0008_booked_by_source`, `NewDocument::booked_by_source()`):
   bên lập đã tự ghi doanh thu thì phát hành KHÔNG ghi bút toán; hoá đơn điều chỉnh / thay thế kế thừa cờ của chứng
   từ gốc. `orders` luôn lập với cờ này — một nguồn doanh thu duy nhất cho đơn (bút toán khi giao, đảo khi hoàn), hoá đơn
   chỉ là chứng từ pháp lý. Chọn thay vì để `invoices` ghi và `orders` thôi ghi: không phải đơn nào cũng có hoá đơn ngay
   (`on_request`, `invoices` chưa cài) mà doanh thu vẫn phải ghi lúc chuyển quyền sở hữu.
2. **`Invoices::adjust(original, direction, lines, reference)`** thêm vào trait (cùng đường với REST `adjust`, không
   kiểm permission — module gọi tự chịu). Hoàn tiền của một lần trả (`orders.refund_completed` có `return_id`) → hoá đơn
   điều chỉnh giảm theo các dòng hàng trả (giá, khoản giảm chia theo số lượng), mã `orders:<id>:return:<return_id>:invoice`.
   Hoá đơn thay thế (sửa sai thông tin) vẫn do nhân viên làm ở `invoices`.
3. **Thời điểm lập** là cài đặt kế thừa `orders.invoice_timing` (`on_confirm` / `on_delivered` mặc định / `on_complete` /
   `on_request`) như các chính sách khác của `orders` (không bảng `orders_policies`, D-20261118-step45-lifecycle); loại
   hoá đơn `orders.invoice_kind` (`vat_invoice` mặc định / `sales_invoice` theo phương pháp tính thuế của người bán); bán
   tại quầy luôn `cash_register_invoice` (hoá đơn khởi tạo từ máy tính tiền). `orders` nghe event của chính mình rồi gọi
   `dyn Invoices` ngoài transaction; liên kết ở `orders_invoice_links` (unique theo `reference`), event
   `orders.invoice_linked` một lần. Gửi nhà cung cấp hoá đơn điện tử (`EInvoiceProvider`) là việc của `invoices` theo
   ký hiệu.
4. **Thông tin xuất hoá đơn doanh nghiệp** (`invoice_request`: mã số thuế, tên, địa chỉ, email) nhận lúc đặt hay sau đó
   qua `POST /{id}/invoice` (nhân viên) / `POST /me/orders/{id}/invoice` (người mua); đã có hoá đơn → 409
   `orders.invoice_already_issued`. Chứng từ của người mua: `GET /me/orders/{id}/documents` và bản dùng token xem đơn,
   bản in PDF qua `Invoices::render`.
5. **Thư xác nhận / trạng thái** qua `dyn Notifier` (tin giao dịch, 5 loại `orders.order_placed|confirmed|shipped|
   delivered|cancelled`, mẫu i18n), khoá chống trùng = ID event; người nhận: tài khoản, không thì email, không thì số điện
   thoại; đơn quầy không gửi. Thư "đã xác nhận" chỉ khi đã thu tiền (COD đã có thư "đã nhận đơn").
6. **Bán tại quầy:** ca ở `orders_pos_shifts` (một ca mở mỗi quầy qua cột khoá `open_key`), lượt bán ở `orders_pos_sales`
   (unique (ca, `reference` của máy quầy) — gửi lại / gửi song song ra một đơn); dùng chung hàm báo giá, giữ tồn chỉ ở
   kho của cửa hàng (`ReserveRequest.fixed_org_id`); đơn tạo `confirmed` rồi `delivered` trong một transaction (khách
   nhận tại chỗ). Tổng khoản thu phải bằng tổng đơn (`orders.payments_mismatch`); tiền thối = khách đưa − số tiền. Sổ:
   Nợ `cash.on_hand` (tiền mặt) / `clearing.gateway` đối tượng `partner:card_terminal` (thẻ) / `cash.bank` (VietQR) — Có
   `receivable.customer`; doanh thu như đơn online. Core không giữ tiền (D-187). `GET /pos/catalog?barcode=` tra một mã;
   danh mục đầy đủ và `GET /pos/customers` chờ `customers` (bước 47).
7. **Đóng ca có chênh lệch** (D-227): chênh lệch ≠ 0 bắt buộc lý do thuộc danh sách `orders.cash_variance` và ghi chú; ghi
   sổ vào tài khoản chờ xử lý riêng của module (`orders.cash_shortage` bên Nợ / `orders.cash_overage` bên Có, mở bằng
   `Ledger::open_account` — mục đích riêng có tiền tố module thay `suspense.*` của đặc tả, vì `ledger` chỉ có bộ mục
   đích cơ bản). Vượt `orders.cash_variance_approval_minor` (cài đặt kế thừa, mặc định 200.000) thì ca `pending_approval`,
   một người KHÁC người đóng ca có `orders.pos.shift` duyệt (`POST /pos/shifts/{id}/approve`, tự duyệt → 403) rồi mới ghi
   sổ — làm trong `orders` thay vì yêu cầu duyệt chung P12 của bước 51 (chưa có trong kernel).

Căn cứ: NĐ 123/2020/NĐ-CP sửa đổi bởi NĐ 70/2025/NĐ-CP (hiệu lực 01/6/2025) — Điều 9 (thời điểm lập hoá đơn), hoá đơn
khởi tạo từ máy tính tiền, xử lý hoá đơn khi trả hàng (điều chỉnh) — **cần đối chiếu văn bản gốc và người có chuyên môn
trước khi chạy thật**; chênh lệch tiền két vào tài khoản chờ xử lý: TT 200/2014/TT-BTC, TT 133/2016/TT-BTC (TK 1381 /
3381) — ánh xạ số hiệu là dữ liệu của `ledger`, cần đối chiếu; Luật Kế toán 88/2015 Điều 41 (lưu chứng từ).

Hệ quả: `invoices` có thêm cột và hàm trait (không phá vỡ: cột mặc định `false`, hàm mới). Đơn kênh / sàn (bước 46, 48)
dùng cùng đường lập hoá đơn khi nghe event của `orders`.
