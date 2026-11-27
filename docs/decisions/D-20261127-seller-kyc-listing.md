## D-20261127-seller-kyc-listing · Sàn chặn mở bán hàng của người bán chưa định danh — 2026-11-27 — agent quyết

Chủ đề: Pháp luật và tuân thủ

Quyết định: Khi khách đặt `kernel.business_model = marketplace` và có module `compliance`, `catalog` hỏi
`KycGate::require_verified(khách, Subject::Org(nút sở hữu sản phẩm), Activity::Sell)` trước khi chuyển sản phẩm sang
`active`. Sản phẩm của nút gốc (chính sàn) không hỏi. Có cần định danh hay không do quy tắc `kyc.required_activities`
(dữ liệu có ngày hiệu lực) của `compliance` quyết; thiếu thì lỗi `compliance.kyc_required` (403). `catalog` giữ
`KycGate` riêng, không chiếm chỗ `ListingGate` (một chỗ, dành cho module sàn sau này). `kernel.business_model` vì vậy
không còn chỉ là thông tin.

Căn cứ: Luật Thương mại điện tử 122/2025/QH15 (hiệu lực 01/7/2026); NĐ 248/2026/NĐ-CP Điều 18 — chủ nền tảng thương mại
điện tử trung gian xác thực danh tính người bán (cá nhân: họ tên, ngày sinh, số định danh cá nhân; tổ chức, hộ kinh
doanh: tên, trụ sở, số định danh, người đại diện; cá nhân nước ngoài: hộ chiếu còn hạn ít nhất 6 tháng); áp dụng từ
01/01/2027 (Điều 52) theo các bản tổng quan (thuvienphapluat, luatvietnam, tapchicongthuong) — cần đối chiếu văn bản gốc.
Quy tắc hiện bật `sell` từ 01/7/2026, sớm hơn ngày áp dụng: giữ vì là hướng an toàn cho người mua.

Lý do: Nghĩa vụ định danh là của sàn đối với người bán trên sàn, không phải của cửa hàng tự bán hàng của mình; core chỉ
biết khách là sàn qua `kernel.business_model`. Nút khác gốc trên sàn được coi là người bán (kể cả cửa hàng của chính sàn
đặt thành nút con — định danh nút đó một lần). Kiểm trước transaction ghi, như `ListingGate`.

Đã cân nhắc:
- `compliance` cắm `ListingGate`: không được, `compliance` đăng ký trước `catalog`, và chiếm mất chỗ của module sàn.
- Hỏi định danh cho mọi khách: chặn cửa hàng độc lập không có nghĩa vụ này.
- Cấu hình riêng `catalog.seller_kyc`: thêm một nút bật trùng nghĩa với mô hình kinh doanh.

Không có `compliance` (chỉ khi tự ghép module; binary chính thức luôn có): không chặn, ghi log cảnh báo mỗi lần mở bán
trên sàn — không chặn cứng vì `catalog` không tự định danh được, chặn thì sàn tự ghép không bán được gì.

Còn mở (phiên điều phối chép vào ROADMAP mục LG-3): hồ sơ KYC chưa ghi đủ mục Điều 18 (ngày sinh, địa chỉ, người đại
diện, nước cấp và hạn hộ chiếu ≥ 6 tháng lúc xét duyệt) — cần quyết giữ ở core hay để nhà cung cấp eKYC giữ; hàng đang
bán không bị kiểm lại khi khách chuyển thành sàn, quy tắc bắt đầu áp dụng hay định danh hết hạn / bị thu hồi (cần job
hay event của `compliance` dừng hàng); livestream và tiếp thị liên kết chưa hỏi `KycGate` (chưa có module).
