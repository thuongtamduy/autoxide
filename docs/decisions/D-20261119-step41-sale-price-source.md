## D-20261119-step41-sale-price-source · `catalog`: giá giảm tự động vào storefront qua điểm cắm `CatalogExtensions` — 2026-11-19 — agent quyết

Chủ đề: Thương mại

Quyết định:

1. `catalog::api` công bố trait `SalePriceSource` (`sale_prices(tenant, org_id, at, &[(variant, giá niêm yết)]) →
   Vec<SalePrice { variant_id, sale_price, ends_at }>`) và `CatalogExtensions::set_sale_price_source(Arc<dyn …>)`
   (`context.provide::<dyn CatalogExtensions>` trong `start`). Một nguồn; cắm lần hai trả `false`, giữ nguồn đầu.
   Đây cũng là chỗ `set_listing_gate` của bước 46 sẽ vào (mục `api` của bước 41).
2. Khi dựng lại storefront, có nguồn thì `catalog` hỏi giá giảm cho giá niêm yết của từng nút bán (ngoài
   transaction), nhận giá cùng tiền tệ, thấp hơn giá niêm yết, chưa hết; ghi `sale_price_minor` = giá giảm thấp nhất
   trên các biến thể và `sale_ends_at` của giá đó. Chưa có nguồn thì giữ giá giảm đã ghi. Thẻ công khai không hiện giá
   giảm đã quá `sale_ends_at` dù bảng chưa kịp dựng lại.
3. `catalog` nghe `promotions.automatic_prices_changed` bằng `Subscription::optional`, đọc `after.variant_ids`, xếp job
   dựng lại storefront của các sản phẩm đó. Không đọc bảng của `promotions`.

Việc của `promotions` (bước 43, phiên khác): trong `Module::start`, lấy `context.require::<dyn
autoxide_catalog::api::CatalogExtensions>()` rồi cắm một adapter gọi `Promotions::automatic_prices` (chữ ký khớp từng
trường: `AutomaticPrice { variant_id, sale_price, ends_at }`). Phát `promotions.automatic_prices_changed` cả khi một
giá giảm kết thúc, để storefront bỏ giá cũ.

Lý do: `promotions` phụ thuộc `catalog` và đăng ký sau nó, nên `catalog` không gọi thẳng `dyn Promotions` được (vòng
phụ thuộc giữa crate, và trait chưa có lúc `catalog` khởi động). Điểm cắm đảo chiều phụ thuộc mà không thêm hook
vào `Module` của kernel.

Đã cân nhắc: chép giá giảm trong payload event — event phải mang giá theo từng nút bán và tiền tệ, và nhận lại
event cũ sẽ ghi đè giá mới; `catalog` đọc bảng của `promotions` — trái luật kiến trúc 1.
