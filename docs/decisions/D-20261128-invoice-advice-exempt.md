## D-20261128-invoice-advice-exempt · Người bán `exempt` vẫn được khuyên hoá đơn bán hàng — 2026-11-28 — agent quyết

Chủ đề: Tiền, thuế, chứng từ

Quyết định: `invoice_advice` của `tax` giữ `kind = sales_invoice` cho mọi chế độ ngoài `vat_credit`, kể cả `exempt`
(người bán không chịu thuế), khi không bán tại quầy; ghi rõ trong code, trong doc của `InvoiceAdvice::kind` và test. Lựa
chọn người bán tự đặt (`orders.invoice_kind`) vẫn thắng ở `orders` kèm cảnh báo trong log (đã có từ #620).

Lý do: kiểm độc lập #620 yêu cầu xem lại nhánh `exempt`. Trả `None` (để người bán chọn) nghe "trung lập" hơn nhưng
`orders` khi đó dùng giá trị mặc định của `orders.invoice_kind` là `vat_invoice` — hoá đơn GTGT do người không khai GTGT
lập, sai hơn hoá đơn bán hàng. Báo giá (`TaxQuote::invoice_type`) cũng đã coi `exempt` là hoá đơn bán hàng: một luật cho
cả hai đường. Loại hoá đơn đúng cho từng trường hợp không chịu thuế (tổ chức không kinh doanh, hàng không chịu thuế của
người khai khấu trừ…) **cần đối chiếu văn bản gốc / người có chuyên môn pháp lý** (NĐ 123/2020 Điều 8, sửa bởi NĐ
70/2025); không chặn công việc vì người bán luôn tự đặt được loại khác.

Đã cân nhắc: trả `None` cho `exempt` — dẫn tới hoá đơn GTGT mặc định như trên; thêm chế độ con cho từng loại không chịu
thuế — chưa có nguồn thống nhất, để khi có tư vấn.
