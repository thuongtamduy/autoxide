## D-20261125-tenant-postal-address · Địa chỉ người gửi thư quảng cáo theo từng khách — 2026-11-25 — agent quyết

Chủ đề: Pháp luật và tuân thủ

Quyết định: Địa chỉ bưu điện trong chân thư chiến dịch newsletter lấy theo thứ tự: cấu hình khách
`newsletter.postal_address` → tên và địa chỉ chủ sở hữu trong hồ sơ pháp lý của khách (module `compliance`, qua
trait công bố `LegalProfiles::template_variables`, như chân thư của `notifications`) → biến chung
`AUTOXIDE_NEWSLETTER_POSTAL_ADDRESS`. Thư quảng cáo không gửi khi cả ba đều trống (503
`newsletter.postal_address_missing`, như cũ). Trang lựa chọn và trang xác nhận đăng ký cũng theo khách
(`newsletter.link.preferences`, `newsletter.link.confirm`, D-20261125-tenant-link-templates).

Lý do: một biến chung làm mọi khách gửi quảng cáo dưới địa chỉ của người vận hành — sai người gửi. Căn cứ: thư điện
tử quảng cáo phải ghi rõ thông tin người gửi (tên, địa chỉ) và cách từ chối nhận — Nghị định 91/2020/NĐ-CP về
chống tin nhắn rác, thư điện tử rác, cuộc gọi rác (hiệu lực 01/10/2020), các điều về thư điện tử quảng cáo; cần
đối chiếu văn bản gốc / người có chuyên môn pháp lý về đúng điều khoản và yêu cầu "địa chỉ bưu điện". Hồ sơ pháp lý
của khách đã là nguồn thông tin chủ sở hữu phải công khai, nên dùng làm mặc định.

Đã cân nhắc: chỉ thêm setting khách, không đọc hồ sơ pháp lý — khách đã khai hồ sơ phải khai lại. Đọc bảng của
`compliance` trực tiếp — trái luật kiến trúc 1 và 2.
