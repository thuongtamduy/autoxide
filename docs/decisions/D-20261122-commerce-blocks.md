## D-20261122-commerce-blocks · Khối bán hàng của `catalog`, `promotions`, `orders` đọc như endpoint công khai — 2026-11-22 — agent quyết

Chủ đề: Thương mại

Quyết định (bước 41, 43, 45 phần khối; bước 67):

1. `catalog` khai `catalog.product_grid` (chọn tay tối đa 24 sản phẩm theo thứ tự, hay theo `category_id` /
   `on_sale` / `sort`, `limit` ≤ 24), `catalog.product_card` (`product_id`), `catalog.collection` (`collection_id`).
   Dữ liệu là đúng thẻ của `GET /api/v1/catalog/public/products` (giá hiện tại, ảnh, điểm) và lọc như đọc công
   khai: website đang xem, sản phẩm đang hiện, có bản dịch bằng ngôn ngữ của trang; mọi thẻ sản phẩm và mọi sản
   phẩm chọn tay của cả trang gom vào **một** lần đọc. Ngôn ngữ trang mà catalog không có → khối `null`.
2. `promotions` khai `promotions.flash_sale` (dữ liệu như `GET /public/promotions/flash-sales`, gồm cả flash sale
   sắp chạy quá 24 giờ để trang đếm ngược) và `promotions.voucher_claim` (chỉ chương trình đang chạy, `public`,
   `claimable`; giao diện lưu voucher qua `POST /public/promotions/{id}/claim` sẵn có). Hai đọc theo lô mới
   (`public_*_by_id`) dùng chung phần ghép với danh sách công khai.
3. `orders` khai `orders.add_to_cart`, `orders.buy_now`, `orders.cart_link` (chỉ props — giao diện gọi API giỏ hàng và
   thanh toán sẵn có, khối không giữ giá) và `orders.reviews` (số sao và vài đánh giá đã đăng mới nhất, như hai
   endpoint công khai của đánh giá). `orders.reviews` đọc từng sản phẩm trong một lần gọi (trang thường có một khối
   đánh giá).
4. Không khối nào viết giá, ngày hay suất vào props: chữ do AI hay người viết không bao giờ mang số liệu bán hàng
   (D-229 mục 6) — số liệu luôn đến từ dữ liệu khối lúc đọc.

Lý do: khối dùng lại đúng đường đọc công khai đã có index và đã kiểm website / ngôn ngữ, nên trang dựng từ khối
không mở đường đọc dữ liệu mới và luôn khớp trang sản phẩm.

Đã cân nhắc: cho `add_to_cart` / `buy_now` dữ liệu giá và tồn (trùng với `catalog.product_card`, và `orders` phải đọc
catalog cho mỗi trang); một khối "giỏ hàng mini" có dữ liệu giỏ (giỏ theo người xem, không cache chung được — giao
diện đọc API giỏ hàng).
