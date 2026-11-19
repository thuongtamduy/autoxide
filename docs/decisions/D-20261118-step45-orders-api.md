## D-20261118-step45-orders-api · Module `orders`: hợp đồng `api` cho bước 46–49, điểm cắm `OrderExtensions`, người mua không cần permission — 2026-11-18 — agent quyết

Chủ đề: Thương mại

Bối cảnh: đặc tả bước 45 (`docs/steps/45-orders.md`, mục "Kiểu và trait") phác thảo `orders::api`; bước 46–49
(`marketplace`, `customers`, `channels`, `affiliate`) và `auth` (tạo tài khoản từ số đã xác thực) phụ thuộc hợp đồng
này, nên nó được công bố ngay ở commit đầu của bước 45. Code thật của bước 41–44 lệch phác thảo ở vài chỗ.

Quyết định:

1. **Mã đơn là `Uuid`, không newtype `OrderId`.** `inventory`, `promotions`, `shipping` đã chốt `order_id: Uuid` trong
   hợp đồng của họ; một newtype riêng ở `orders` chỉ thêm phép đổi qua lại ở mọi chỗ gọi. Mã tham chiếu giữ chỗ chung là
   `api::hold_reference(order_id)` = `orders:<order_id>`.
2. **Hàm của trait nhận `&TenantContext`** (luật chung D-198), không `TenantId` như phác thảo.
3. **`FundsFlow` dùng tên của `ledger::FlowMode`** (`self_collect`, `partner_collect`, `split_at_gateway`) cộng
   `channel_collect` (kênh ngoài thu rồi trả lại — bước 48). Phác thảo gọi `GatewaySplit`; tên thật theo bên cung cấp.
4. **Điểm cắm `OrderExtensions`** (`set_seller_split`, `set_buyer_groups`, `add_tender`): `orders` khởi động trước
   `marketplace` / `customers`, nên không `require` được trait của họ; họ `require::<dyn OrderExtensions>()` rồi tự cắm.
   Mỗi điểm cắm một lần (`OnceLock`), cắm lần hai hay hai `Tender` cùng `kind` → lỗi khởi động. Không ai cắm thì đơn chỉ
   có một người bán, không nhóm khách, không điểm.
5. **`BuyerGroups::profile_of` trả `BuyerProfile`** (`groups`, `membership_tier`, `price_group`) thay vì chỉ danh sách
   nhóm: `promotions::Cart` cần cả nhóm lẫn hạng thành viên, `catalog::PriceQuery` cần mã nhóm giá — một lần gọi đủ cả ba.
6. **Người mua không cần permission.** Tài khoản người mua (`Realm::Customer`, bước 34) đăng nhập được nhưng không có
   permission nào; đường `/me/…` của `orders` chỉ cần đăng nhập và chỉ chạm giỏ / đơn của chính người đó, như sổ địa chỉ
   của `shipping` (D-20261118-step44-shipping-api) và `/me/tickets` của `support`. Bỏ permission `orders.buy` của đặc tả.
7. **Kiểu trong `api` đều `#[non_exhaustive]` kèm hàm dựng** (`ImportedOrder::new`, `ImportedLine::new`,
   `SplitLine::new`…) để thêm trường sau không phá module dùng.
8. **Danh sách lý do** (`orders.review_hidden` — mọi mã mặc định là `legal`, không tắt được; `orders.return_rejected`;
   `orders.return_reason` với bộ mã `changed_mind`, `damaged`, `wrong_item`, `not_as_described`, `missing_parts`,
   `other`; `orders.cash_variance`) khai ngay ở commit đầu.

Hệ quả: bước 46–49 viết code theo `autoxide_orders::api` ngay; các trait `Orders`, `OrderIntake`, `OrderRefunds`,
`OrderHolds`, `OrderLookups` được `provide` khi mục tương ứng của bước 45 xong (module dùng phải `require` lúc đó, hay
dùng `StartContext::optional` nếu muốn chạy khi chưa có).
