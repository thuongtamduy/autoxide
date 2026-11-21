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
| `GET /public/products/{id}/similar?locale=vi` | tối đa 12 sản phẩm có nội dung gần nghĩa (embedding của AI); không có embedding thì sản phẩm cùng danh mục, bán chạy trước |
| `GET /public/products/{id}/frequently-bought-together?locale=vi` | tối đa 12 sản phẩm nằm cùng đơn hoàn tất trong 90 ngày qua, nhiều đơn nhất trước |

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

Bán chạy và điểm đánh giá lấy từ module `orders`: mỗi đơn hoàn tất (`orders.order_completed`) đếm
sản phẩm của nó một lần (event nhận lại không đếm hai lần), trong cửa sổ 90 ngày mà job hằng ngày
(`catalog.refresh_sales`) dời tới; event đánh giá mang tổng số đánh giá và tổng số sao hiện tại của sản
phẩm. Cũng các đơn đó cho "thường mua cùng". "Tương tự" cần module `ai` bật embedding
(`AUTOXIDE_EMBEDDING_PROVIDER` và các biến `AUTOXIDE_EMBEDDING_*` khác): mỗi sản phẩm đang bán có một vector mỗi ngôn ngữ, danh sách sản phẩm tương
tự tính lúc lập index, không tính lúc đọc.

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
- `ai_assisted`: chữ soạn từ gợi ý của AI; trang công khai trả kèm để website gắn nhãn AI (Luật Trí tuệ nhân
  tạo — cần đối chiếu văn bản gốc). Tool MCP mặc định đặt dấu này; người viết lại chữ thì gỡ bằng `PATCH`.

Website đọc `GET /api/v1/catalog/public/collections/{slug}?locale=vi` (công khai; `site` hay tên miền): chữ của
bộ sưu tập và một trang sản phẩm, cùng thẻ như `/public/products`, chỉ sản phẩm còn hiện trên website đó, theo
thứ tự của bộ sưu tập. Chỉ bộ sưu tập `active` trong khung giờ, của website hay nút cấp trên, là công khai. Banner
và menu trỏ tới bộ sưu tập bằng `catalog.collection` của sổ đối tượng (`/collections/<slug>`). Tool MCP:
`catalog.list_collections`, `catalog.save_collection` (chỉ bản nháp: người dùng bật `active`).
Ví dụ dựng trang chủ: [`docs/guides/storefront.vi.md`](guides/storefront.vi.md).

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

`catalog.display_price_changed` báo khi giá công khai của một sản phẩm trên một website đổi (giá sau giảm
tự động nếu có, không thì giá thấp nhất của các biến thể; `null` khi không bán ở đó; không bao giờ là giá
gạch `compare_at`). Dựng lại mà giá như cũ thì không gửi gì.

## Nhập và xuất (CSV)

Cả hai chạy nền và cần nơi lưu file (module `files`); không có thì trả 503 `catalog.storage_unavailable`.
File giữ 7 ngày.

- `POST /imports?org_id=<nút>` với thân là chính file (`Content-Type: text/csv`, UTF-8, tối đa 20 MiB và
  50 000 dòng; `catalog.import`). Cột ở dòng đầu, thứ tự tuỳ ý: `sku` (bắt buộc), `name`, `locale`,
  `short_description`, `description`, `kind`, `tax_category`, `barcode`, `weight_grams`. SKU đã có thì
  sửa biến thể đó và nội dung của sản phẩm ở `locale` (giữ đường dẫn); SKU mới thì tạo sản phẩm **nháp**
  một biến thể (cần `name` và `tax_category`). Nhập không bao giờ mở bán, nên nhập lại cùng file không
  đổi gì. Mỗi dòng được kiểm như sửa tay bằng quyền của bạn: dòng sai được báo, các dòng khác vẫn ghi.
- `GET /imports/{id}`: `status` là `queued`, `running`, `succeeded`, `succeeded_with_errors` (xem
  `errors`: dòng, cột, mã, lời nhắn), `rejected` (file không hợp lệ; không ghi gì) hay `failed`. Chỉ người
  bắt đầu thấy. Sản phẩm nhiều biến thể hay tuỳ chọn thì hoàn thiện qua API sau khi nhập.
- `POST /exports` `{"org_id": …}` (`catalog.export`): mọi biến thể của sản phẩm bạn đọc được, cùng cột,
  để sửa rồi nhập lại. Ô mà bảng tính sẽ chạy như công thức (`=`, `+`, `-`, `@`) có `'` ở đầu; khi nhập
  lại, `'` đó được bỏ. Khi `GET /exports/{id}` báo `succeeded` thì tải `content_path`.

Tool MCP: `catalog.start_import`, `catalog.import_status`.

## Bản nháp AI cho mô tả và SEO

`POST /products/{id}/suggestions` `{"locale": "vi"}` (`catalog.products.write`, tính năng gói
`catalog.ai`, module `ai` đã bật) xin mô hình mô tả ngắn, mô tả và chữ SEO chỉ dựa trên điều nội dung sản
phẩm đã nói. Kết quả là **bản nháp**: sản phẩm không đổi gì. `POST /suggestions/{id}/apply` ghi vào nội
dung ở ngôn ngữ đó (giữ tên và đường dẫn) và đánh dấu sản phẩm `ai_assisted`; `POST
/suggestions/{id}/discard` bỏ nó. Tool MCP: `catalog.suggest_copy`.

## Hỏi đáp sản phẩm, menu và module khác

Sản phẩm và danh mục nằm trong sổ đối tượng của kernel, nên module khác trỏ tới được mà không đọc
bảng của catalog:

- `catalog.product`: công khai khi đang bán (`active`) và có nội dung ở ngôn ngữ hỏi; tiêu đề, mô tả
  ngắn, đường dẫn `/<tiền tố>/<slug>` và ảnh đầu tiên. Hỏi đáp sản phẩm dùng module `comments` với loại
  đích này. Cấu hình gợi ý: `AUTOXIDE_COMMENTS_TARGET_TYPES` gồm `catalog.product`,
  `AUTOXIDE_COMMENTS_VERIFIED_ONLY_TYPES=catalog.product` (chỉ tài khoản đã xác thực, Nghị định
  147/2024 — cần đối chiếu phạm vi với văn bản gốc) và
  `AUTOXIDE_COMMENTS_MODERATION_BY_TYPE=catalog.product:user=pre` (câu hỏi chờ duyệt). Lưu trữ hay xoá
  sản phẩm thì hỏi đáp của nó bị đóng và ẩn. Tác giả quản lý được sản phẩm (`catalog.products.write` ở
  nút của nó) có nhãn `seller`; module `orders` thêm nhãn `verified_buyer`.
- `catalog.category`: cho menu của website (module `menus`) — tên và đường dẫn (`/categories/<slug>`)
  theo ngôn ngữ, danh mục con theo thứ tự hiển thị, chỉ danh mục đang hiện. Khối "Bán chạy" trong menu
  là danh sách sản phẩm công khai lọc theo danh mục, sắp `best_selling`.

Chữ mô tả ảnh theo ngôn ngữ: `PUT /products/{id}/media/{media_id}/alts/{locale}` `{"alt": "…"}` (kèm
`If-Match`; `DELETE` để bỏ). Trang sản phẩm ở ngôn ngữ đó hiện chữ này; không có thì `alt` của sản phẩm ở
ngôn ngữ mặc định, rồi tới chữ của thư viện media ở ngôn ngữ đó. Bỏ ảnh khỏi sản phẩm thì chữ của nó cũng
bị bỏ.

## Thông tin khác nhau giữa các biến thể

`PUT /variants/{id}/attributes` đặt giá trị thuộc tính cho một biến thể — dùng cho mấy người, trong hộp có
gì, cách chăm sóc, dùng được máy rửa bát — bằng bộ thuộc tính của khách (`POST /attributes`), nên không
viết cứng cho ngành nào. Giá trị chữ có thể kèm bản dịch: `{"attribute_id": …, "value": "Rửa tay",
"texts": {"en": "Hand wash"}}`. Trang sản phẩm công khai liệt kê chúng dưới từng biến thể
(`variants[].attributes`) ở ngôn ngữ hỏi.

## Hàng số

Sản phẩm loại `digital` giao bằng link tải thay vì giao hàng (tính năng gói `catalog.digital`).

1. Tải file lên bằng `POST /api/v1/files` (hay tải thẳng, `/api/v1/files/uploads`).
2. Gắn vào biến thể: `POST /variants/{id}/digital-assets` `{"file_id": …, "download_limit": 5,
   "grant_days": 30}` (`catalog.products.write`). Nội dung được chép lại, nên xoá file đã tải lên sau
   đó không ảnh hưởng người mua. Hàng số chưa có file thì không mở bán được
   (`catalog.digital_asset_missing`).
3. Khi module `orders` xác nhận một đơn **đã trả tiền** (`orders.order_confirmed` với `payment_status`
   `paid`), mỗi dòng đơn của biến thể hàng số được một quyền tải cho mỗi file: `download_limit` lượt,
   hiệu lực `grant_days` ngày. Nhận lại event không phát thêm. Huỷ đơn, hay hoàn tiền không gắn trả
   hàng, thu hồi quyền ngay.
4. Người mua có tài khoản xem quyền của mình bằng `GET /downloads` và dùng một lượt bằng
   `POST /downloads/{id}`. Người quản lý có `catalog.downloads.manage` xem quyền của đơn
   (`GET /orders/{id}/downloads`), thu hồi (`POST /orders/{id}/downloads/revoke`) hay cấp token mới để
   gửi cho người mua không có tài khoản (`POST /orders/{id}/downloads/reissue`, lần duy nhất
   `token_path` hiện ra).
5. Dùng một lượt (`POST /public/downloads/{token}`, không cần đăng nhập) trả địa chỉ file đã ký, sống
   `AUTOXIDE_CATALOG_DOWNLOAD_LINK_SECONDS`; địa chỉ nhận `Range` để tải tiếp và không trừ thêm lượt.
   Địa chỉ ký bằng khoá riêng của khách: địa chỉ của khách này không dùng được trên tên miền của khách
   khác.

Dịch vụ (`service`) không giao hàng, không tồn kho; combo (`bundle`) trừ kho từng thành phần
(`PUT /variants/{id}/components`); hàng đặt trước dùng `sale_mode` của biến thể.

## Cấu hình

| Biến | Mặc định | Ý nghĩa |
|---|---|---|
| `AUTOXIDE_CATALOG_LOCALES` | `en,vi` | Ngôn ngữ được viết nội dung sản phẩm và danh mục |
| `AUTOXIDE_CATALOG_PRODUCT_PATH_PREFIX` | `products` | Đoạn đầu của đường dẫn công khai của sản phẩm |
| `AUTOXIDE_CATALOG_DOWNLOAD_LINK_SECONDS` | `300` | Số giây địa chỉ file đã ký của hàng số còn dùng được |
| `AUTOXIDE_CATALOG_DEFAULT_DOWNLOAD_LIMIT` | `5` | Số lượt tải mỗi file khi file không tự đặt |
| `AUTOXIDE_CATALOG_DEFAULT_GRANT_DAYS` | `30` | Số ngày người mua tải được khi file không tự đặt |
