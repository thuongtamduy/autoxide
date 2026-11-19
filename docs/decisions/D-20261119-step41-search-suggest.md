## D-20261119-step41-search-suggest · Gợi ý tìm gom nhiều module, từ đồng nghĩa và số đếm bộ lọc của `catalog` — 2026-11-19 — agent quyết

Chủ đề: Thương mại

Quyết định:

1. **Kernel `autoxide_kernel::suggest`**: trait `SuggestSource { kind(); suggest(tenant, site, locale, q, limit) }`
   trả `Vec<Suggestion { kind, label, path?, id?, exact, context }>`. Module góp nguồn bằng
   `StartContext::suggest_source`; module phục vụ endpoint giữ sổ dùng chung `StartContext::suggest_sources()` (nguồn
   đăng ký sau vẫn có mặt). Không thêm hook vào `Module` (phiên điều phối đồng ý). Sổ gom theo thứ tự đăng ký, xếp
   hai tầng (khớp đúng dấu trước, D-202), bỏ trùng (cùng loại và chữ), tối đa 10 (`MAX_SUGGESTIONS`). Một nguồn lỗi
   bị bỏ qua và ghi log, endpoint vẫn trả gợi ý của nguồn còn lại. Mỗi nguồn tự lọc theo khách và website.
2. **Endpoint** `GET /api/v1/catalog/public/search/suggest?q&locale&limit&site` ở `catalog` (đặc tả 41). Nguồn của
   `catalog`: `catalog.product` (sản phẩm đang bán ở website; đúng dấu trên `name_key`, không dấu trên `search_key`
   kèm từ đồng nghĩa) và `catalog.category` (danh mục đang hiện; `context` là tên danh mục cha).
3. **Từ đồng nghĩa** `catalog_synonyms` (m0010): một nhóm một dòng, từ lưu dạng `search::fold`; `GET` / `PUT
   /synonyms` (thay cả tập, tối đa 500 nhóm × 2–20 từ, `catalog.categories.manage`). Câu tìm mở rộng theo từ (không
   theo một phần của từ), tối đa 10 cách nói, mỗi cách một `LIKE` gộp bằng `OR` trong cùng subquery. Bảng là cấu
   hình cả khách: `copy_on_split`.
4. **Số đếm bộ lọc** là endpoint riêng `GET /public/products/facets` nhận đúng bộ lọc của `/public/products` (đặc tả
   viết `facets=true`, nhưng luật code chỉ cho ba hình dạng response, nên số đếm không ghép vào `Page`). Đếm thuộc
   tính lọc được kiểu `option` / `options` / `boolean` (tối đa 20 thuộc tính, 50 giá trị mỗi thuộc tính) và người
   bán (50), nhiều nhất trước. Đếm của một bộ lọc lấy với mọi bộ lọc khác trừ chính nó. Điều kiện lọc dựng ở MỘT chỗ
   (`public_filter.rs`) cho cả danh sách và số đếm. Câu đếm là `GROUP BY` trên index bắt đầu bằng `tenant_id`; m0010
   thêm index `(tenant_id, org_id, visible, owner_org_id)` cho người bán và `(…, rating_x100, product_id)` cho sắp
   theo điểm.

Còn lại: nguồn gợi ý `content.entry` (bài viết) và `organizations.unit` (cửa hàng) thuộc vùng của phiên khác; mỗi
nguồn khoảng 40 dòng theo khuôn `catalog/src/suggest.rs`, cắm bằng `context.suggest_source(…)`.

Lý do: gợi ý gom nhiều module mà `content`, `organizations` không phụ thuộc `catalog` nên trait phải ở kernel;
sổ dùng chung (như `finder_source`) để thứ tự đăng ký module không quan trọng.

Đã cân nhắc: chỉ mục `kernel_search_docs` (D-220) cho gợi ý — cần đồng bộ chỉ mục cho mọi loại trước, còn gợi ý chỉ
cần vài dòng đầu; ghép `facets` vào thân `Page` — trái luật ba hình dạng response.
