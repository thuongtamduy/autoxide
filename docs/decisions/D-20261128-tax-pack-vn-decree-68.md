## D-20261128-tax-pack-vn-decree-68 · Gói thuế vn bản 4 theo NĐ 68/2026; orders hỏi engine thuế loại hoá đơn — 2026-11-28 — agent quyết

Chủ đề: Tiền, thuế, chứng từ

Quyết định: Gói `vn` bản 4 thêm quy tắc `einvoice.not_required_below` (hộ kinh doanh doanh thu năm dưới 500 triệu đ
không bắt buộc hoá đơn điện tử, từ 05/3/2026) và `household.tax_free_revenue` (ngưỡng không chịu GTGT, TNCN 500
triệu đ, từ 01/01/2026); căn cứ NĐ 68/2026 cho máy tính tiền (từ 1 tỷ) và khấu trừ qua sàn (giữ tỉ lệ cũ, ghi "các
nguồn không thống nhất"). `TaxEngine` thêm `invoice_advice` (cài đặt mặc định `None`, D-038): có bắt buộc hoá đơn điện
tử không, loại hoá đơn luật đặt. `orders` lập hoá đơn theo lời khuyên: không bắt buộc → không tự lập; người bán không
theo phương pháp khấu trừ → hoá đơn bán hàng; bán tại quầy mà phải dùng máy tính tiền → hoá đơn máy tính tiền; còn lại
như trước (tại quầy → máy tính tiền, khác → `orders.invoice_kind`). Số đơn, phiếu trả, phiếu bảo hành lấy năm theo ngày
thuế của người bán (#575 N4, như D-20261126-tax-date-time-zone).

Căn cứ: NĐ 68/2026/NĐ-CP (ký và hiệu lực 05/3/2026 theo các bản tổng hợp) Điều 8, 10, 11; NĐ 117/2025/NĐ-CP; NĐ
123/2020 sửa bởi NĐ 70/2025 — cần đối chiếu văn bản gốc / người có chuyên môn pháp lý.

Lý do: trước đây hộ kinh doanh bán online nhận hoá đơn GTGT (sai loại) và hộ nhỏ bị tự lập hoá đơn điện tử dù luật
không buộc. Khoảng 500 triệu – 1 tỷ các nguồn khác nhau: coi là bắt buộc (lập hoá đơn khi không buộc không trái luật,
không lập khi buộc thì trái). Doanh thu chưa khai cũng coi là bắt buộc. Phương thức mới thay vì thêm trường vào
`InvoiceRequirements` (struct công khai không `non_exhaustive`: thêm trường là phá vỡ).

Đã cân nhắc: đổi `InvoiceRequirements` (phá vỡ hợp đồng); bỏ mặc định máy tính tiền tại quầy cho doanh nghiệp (NĐ 70/2025
đưa bán lẻ trực tiếp vào diện dùng máy tính tiền — giữ như cũ).
