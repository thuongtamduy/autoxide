## D-20261127-no-active-series-code · Lập chứng từ khi không có dãy số đang hoạt động: `409 invoices.no_active_series` thay `404 invoices.series_not_found` — 2026-11-27 — agent quyết

Chủ đề: Tiền, thuế, chứng từ

Quyết định: Khi lập (`POST /api/v1/invoices/documents/{id}/issue`, `dyn Invoices::issue`) một chứng từ không ghim dãy
— hay điều chỉnh / thay thế mà dãy của gốc khác năm lập — và nút, loại chứng từ, năm lập không có dãy đang hoạt động
nào, trả `409 invoices.no_active_series` (tham số `year`, thông báo chỉ cách tạo dãy). `404 invoices.series_not_found`
chỉ còn nghĩa "dãy ghim trên chứng từ, hay dãy được hỏi theo ID, không tồn tại". Phá vỡ tại chỗ trước 1.0 theo
D-20261125-api-break-before-1-0; ghi ở changelog. `plans` (lập hoá đơn kỳ thuê bao) nhận cả hai mã.

Lý do: thiếu dãy của năm lập là trạng thái cần người vận hành xử lý (tạo dãy), không phải "không tìm thấy tài nguyên
được hỏi"; từ khi điều chỉnh năm sau tự chọn dãy của năm lập (kiểm độc lập #575) lỗi này hay gặp hơn, và mã cũ không
nói năm nào thiếu dãy. Chưa có client ngoài phụ thuộc mã cũ ở đường này.

Đã cân nhắc: giữ mã `series_not_found`, chỉ thêm tham số `year` — loại: bản dịch theo mã (i18n key) không nói được
"thiếu dãy của năm {year}", và 404 làm agent tưởng gọi sai ID. Làm endpoint mới — loại, quá nặng cho một mã lỗi trước 1.0.
