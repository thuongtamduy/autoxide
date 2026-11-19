# Sản phẩm

[English](CATALOG.md)

Module `catalog` giữ sản phẩm mà mọi module thương mại đọc: sản phẩm có biến thể, cây danh mục và bảng
giá có đủ lịch sử giá. Kho, khuyến mại, vận chuyển, đơn hàng và kênh bán đọc sản phẩm và giá qua
`autoxide_catalog::api::Catalog`, không bao giờ đọc bảng `catalog_…`. Mọi thứ nằm dưới
`/api/v1/catalog`; mỗi endpoint được mô tả trong `docs/openapi.json`, và cùng thao tác có tool MCP
(`catalog.*`).

## Sản phẩm và biến thể

- `POST /products` tạo sản phẩm ở trạng thái **nháp** cùng biến thể đầu tiên. `kind` là `physical`
  (giao hàng, có tồn kho), `digital`, `service` hay `bundle`. `tax_category` là mã nhóm thuế của engine
  thuế (`GET /api/v1/tax/categories`), vd `vat.standard`.
- Nội dung theo ngôn ngữ (`PUT /products/{id}/translations/{locale}`); ngôn ngữ được phép là
  `AUTOXIDE_CATALOG_LOCALES`. Mỗi bản dịch có đường dẫn công khai `path`,
  `/<AUTOXIDE_CATALOG_PRODUCT_PATH_PREFIX>/<slug>`. `slug` trống thì sinh từ tên, trùng thì thêm `-2`, `-3`… Mô tả là chữ
  thuần (dòng trống ngăn đoạn); HTML không được hiểu.
- Tuỳ chọn (màu, cỡ…) đặt bằng `PUT /products/{id}/options`: tối đa 3 tuỳ chọn, mỗi tuỳ chọn tối đa
  100 giá trị. Thêm tuỳ chọn thì mọi biến thể đang có nhận giá trị đầu của tuỳ chọn đó. Mỗi biến thể
  giữ một giá trị của mỗi tuỳ chọn, và không hai biến thể nào trùng tổ hợp
  (`catalog.option_combination_taken`).
- SKU và mã vạch duy nhất trong một khách (`catalog.sku_taken`, `catalog.barcode_taken`); SKU so không
  phân biệt hoa thường và khoảng trắng. Mã vạch toàn số dài 8, 12, 13 hay 14 chữ số phải đúng chữ số
  kiểm tra GTIN.
- Chế độ bán của biến thể: `in_stock`, `backorder` hay `preorder` (kèm `preorder_available_on` sau hôm
  nay và `preorder_limit` nếu muốn).
- Ảnh là media công khai của module `media` (`PUT /products/{id}/media`).
- Trạng thái (`POST /products/{id}/status`, permission `catalog.products.publish`): `draft` → `active`
  hay `archived`; `active` ↔ `inactive`; mọi trạng thái → `archived`; `archived` → `draft`. Mở bán cần
  nhóm thuế, tên và một biến thể đang bật. Sản phẩm đã từng bán không xoá được, chỉ lưu trữ.
- Combo (`kind: bundle`) khai thành phần bằng `PUT /variants/{id}/components`; combo không chứa combo
  khác, chính nó hay dịch vụ.

Số sản phẩm là hạn mức `catalog.products` của gói (sản phẩm lưu trữ vẫn tính). Tính năng theo gói:
`catalog.variants` (hơn một biến thể), `catalog.price_lists` (hơn một bảng giá), `catalog.bundles`,
`catalog.digital`, `catalog.ai`.

## Danh mục

Mỗi khách một cây, sâu tối đa 7 tầng (`/categories`, permission `catalog.categories.manage`).
`POST /categories/{id}/move` dời danh mục cùng mọi thứ bên dưới; dời vào chính nó bị từ chối
(`catalog.category_cycle`). Lọc sản phẩm theo một danh mục gồm cả danh mục con.

## Thuộc tính, hàng cấm và readiness

- **Thuộc tính** (`/attributes`, permission `catalog.categories.manage`) có kiểu: `text`, `integer`,
  `decimal` (kèm `scale`), `boolean`, `date`, `option`, `options`. Sản phẩm đặt giá trị bằng
  `PUT /products/{id}/attributes`.
- Danh mục khai thuộc tính bằng `PUT /categories/{id}/attributes`; thuộc tính có thể **bắt buộc**, từ
  `effective_from` tới `effective_to`, cho danh mục và (mặc định) các danh mục con. Thuộc tính bắt buộc là
  dữ liệu: quy định nhãn mới được thêm kèm ngày bắt đầu.
- **Hàng cấm và hàng có điều kiện** là quy tắc dùng chung kèm căn cứ pháp lý (`GET /restriction-rules`).
  Người vận hành nạp bằng `autoxide task catalog.load_restrictions` (gói `vn` phát hành kèm máy chủ; cùng
  lệnh tạo thuộc tính nhãn hàng hoá mẫu như xuất xứ, hạn sử dụng cho một khách). Gắn quy tắc vào danh mục
  bằng `PUT /categories/{id}/restrictions`. Quy tắc tổng hợp từ nguồn thứ cấp — cần đối chiếu văn bản gốc
  với người có chuyên môn pháp lý.
- Hàng có điều kiện cần giấy tờ còn hạn (`/products/{id}/documents`, permission
  `catalog.documents.manage`), vd giấy phép bán lẻ rượu.
- Mỗi sản phẩm có **readiness**: `restricted` (hàng cấm), `missing_attributes`, `missing_documents` hay
  `ready`. Chỉ sản phẩm `ready` mới mở bán được. Lọc bằng `?readiness=`, đếm bằng
  `GET /products/counts?group_by=readiness`. Giấy tờ hết hạn được một job hằng ngày phát hiện.
- Mở bán còn kiểm nhóm thuế có trong engine thuế theo hồ sơ thuế của nút
  (`catalog.tax_category_unknown`).

## Giá

**Bảng giá** thuộc một nút tổ chức (chuỗi hay cửa hàng) và áp ở đó cùng các nút bên dưới. Bảng có tiền
tệ, `tax_inclusive`, đối tượng (`consumer` hay `business`), nhóm khách (không bắt buộc) và `priority`.

**Dòng giá chỉ được thêm.** Giá mới là dòng mới với `valid_from` riêng; `valid_from` ở tương lai là
giá hẹn giờ, `valid_to` làm giá tạm thời — hết hạn thì giá trước đó tự áp lại. Lịch sử giá là
`GET /price-lists/{id}/prices`. `POST /price-lists/{id}/prices` thêm tối đa 500 dòng dưới `reference`
của bạn: gửi lại cùng reference và nội dung thì nhận lại các dòng đã ghi.

**Bảng nào thắng.** Với một cửa hàng, nhóm khách và thời điểm, các bảng áp được là bảng đang bật, đúng
tiền tệ, ở cửa hàng hay một nút phía trên, có nhóm khách (và công ty, kênh, thị trường) trống hay khớp.
Thứ tự: nút gần cửa hàng nhất trước, rồi khớp công ty, nhóm khách, kênh, thị trường, rồi `priority` lớn
hơn, rồi id. Mỗi biến thể lấy giá từ bảng đầu tiên theo thứ tự đó có giá cho nó.
`GET /variants/{id}/price?org_id=…&currency=…` cho thấy giá và danh sách bảng ứng viên theo thứ tự.

**Giá bán cho người tiêu dùng ở Việt Nam** phải bằng VND và gồm thuế khi nút có hồ sơ thuế Việt Nam
(Luật Giá 16/2023/QH15 — cần đối chiếu văn bản gốc với người có chuyên môn pháp lý).

**Bảng giá suy ra** lấy giá từ một bảng gốc (`derived_from`; chỉ một tầng, `catalog.derived_chain`),
điều chỉnh theo `adjust_bp` (phần vạn, có dấu: `-1000` = giảm 10%) và `adjust_minor`, rồi làm tròn theo
**quy tắc làm tròn** (`/rounding-rules`: bội số của `step_minor`, hay dạng `k × step_minor +
ending_minor`, vd bước 10.000 và đuôi 9.000 cho 129.000, 199.000). Một job ghi dòng giá thật vào bảng suy
ra mỗi khi bảng gốc có giá mới, nên đọc giá không bao giờ tính công thức. Dòng tay trong bảng suy ra ghim
giá biến thể đó khi còn hiệu lực.

**Đổi giá hàng loạt** (`/price-lists/{id}/changes`): tạo (`set`, `percent` hay `amount`, cho danh mục,
sản phẩm hay biến thể, có thể tạm thời với `valid_to`) → xem trước (số biến thể, 20 dòng mẫu, vi phạm sàn
giá; không ghi gì) → duyệt (`catalog.prices.approve`; cài đặt nút
`catalog.price_change_requires_second_person` bật thì người duyệt phải khác người tạo) → job ghi dòng giá
(`source = bulk`). Hoàn tác bằng một lần đổi mới về giá cũ.

**Sàn giá** (`/price-guards`): giá tối thiểu và / hoặc biên lợi nhuận tối thiểu trên giá vốn, theo nút
và (nếu muốn) theo danh mục, tác dụng `warn` (giá vẫn ghi, kèm cảnh báo) hay `block`
(`catalog.price_below_floor`). Quy tắc của nút gần nhất thắng; trong một nút, quy tắc của danh mục gần
nhất thắng. Giá vốn đến từ event `inventory.cost_changed`.

## Đọc công khai cho website

Không cần đăng nhập (`/api/v1/catalog/public/…`). Chỉ sản phẩm `active`, có giá ở website đang hỏi và có
nội dung bằng ngôn ngữ hỏi; website lấy theo tên miền hay `?site=<nút>`. Mọi câu trả lời có `ETag` (gửi
lại trong `If-None-Match` để nhận 304) và `Cache-Control: public, no-cache`.

| Endpoint | Việc |
| --- | --- |
| `GET /public/products?locale=vi` | thẻ sản phẩm; lọc `category=<slug>` (gồm danh mục con), `attr.<mã>=<giá trị>[,<giá trị>]` (thuộc tính lọc được), `price_min` / `price_max`, `q` (có dấu hay không đều được), `on_sale`, `seller=<nút>`, `sold_by=own\|marketplace`, `min_rating`; `sort=newest\|price_asc\|price_desc\|best_selling\|rating` |
| `GET /public/products/by-slug/{slug}?locale=vi` | trang sản phẩm: biến thể kèm giá hiện tại, ảnh, thuộc tính, danh mục, ngôn ngữ khác |
| `GET /public/categories?locale=vi` · `GET /public/categories/{slug}?locale=vi` | danh mục đang hiện (danh mục nằm dưới danh mục ẩn cũng ẩn) |
| `GET /public/sitemap` | mọi đường dẫn sản phẩm ở mọi ngôn ngữ, tối đa 200 mỗi trang |

Thẻ sản phẩm có `variant_summary` ("3 cỡ"), `brand` (thuộc tính mã `brand`) và `unit_price` (giá mỗi
kg, l, m hay cái) khi biến thể có lượng tịnh (`net_quantity` + `net_unit`, vd 500 và `g`). Trang sản
phẩm cho mỗi biến thể một `path` kèm `?variant=<id>`, `unit_price`, và với giá tạm thời có
`price_valid_until` và `regular_price` (giá ngay trước nó) — đủ cho dữ liệu có cấu trúc (`Product`,
`ProductGroup`, `Offer`); GTIN là `barcode`.

Hỗ trợ tìm:

- `GET /public/search/suggest?q=…&locale=vi` — gợi ý khi người mua gõ (sản phẩm, danh mục, và thứ
  module khác góp), khớp đúng dấu trước, tối đa 10.
- `GET /public/products/facets` — nhận bộ lọc của `/public/products` và đếm số sản phẩm theo từng giá
  trị thuộc tính và theo người bán (số đếm của một bộ lọc không lọc theo chính nó).
- `GET` / `PUT /synonyms` — nhóm từ đồng nghĩa của khách (`[["áo thun", "áo phông", "tee"]]`); tìm và
  gợi ý dùng chúng (`catalog.categories.manage`).

Website hiện sản phẩm nào: của chính nút đó, của các nút cấp trên, và của người bán bên dưới nó
(`seller.own = false` trên thẻ). Cửa hàng ẩn một sản phẩm của chuỗi bằng
`PUT /stores/{org_id}/overrides/{product_id}` `{"hidden": true}` (`catalog.products.write` ở cửa hàng).

Giá trên các trang này là giá hiện tại của bảng giá của website (nút gần nhất có bảng giá đang bật),
không theo nhóm khách. Giá được tính sẵn vào một bảng dẫn xuất, dựng lại bằng job sau mỗi lần đổi giá,
trạng thái, bảng giá hay lựa chọn của cửa hàng, và mỗi phút cho giá hẹn giờ vừa bắt đầu hay vừa hết
(event `catalog.price_effective`). Nếu danh sách công khai có vẻ chưa cập nhật, chạy
`autoxide task catalog.rebuild_storefront [tenant=<slug>]`.

## Bộ sưu tập

Bộ sưu tập gom sản phẩm cho một trang của website ("Giảm giá mùa hè", "Hàng mới tháng này"). Quản lý dưới
`/collections` (`catalog.collections.manage` ở nút của bộ sưu tập; đọc cần `catalog.products.read`); `PATCH`
và `PUT /collections/{id}/items` cần `If-Match`.

- `kind: manual` — sản phẩm chọn tay có thứ tự: `PUT /collections/{id}/items` `{"product_ids": [...]}` (tối đa
  500) thay cả danh sách.
- `kind: rule` — sản phẩm khớp mọi điều kiện của `rules` (tối đa 10):
  `{"category_ids": [...], "attributes": {"color": ["red"]}, "price_min": 100000, "price_max": 500000,
  "new_within_days": 30, "on_promotion": true, "match": "all"}`. Giá theo đơn vị nhỏ nhất của tiền tệ bảng giá
  của website; `on_promotion` cần module `promotions` (`catalog.rule_needs_module`).
- `sort`: `manual` (thứ tự chọn, chỉ bộ sưu tập chọn tay), `newest`, `price_asc`, `price_desc`, `best_selling`;
  giá trị bằng nhau giữ thứ tự ổn định theo ID trên cả hai database.
- `texts` theo ngôn ngữ: `title`, `slug` (không trùng trong ngôn ngữ), `description`, `meta_title`,
  `meta_description`; `media_id` (ảnh công khai); `status` (`draft`, `active`, `archived`) và `starts_at` /
  `ends_at` tuỳ chọn.

Website đọc `GET /api/v1/catalog/public/collections/{slug}?locale=vi` (công khai; `site` hay tên miền): chữ của
bộ sưu tập và một trang sản phẩm, cùng thẻ như `/public/products`, chỉ sản phẩm còn hiện trên website đó, theo
thứ tự của bộ sưu tập. Chỉ bộ sưu tập `active` trong khung giờ, của website hay nút cấp trên, là công khai. Banner
và menu trỏ tới bộ sưu tập bằng `catalog.collection` của sổ đối tượng (`/collections/<slug>`). Tool MCP:
`catalog.list_collections`, `catalog.save_collection` (chỉ bản nháp: người dùng bật `active`).

## Cho module khác

```rust
let catalog = context.require::<dyn autoxide_catalog::api::Catalog>()?;
```

`variants`, `variant_by_code`, `prices` (giá hiệu lực tại một thời điểm), `reference_prices` (giá hiệu
lực ngay trước một thời điểm — "giá trước khuyến mại"), `price_floors` và `bundle_components`. Mỗi lần
gọi nhận tối đa 500 ID và chạy số câu query cố định. Danh sách event ở
`autoxide_catalog::api::events`.

Module đăng ký sau `catalog` cắm vào qua `dyn CatalogExtensions`: module khuyến mại đưa giá giảm tự động
vào danh sách công khai bằng `set_sale_price_source` (một `SalePriceSource`), và phát
`promotions.automatic_prices_changed` (`after.variant_ids`) khi giá giảm đổi. Module sàn có thể từ chối
mở bán một sản phẩm bằng `set_listing_gate` (một `ListingGate`); lúc đó đổi trạng thái trả 422
`catalog.listing_denied`.

## Cấu hình

| Biến | Mặc định | Ý nghĩa |
|---|---|---|
| `AUTOXIDE_CATALOG_LOCALES` | `en,vi` | Ngôn ngữ được viết nội dung sản phẩm và danh mục |
| `AUTOXIDE_CATALOG_PRODUCT_PATH_PREFIX` | `products` | Đoạn đầu của đường dẫn công khai của sản phẩm |
