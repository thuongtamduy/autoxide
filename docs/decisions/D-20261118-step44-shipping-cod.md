## D-20261118-step44-shipping-cod · `shipping`: tiền thu hộ (COD) chỉ ghi nghĩa vụ trong sổ cái, đối soát bảng kê CSV chung, chênh lệch giải quyết bằng danh sách lý do — 2026-11-18 — agent quyết

Chủ đề: Thương mại

Bối cảnh: đặc tả bước 44 mục 5 (COD) và tiêu chí xong "COD ghi sổ cái (core không giữ tiền, căn cứ)". Hãng vận chuyển thu
tiền của người mua khi giao và chuyển trả người bán theo hợp đồng với hãng; core không nhận hay giữ tiền (D-187, D-216).

Quyết định:

1. **Bút toán, sổ là nút bán của vận đơn, ghi bằng job sau commit** (không gọi `Ledger` trong transaction của
   `shipping`; mã tham chiếu chống ghi hai lần khi job chạy lại):
   - giao xong có COD (`shipping.post_cod`, mã `shipping.cod:<vận đơn>`): Nợ `clearing.carrier_cod` (đối tượng
     `shipping.carrier_account:<id>`) / Có `receivable.customer` (đối tượng `orders.order:<id>`);
   - dòng bảng kê khớp (`shipping.settle_cod`, mã `shipping.cod_settle:<dòng>`): Nợ `shipping.settlement_in_transit` (số
     hãng thực trả) + Nợ `shipping.carrier_fee` (phí hãng trừ) / Có `clearing.carrier_cod`;
   - chênh lệch giải quyết `written_off` (`shipping.write_off_cod`, mã `shipping.cod_write_off:<chênh lệch>`): Nợ
     `shipping.cod_write_off` / Có `clearing.carrier_cod` phần thiếu.
   Ba mục đích riêng mở bằng `open_account` với bên thường là Nợ. Không cài `ledger` thì không ghi sổ (cảnh báo log).
2. **Bảng kê dạng CSV chung** `tracking_number,cod_amount,fee` (đơn vị nhỏ nhất, đọc bằng `autoxide_kernel::csv`) hay lấy
   qua `ShippingCarrier::cod_statement`; bộ nối từng hãng đổi định dạng riêng của hãng sang dạng chung. Khớp theo mã vận
   đơn của hãng: `matched` / `amount_mismatch` / `unknown_tracking` / `not_delivered`. Chống nhập trùng: unique
   (`carrier_account_id`, `statement_reference`) + băm nội dung — cùng nội dung trả bảng kê cũ, khác nội dung →
   `shipping.statement_duplicate`.
3. **Số tiền COD đúng là của yêu cầu giao** (của `orders`), không lấy từ webhook hay bảng kê; lệch thì thành chênh lệch
   `open` (`shipping.cod_mismatch`). Lịch hằng ngày `shipping.cod_aging` mở chênh lệch `overdue` cho vận đơn đã thu quá
   `AUTOXIDE_SHIPPING_COD_SETTLEMENT_DAYS` ngày chưa có trong bảng kê (khoá `overdue:<vận đơn>`, mở một lần).
4. **Giải quyết chênh lệch** cần mã đang bật của danh sách lý do `shipping.cod_resolution` (D-223; mặc định
   `carrier_corrected`, `delivered_confirmed`, `returned_confirmed`, `written_off` — khách đổi nhãn / thêm mã ở
   `/api/v1/kernel/reasons`) và ghi chú bắt buộc; lưu người giải quyết, thời điểm, phát
   `shipping.cod_discrepancy_resolved` (audit nghe event). `POST /cod/discrepancies/{id}/refresh-tracking` hỏi lại hành
   trình từ hãng ngoài transaction ghi.
5. **`shipping.cod_settled`** mang `seller_org_id` và `paid_to` (`carrier_account_payee`: hãng trả thẳng tài khoản người
   bán khai với hãng) để `ledger` / `marketplace` ghi đúng chỗ (D-216).

Căn cứ (cần đối chiếu văn bản gốc / người có chuyên môn pháp lý): NĐ 52/2024/NĐ-CP (thanh toán không dùng tiền mặt — phạm
vi dịch vụ thu hộ của doanh nghiệp bưu chính); Luật Bưu chính 49/2010/QH12; chứng từ đối soát là chứng từ kế toán theo Luật
Kế toán 88/2015/QH13.
