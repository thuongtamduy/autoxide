## D-20261128-seller-kyc-pause · Sàn dừng hàng đang bán của người bán không còn định danh — 2026-11-28 — agent quyết

Chủ đề: Pháp luật và tuân thủ

Quyết định: Trên sàn (`kernel.business_model = marketplace`), `catalog` kiểm lại người bán có hàng `active` và chuyển
hàng của người bán không qua được cổng định danh (`compliance.kyc_required`, hay sàn thiếu `compliance`) sang
`inactive`, phát `catalog.product_status_changed` với tác nhân hệ thống. Kiểm lại khi: nhận
`compliance.kyc_expired` / `compliance.kyc_revoked` của một nút (người bán trong nhánh nút đó), nhận
`organizations.unit_moved` (mọi người bán của khách), và mỗi ngày (job `catalog.seller_kyc_sweep`: khách vừa thành
sàn, quy tắc `kyc.required_activities` bắt đầu áp dụng, khách gộp vào sàn). Định danh lại xong, người bán tự mở bán
lại — không tự bật lại hàng. `compliance` công bố hằng tên hai event trong `api` (`KYC_EXPIRED`, `KYC_REVOKED`).

Căn cứ: Luật Thương mại điện tử 122/2025/QH15 Điều 17; NĐ 248/2026/NĐ-CP Điều 18 (chủ nền tảng trung gian xác thực
người bán) — cần đối chiếu văn bản gốc. Không kiểm được thì không coi là đã kiểm (luật số hai, D-20261127-seller-kyc-listing).

Lý do: cổng của D-20261127-seller-kyc-listing chỉ chặn lúc chuyển sang `active`; hàng bán trước đó vẫn bán khi định
danh hết hạn, bị thu hồi, hay khi khách mới thành sàn. Event cho phản ứng ngay; job hằng ngày bắt mọi trường hợp không
có event (đổi cài đặt, quy tắc tới ngày áp dụng). Bên nhận chịu nhận lại (D-054): chỉ đổi dòng còn `active`.

Đã cân nhắc:
- `compliance` tự dừng hàng: không được, module không ghi bảng của module khác.
- Bật lại hàng khi định danh lại: người bán có thể đã muốn tắt; không đoán.
- Chỉ event, không job: đổi `kernel.business_model` hay quy tắc mới có hiệu lực không phát event nào.
