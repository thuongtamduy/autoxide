## D-20261120-catalog-recommendations · Bán chạy, đánh giá, mua cùng và sản phẩm tương tự của `catalog` — 2026-11-20 — agent quyết

Chủ đề: Thương mại

Quyết định: (1) Số bán và "thường mua cùng" dựng từ `orders.order_completed`: một dòng
`catalog_sold_products` mỗi (đơn, sản phẩm), unique theo đơn (nhận lại event không đếm hai lần), rồi
tính lại từ đầu số bán 90 ngày (`catalog_product_stats.units_sold` → cột `best_selling`) và top 20
sản phẩm cùng đơn (`catalog_co_purchases`); lịch `catalog.refresh_sales` mỗi ngày bỏ dòng cũ và tính
lại. (2) Event đánh giá của `orders` mang **tổng hiện tại** `product_rating = {count, sum}` của sản
phẩm; `catalog` ghi đè. (3) Dòng storefront lấy bán chạy / điểm từ `catalog_product_stats`, kể cả dòng
mới dựng. (4) Sản phẩm tương tự: một vector mỗi (sản phẩm, ngôn ngữ) — không cắt đoạn như `content`,
vì chữ sản phẩm ngắn; danh sách 12 sản phẩm tương tự tính sẵn lúc lập index (job
`catalog.index_embeddings`, xếp trong transaction dựng lại storefront), thêm đối xứng vào danh sách của
hàng giống nó; đọc công khai chỉ đọc danh sách. Chưa có danh sách thì cùng danh mục, bán chạy trước.

Lý do: Đường đọc công khai phải nhanh (D-222: không tính lúc đọc); tính lại từ đầu chịu được nhận lại
và lệch thứ tự event (D-054); tổng hiện tại thay vì phần thay đổi để ẩn / mở lại đánh giá không làm
lệch điểm. Không giữ chỉ mục vector trong bộ nhớ như `content::ai::semantic` (đặc tả 41 dặn không chép
nguyên) — duyệt vector chỉ ở job.

Đã cân nhắc: Đếm "mua cùng" lúc đọc bằng `GROUP BY` (rẻ khi ít đơn, chậm khi sản phẩm bán nhiều);
chỉ mục vector trong bộ nhớ dùng chung với `content` (phải dời vào kernel, lớn hơn việc này cần); lịch
`catalog.catch_up_embeddings` mỗi giờ (bật embedding sau thì `catalog.rebuild_storefront` đã xếp lập
index cho cả khách — một cách cho một việc).
