## D-20261127-einvoice-cancel-unsupported · Nhà cung cấp không huỷ được hoá đơn đã gửi: `409 invoices.einvoice_cancel_unsupported` thay `422 einvoice.not_supported` — 2026-11-27 — agent quyết

Chủ đề: Tiền, thuế, chứng từ

Quyết định: Huỷ chứng từ (`POST /api/v1/invoices/documents/{id}/cancel`, `dyn Invoices::cancel`) mà nhà cung cấp hoá
đơn điện tử trả `einvoice.not_supported` (SePay eInvoice không có lệnh huỷ) thì trả `409 invoices.einvoice_cancel_unsupported`
(tham số `provider`), hướng sang điều chỉnh giảm hay thay thế và nhắc tạo hồ sơ thuế cho người bán. `plans` xử lý mã này
như `invoices.einvoice_not_cancellable` (điều chỉnh giảm toàn bộ). Phá vỡ tại chỗ trước 1.0 (đổi mã và status của lỗi)
theo D-20261125-api-break-before-1-0; ghi ở changelog mục Changed.

Lý do: ca này gặp khi người bán chưa có hồ sơ thuế nên quy tắc cấm huỷ của vùng thuế (D-20261126-einvoice-no-cancel)
chưa áp; `einvoice.not_supported` ("không hỗ trợ loại hoá đơn này") không nói người dùng phải làm gì, và 422 sai nghĩa —
yêu cầu hợp lệ, chỉ trạng thái của hoá đơn (đã gửi) không cho huỷ (kiểm độc lập #575, N2).

Đã cân nhắc: giữ lỗi của nhà cung cấp — loại, khó hiểu và mỗi nhà cung cấp một kiểu; bắt buộc hồ sơ thuế trước khi lập
hoá đơn điện tử — loại, phạm vi lớn hơn việc sửa này và chạm luồng lập hoá đơn của mọi module.
