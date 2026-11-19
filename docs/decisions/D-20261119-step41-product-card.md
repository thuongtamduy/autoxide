## D-20261119-step41-product-card · `catalog`: thẻ sản phẩm và dữ liệu cho JSON-LD — 2026-11-19 — agent quyết

Chủ đề: Thương mại

Quyết định:

1. **Lượng tịnh** là dữ liệu của biến thể: `net_quantity` (số nguyên > 0) + `net_unit` (`g`, `kg`, `ml`, `l`, `m`,
   `piece`), cho cả hai hay không cho cái nào (m0012). **Đơn giá theo đơn vị cơ sở** (D-201) tính bằng số nguyên,
   làm tròn nửa lên: `g`→`kg` (×1000), `ml`→`l` (×1000), còn lại theo chính đơn vị. Thẻ dùng biến thể rẻ nhất có lượng
   tịnh; storefront ghi sẵn `unit_price_minor` / `unit_price_unit` khi dựng lại.
2. **Thương hiệu** là thuộc tính mã `brand` (kiểu `text` hay `option`) mà khách tự khai — không thêm cột vào sản
   phẩm, core không gắn với một ngành. Thẻ và trang sản phẩm trả `brand`.
3. **Tóm tắt biến thể** (`variant_summary`): mỗi tuỳ chọn một mục `{name, count}` theo ngôn ngữ hỏi ("3 cỡ"); đọc theo
   lô cho cả trang (hai câu query), không lưu sẵn.
4. **Trang sản phẩm cho JSON-LD**: mỗi biến thể có `path` (`…/<slug>?variant=<id>`, cho từng `Offer` của
   `ProductGroup`), `unit_price`, `price_valid_until` (dòng giá tạm thời) và `regular_price` — giá thắng ngay trước
   `valid_from` của dòng giá tạm thời đang áp, chỉ khi cao hơn (giá gạch là giá ngay trước khuyến mại). GTIN là
   `barcode` (đã kiểm số kiểm tra).
5. Sửa tài liệu OpenAPI: `value` của thuộc tính (công khai và khi ghi) là JSON bất kỳ (chữ, số, `true`/`false`,
   danh sách), không phải object — test hợp đồng của testkit bắt được.

Lý do: D-201 yêu cầu thẻ có tóm tắt biến thể, điểm, đơn giá theo đơn vị cơ sở và đủ dữ liệu JSON-LD; giữ thương
hiệu là thuộc tính để khách ngành nào cũng dùng được.

Đã cân nhắc: số thập phân cho lượng tịnh (`0.5 kg`) — dùng đơn vị nhỏ hơn (`500 g`) thay, không cần số thực; cột
`brand` riêng — khách không bán hàng có thương hiệu phải bỏ trống, và trùng với thuộc tính.

Bổ sung cùng ngày — **điểm cắm `set_listing_gate`** (mục 10, cho bước 46): `CatalogExtensions::set_listing_gate(Arc<dyn
ListingGate>)`, một điều kiện (cắm lần hai trả `false`). Trước khi chuyển sản phẩm sang `active`, `catalog` đọc sản
phẩm qua kiểm quyền đọc (ngoài phạm vi → 404, không lộ lý do) rồi hỏi `can_publish(ListingCandidate { product_id,
org_id, category_ids, kind })` NGOÀI transaction ghi; `Deny { code, message }` → 422 `catalog.listing_denied`, mã và
lời của module cắm vào nằm trong `detail`. Không có module cắm vào thì không đổi gì.
