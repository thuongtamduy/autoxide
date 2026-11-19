## D-20261118-step45-lifecycle · Vòng đời đơn của `orders`: chính sách là cài đặt kế thừa, theo vận đơn, bút toán doanh thu — 2026-11-18 — agent quyết

Chủ đề: Thương mại

Bối cảnh: mục 4 của bước 45 (vòng đời đơn). Đặc tả 45 dùng bảng `orders_policies` cho chính sách theo nút, và ghi các
tài khoản sổ cái là "tên tạm" cần viết lại theo bộ tên của bước 36.

Quyết định:

1. **Chính sách theo nút là cài đặt kế thừa của bước 34** (`Module::org_settings`, đọc qua `dyn OrgDirectory` của
   `organizations`), không thêm bảng `orders_policies`: khuôn kế thừa (chuỗi đặt, cửa hàng ghi đè), kiểm giá trị và màn
   cài đặt đã có sẵn. Mục này có `orders.buyer_cancel_until` (mặc định `confirmed`), `orders.acceptance_hours` (0),
   `orders.return_window_days` (7), `orders.cod_enabled` (`true`); mục sau thêm cài đặt của mình cùng cách. Chưa cài
   `organizations` thì dùng mặc định.
2. **Nhân viên chuyển tay** chỉ tới `preparing` (nhận đơn), `shipping`, `delivered`, `delivery_failed`, `cancelled`;
   `confirmed` do thanh toán, `completed` do job. `POST /{id}/transitions` bắt buộc `If-Match` (`updated_at` cắt tới micro
   giây ở cả hai database). Người mua huỷ theo `buyer_cancel_until` — cùng hàm cho đường có tài khoản và vãng lai.
3. **Huỷ đơn đã thu hoàn toàn bộ qua `Ledger::refund`** ngay sau khi chuyển (ngoài transaction), mã tham chiếu theo khoản
   (`orders:<id>:cancel:<payment>`) nên gọi lại không hoàn hai lần; `orders.refund_completed` cho `Tender` (bước 47). Bảng
   `orders_refunds` và hoàn một phần thuộc mục đổi trả.
4. **Theo vận đơn:** nghe `shipping.shipment_*` / `shipping.cod_collected` (`Subscription::optional`). Trạng thái đích
   tính từ trạng thái vận đơn; đơn đi đủ các bước trung gian của bảng (mỗi bước một dòng lịch sử), event đến hai lần
   hay sai thứ tự không làm sai. `POST /{id}/shipments` nhận đơn (`preparing`) rồi phát `orders.fulfillment_requested`
   với `after.shipment` là `ShipmentRequest` (khoá `order:<id>:<version>`).
5. **Bút toán doanh thu khi `delivered`, chỉ tự thu / kênh thu**, theo bộ tên của bước 36: Nợ `receivable.customer`
   (tổng đơn; COD thì theo đối tượng `orders.order:<id>` như `shipping` ghi khi thu COD) / Có `revenue.sales` (hàng chưa
   thuế), `tax.vat_output`, `revenue.shipping`. Khoản thu do `ledger` ghi (Nợ `clearing.gateway` / Có
   `receivable.customer`) nên `orders` không ghi lại khoản thu. Tên `orders.customer_advance` của đặc tả chỉ dùng cho trả
   trước một phần (mục đặt trước). Ánh xạ số hiệu tài khoản (vd TK 131, 511, 33311) là dữ liệu của `ledger` — **cần đối
   chiếu** TT 99/2025, TT 133/2016.
6. **Chờ cửa hàng nhận đơn** (`acceptance_hours` > 0, D-209): nhận đơn = chuyển `preparing`; job `orders.expire_unaccepted`
   (mỗi 10 phút) huỷ đơn web `confirmed` quá hạn với lý do `seller_not_accepted` và hoàn tiền. Hoàn tất: job
   `orders.complete_delivered` (mỗi giờ) khi hết `return_window_days` và không còn dòng `orders_completion_holds`
   (`OrderHolds` cho bước 46).

Hệ quả: `dyn OrderHolds` được provide; tool MCP `list_orders`, `get_order`, `transition_order`, `request_shipment`.
