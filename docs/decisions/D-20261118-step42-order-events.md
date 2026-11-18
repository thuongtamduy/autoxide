## D-20261118-step42-order-events · `inventory` nhận event của `orders` qua `Subscription::optional`, lỗi nghiệp vụ chỉ ghi log — 2026-11-18 — agent quyết

Chủ đề: Thương mại

Quyết định: `inventory` nghe `orders.order_confirmed` (→ `confirm`), `orders.order_cancelled` (→ `release`, lý do
`cancelled`) và `orders.return_received` (→ `restock` với `reference = return:<return_id>`) bằng
`Subscription::optional` — chưa cài `orders` thì không lỗi khởi động. Mỗi event gọi đúng hàm service mà
`api::Inventory` gọi, nên event đến trước hay sau lời gọi trực tiếp, hay đến lại, kết quả như nhau. Payload đọc:
`after.order_id`; với trả hàng `after.return_id`, `after.order_id`, `after.warehouse_org_id`,
`after.lines[{variant_id, quantity, sellable (mặc định true), lot_code?, serials[]}]`. Lỗi nghiệp vụ (4xx: không có chỗ
giữ, chỗ đã hết hạn, dữ liệu sai) chỉ ghi log `warn` và coi như đã xử lý; lỗi hệ thống (5xx) để bộ phát giao lại.

Lý do: giao lại một event mà lỗi nghiệp vụ sẽ lỗi mãi và chặn hàng đợi; đơn không có hàng theo dõi tồn (hàng số, dịch
vụ) hợp lệ mà không có chỗ giữ. Tên event và payload theo đặc tả 42 (module cung cấp hợp đồng thắng, `docs/steps/README.md`).

Đã cân nhắc: trả lỗi để bộ phát giao lại — không bao giờ thành công với lỗi nghiệp vụ; `Subscription::new` — bắt mọi
bản cài phải có `orders`.
