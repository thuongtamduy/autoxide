## D-20261126-invoice-tax-difference · Chênh lệch thuế giữa đơn và hoá đơn: báo, không ép bằng nhau — 2026-11-26 — agent quyết

Chủ đề: Tiền, thuế, chứng từ

Quyết định: Hoá đơn của đơn tính thuế theo ngày lập của nó (D-20261126-tax-date-time-zone); `orders` **không** ép hoá đơn
lấy thuế đã chốt lúc đặt. Khi thuế hay tổng tiền của hoá đơn bán khác số đã chốt của đơn (`order.tax_minor`,
`total_minor`), chứng từ của đơn trả `differs_from_order: true` (tính lúc đọc, không thêm cột) và `orders` ghi log cảnh báo
lúc lập. Không so hoá đơn điều chỉnh, và không so khi đơn có dòng huỷ một phần (hoá đơn chỉ gồm phần còn lại).

Căn cứ (rà pháp lý vòng 1, LG-10): thời điểm xác định thuế GTGT với hàng hoá là lúc chuyển giao quyền sở hữu hay quyền sử
dụng, không phân biệt đã thu tiền (Luật Thuế GTGT 48/2024/QH15, Điều 8 — theo [Deloitte](https://www2.deloitte.com/content/dam/Deloitte/vn/Documents/tax/vn-tax-alert-new-law-on-vat-vi.pdf));
thời điểm lập hoá đơn NĐ 123/2020 Điều 9 sửa bởi NĐ 70/2025. **Cần đối chiếu văn bản gốc và người có chuyên môn thuế**:
thuế suất của hoá đơn khi đơn đặt trước, giao sau mốc đổi thuế suất.

Lý do: phiên điều phối đề nghị hoá đơn lấy đúng thuế đã chốt của đơn; làm vậy thì đơn đặt 30/12/2026, giao 03/01/2027
mang GTGT 8% đã hết hạn trên hoá đơn năm 2027 — trái hướng luật về thời điểm xác định thuế. Với giá đã gồm thuế (bán lẻ),
tổng tiền hoá đơn vẫn bằng tiền khách trả, chỉ phần thuế tách ra khác; với giá chưa thuế (bán buôn), tổng tăng. Cả hai cần
người dùng biết để xử lý với người mua, nên báo thay vì lặng lẽ khác. Đây là tối thiểu mà rà pháp lý đề xuất.

Đã cân nhắc: truyền ngày thuế của đơn sang hoá đơn — sai thời điểm xác định thuế khi giao sau mốc; thêm cột lưu chênh lệch
— không cần, tính lúc đọc rẻ; phát event riêng — chưa ai nghe, thêm khi có module cần.
