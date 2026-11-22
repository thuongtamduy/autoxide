## D-20261122-einvoice-provider-number · Trạng thái "đã chấp nhận" của hoá đơn điện tử mang số do nhà cung cấp cấp — 2026-11-22 — agent quyết

Chủ đề: Tiền, thuế, chứng từ

Quyết định: `EInvoiceStatus::Accepted` của kernel có thêm `number: Option<i64>` — số hoá đơn do nhà cung cấp cấp khi
nhà cung cấp tự cấp số và chỉ biết số lúc hoá đơn được chấp nhận (`None`: không có, hay đã trả ở
`IssuedEInvoice::number` lúc gửi). `invoices` lưu số đó vào `provider_number` của chứng từ (cột đã có, API đã trả ra)
trong cùng transaction ghi mã cơ quan thuế; không trả thì giữ số đã có. Bộ giả và lớp sandbox trả `None` — hành vi sẵn
có không đổi.

Lý do: số hợp lệ về thuế của hoá đơn điện tử là số trên hoá đơn đã phát hành; người bán và người mua phải tra được nó
cùng mã của cơ quan thuế (Nghị định 123/2020/NĐ-CP, Điều 10 khoản 3 về số hoá đơn — cần đối chiếu văn bản gốc). Nhà
cung cấp xử lý bất đồng bộ (SePay eInvoice, và phần lớn dịch vụ ký số rồi mới cấp số) chỉ có số sau bước gửi, nên đây
là nhu cầu chung của hợp đồng chứ không phải vì một nhà cung cấp. Phiên điều phối đã duyệt mở rộng này.

Đã cân nhắc: ghép số vào `authority_code` (hai khái niệm khác nhau, người đọc API sẽ hiểu sai); thêm lời gọi riêng
`number(provider_ref)` (thêm một lượt gọi mạng cho mỗi hoá đơn); đổi `number` của chứng từ theo số của nhà cung cấp (số
nội bộ đã cấp không lùi, không đổi — luật đánh số sẵn có).

Thay: phần "giới hạn: hợp đồng chưa mang số về" ở mục 4 của D-20261121-sepay-einvoice
