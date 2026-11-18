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
  `AUTOXIDE_CATALOG_LOCALES`. `slug` trống thì sinh từ tên, trùng thì thêm `-2`, `-3`… Mô tả là chữ
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

**Sàn giá** (`/price-guards`): giá tối thiểu và / hoặc biên lợi nhuận tối thiểu trên giá vốn, theo nút
và (nếu muốn) theo danh mục, tác dụng `warn` (giá vẫn ghi, kèm cảnh báo) hay `block`
(`catalog.price_below_floor`). Quy tắc của nút gần nhất thắng; trong một nút, quy tắc của danh mục gần
nhất thắng. Giá vốn đến từ event `inventory.cost_changed`.

## Cho module khác

```rust
let catalog = context.require::<dyn autoxide_catalog::api::Catalog>()?;
```

`variants`, `variant_by_code`, `prices` (giá hiệu lực tại một thời điểm), `reference_prices` (giá hiệu
lực ngay trước một thời điểm — "giá trước khuyến mại"), `price_floors` và `bundle_components`. Mỗi lần
gọi nhận tối đa 500 ID và chạy số câu query cố định. Danh sách event ở
`autoxide_catalog::api::events`.

## Cấu hình

| Biến | Mặc định | Ý nghĩa |
|---|---|---|
| `AUTOXIDE_CATALOG_LOCALES` | `en,vi` | Ngôn ngữ được viết nội dung sản phẩm và danh mục |
| `AUTOXIDE_CATALOG_PRODUCT_PATH_PREFIX` | `products` | Đoạn đầu của đường dẫn công khai của sản phẩm |
