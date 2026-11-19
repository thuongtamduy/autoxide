## D-20261119-step42-purchasing · Nhập hàng của `inventory`: phiếu nhập ghi sổ cái bằng job, sổ của nút mua hàng, công nợ đọc qua `Ledger::balances` — 2026-11-19 — agent quyết

Chủ đề: Thương mại

Quyết định:

1. **Nhà cung cấp thuộc một nút** (đơn vị mua hàng, mặc định nút gốc); mã duy nhất theo khách. Bút toán công nợ ghi vào
   sổ (`book`) của nút đó — kho chỉ là nơi để hàng, thường không phải đơn vị kế toán.
2. **Đơn đặt hàng**: nháp → `ordered` (cộng `incoming` ở kho nhận) → nhận một phần / đủ bằng phiếu nhập (không quá số còn
   lại: `inventory.over_receipt`) → `closed` (phần chưa về bỏ khỏi `incoming`); huỷ được khi còn nháp hay đã đặt mà chưa
   nhận gì. Số `PO-…`.
3. **Phiếu nhập** (`PN-…`, `reference` vĩnh viễn — D-184): mỗi dòng ghi sổ `receipt` có giá vốn (tính lại giá vốn bình
   quân), kèm lô / serial. Có nhà cung cấp, tổng > 0 và đã cài `ledger` → `ledger_status = pending` và xếp job
   `inventory.post_receipt_to_ledger` trong cùng transaction; job ghi Nợ `inventory.stock` (mục đích riêng, bên thường Nợ,
   mở bằng `open_account`) / Có `payable.supplier` (đối tượng `supplier`), mã tham chiếu `inventory.receipt:<id>` — ledger
   chống trùng nên chạy lại an toàn. Thành công → `posted`; lỗi nghiệp vụ (4xx) → `failed` + mã lỗi, ghi lại bằng
   `POST /receipts/{id}/retry-ledger`; lỗi hệ thống → job chạy lại. Không có nhà cung cấp hay `ledger` → `not_applicable`.
4. **Trả hàng nhà cung cấp** (`TH-…`): không lấy hàng đã giữ cho đơn; giá trả mặc định là giá vốn bình quân hiện tại; job
   `inventory.post_supplier_return_to_ledger` ghi bút toán đảo (Nợ phải trả / Có hàng tồn kho).
5. **Công nợ** (`payable`) trên danh sách nhà cung cấp đọc bằng MỘT lần gọi `Ledger::balances` cho cả trang; chưa cài
   `ledger` thì trường vắng mặt.

Lý do: gọi `ledger` trong transaction ghi kho là gọi module khác trong transaction (cấm); job xếp cùng transaction bảo đảm
phiếu nào cũng được ghi sổ đúng một lần. Giá vốn bán hàng không ghi ở bước này (thuộc bước 45 / sau).

Đã cân nhắc: sổ của kho nhận — mỗi kho thành một sổ kế toán riêng, sai với hầu hết doanh nghiệp; ghi sổ đồng bộ sau
commit — mất bút toán khi tiến trình dừng giữa chừng.
