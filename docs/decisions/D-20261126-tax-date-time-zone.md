## D-20261126-tax-date-time-zone · Ngày thuế và ngày lập hoá đơn theo múi giờ của vùng thuế người bán — 2026-11-26 — agent quyết

Chủ đề: Tiền, thuế, chứng từ

Quyết định: Ngày thuế (`tax_point`) của đơn hàng và ngày lập mặc định của hoá đơn là **ngày theo lịch của vùng thuế người
bán**, không phải ngày UTC. Thêm `TaxEngine::tax_date(tenant, org, at)` (có cài đặt mặc định trả ngày UTC — D-038):
engine thuế tìm hồ sơ thuế gần nhất của nút, lấy múi giờ từ quy tắc `tax_date.time_zone` của gói thuế (dữ liệu có ngày
hiệu lực; gói `vn` version 2: `Asia/Ho_Chi_Minh`); vùng thuế không khai thì theo `organizations.timezone` của nút; không có
gì thì UTC. `orders` (giá đơn) và `invoices` (ngày lập khi phát hành, ngày tính thuế của nháp) gọi hàm này. Thời điểm vẫn
lưu UTC; chỉ ngày thuế tính theo múi giờ.

Múi giờ của vùng thuế **thắng** cài đặt của nút: luật Việt Nam tính ngày theo giờ Việt Nam bất kể nút đặt múi giờ nào, và
`organizations.timezone` mặc định `UTC` — để cài đặt của nút thắng thì người chưa đặt vẫn dính lỗi.

Căn cứ (rà pháp lý vòng 1, LG-2): NĐ 123/2020/NĐ-CP sửa bởi NĐ 70/2025/NĐ-CP, Điều 9 (thời điểm lập hoá đơn); Nghị quyết
204/2025/QH15 (17/6/2025) và NĐ 174/2025/NĐ-CP (30/6/2025, hiệu lực 01/7/2025 tới hết 31/12/2026) giảm GTGT 10% → 8%
([KPMG](https://kpmg.com/vn/vi/home/phan-tich-chuyen-sau/2025/07/decree-174-extending-vat-reduction-to-2026.html),
[thuvienphapluat](https://thuvienphapluat.vn/ma-so-thue/phap-luat-thue/chi-tiet-mat-hang-giam-thue-gtgt-tu-10-xuong-8-den-het-nam-2026-892633-222484.html)).
Số hiệu và ngày tra bằng tìm kiếm web ngày 26/11/2026 qua nguồn thứ cấp — **cần đối chiếu văn bản gốc**.

Cùng ngày thuế cho mọi chỗ suy ngày (sau kiểm độc lập của #560): ngày ghi sổ cái (`ledger`, mọi bút toán không
nêu ngày — kể cả GTGT đầu ra và khấu trừ thay của sàn) lấy `TaxEngine::tax_date`; nháp hoá đơn tính vào ngày khác ngày
lập được tính lại thuế khi lập; hoá đơn điều chỉnh, thay thế tính thuế theo ngày lập của hoá đơn gốc (hướng dẫn các đợt
giảm thuế trước: điều chỉnh theo thuế suất của hoá đơn bị điều chỉnh — **cần đối chiếu văn bản gốc**). Thuế của đơn là
ước tính lúc đặt; thuế suất do hoá đơn quyết theo ngày lập, vì luật gắn thời điểm xác định thuế GTGT với lúc chuyển giao
quyền sở hữu hàng hay hoàn thành dịch vụ (Luật Thuế GTGT 48/2024/QH15, Điều 8 — cần đối chiếu). `autoxide doctor`
cảnh báo vùng thuế đã nhập không khai múi giờ.

Cần đối chiếu thêm: (1) giảm GTGT có được gia hạn sang 2027 không — chưa xác nhận tính tới 26/11/2026; gia hạn thì nhập
gói mới, không sửa code; (2) nhóm hàng bị loại khỏi diện giảm thuế từ 01/01/2026 (theo Luật Thuế TTĐB 66/2025/QH15, gồm
đồ uống có đường theo một số nguồn) — khách tự xếp những mặt hàng đó vào `vat.standard`.

Lý do: giờ Việt Nam là UTC+7 — đơn hay hoá đơn lập từ 00:00 tới 06:59 ngày D trước đây mang ngày D−1. Ngày 01/01/2027,
khung đó còn tính GTGT 8% đã hết hạn; mọi ngày, ngày lập sai làm sai kỳ khai thuế ở ranh giới tháng, quý. Múi giờ đặt
trong gói (không viết cứng) vì là quy định của từng vùng thuế, đổi bằng gói mới như thuế suất.

Đã cân nhắc: thêm cột múi giờ vào bảng vùng thuế — cần migration, trong khi bảng quy tắc có sẵn ngày hiệu lực; thêm trường
múi giờ vào `TaxProfile` — đổi hợp đồng REST và để từng người bán tự đặt sai; chỉ dùng `organizations.timezone` — mặc định
UTC, đúng lỗi cần sửa.
