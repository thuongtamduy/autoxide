## D-20261120-catalog-variant-attributes · Thuộc tính theo biến thể, ảnh song ngữ, danh mục trong sổ đối tượng — 2026-11-20 — agent quyết

Chủ đề: Thương mại

Quyết định: (1) Thông tin khác nhau giữa các biến thể cho website bán hàng là **giá trị thuộc tính theo
biến thể** (`catalog_variant_attributes`), dùng chung bộ thuộc tính và cách kiểm giá trị của sản phẩm;
thuộc tính kiểu chữ có thêm `texts` (bản dịch theo ngôn ngữ), `value_text` là bản ngôn ngữ mặc định.
`by-slug` trả `variants[].attributes` ở ngôn ngữ hỏi. (2) `alt` công khai của ảnh theo `locale`: bản ghi
đè của sản phẩm → `alt` của sản phẩm khi `locale` là ngôn ngữ mặc định của sản phẩm → `alt` của media
theo ngôn ngữ (`Media::find_many_in`) → `alt` của sản phẩm. (3) `catalog.category` trong sổ đối tượng có
đường dẫn `/categories/<slug>` (hằng số, không phải biến cấu hình); `catalog.product` công khai khi
`active` và có nội dung ở ngôn ngữ hỏi. (4) Nhãn `seller`: tác giả có `catalog.products.write` phủ nút sở
hữu sản phẩm.

Lý do: Không viết cứng thông tin của một ngành ("dùng được máy rửa bát") vào core: thuộc tính là dữ liệu
khách tự khai (D-223…D-227). Một bộ kiểm giá trị cho sản phẩm và biến thể. Thứ tự lùi của `alt` ưu tiên
chữ đúng ngôn ngữ trước chữ của ngôn ngữ mặc định.

Đã cân nhắc: cột cố định trên biến thể (gắn ngành); mô tả tự do theo biến thể bằng Markdown (không lọc
được, không có cấu trúc cho agent); `Media::find_many_in` không phân biệt bản dịch và bản mặc định của
media nên `alt` của sản phẩm ở ngôn ngữ mặc định đứng trước nó.
