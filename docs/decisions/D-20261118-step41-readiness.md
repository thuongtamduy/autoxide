## D-20261118-step41-readiness · `catalog`: thuộc tính bắt buộc, hàng cấm / có điều kiện và `readiness` — 2026-11-18 — agent quyết

Chủ đề: Thương mại

Quyết định:

1. **`readiness` một giá trị, thứ tự cố định**: `restricted` (một danh mục của sản phẩm hay tổ tiên gắn quy tắc hàng
   cấm đang hiệu lực) → `missing_attributes` (thiếu thuộc tính bắt buộc đang hiệu lực hôm nay: của danh mục sản phẩm, hay
   của tổ tiên khi `applies_to_descendants`) → `missing_documents` (hàng có điều kiện thiếu giấy tờ còn hạn) → `ready`.
   Cột tính lại trong cùng transaction với mọi thay đổi ảnh hưởng (danh mục của sản phẩm, giá trị thuộc tính, thuộc
   tính hay quy tắc của danh mục — cho cả nhánh, giấy tờ) và bằng job hằng ngày `catalog.refresh_document_readiness`
   (giấy tờ hết hạn theo ngày). Mở bán **tính lại tại chỗ** chứ không tin cột, để lỗi nêu đúng mã thuộc tính / loại
   giấy tờ còn thiếu.
2. **Quy tắc hàng cấm / có điều kiện là dữ liệu dùng chung** (`catalog_restriction_rules`, mặt điều khiển, D-198),
   khoá (vùng, mã, ngày hiệu lực), nạp bằng `autoxide task catalog.load_restrictions` từ gói `data/vn/restrictions.json`
   phát hành kèm (mỗi quy tắc ghi căn cứ, nguồn, ngày hiệu lực; nạp lại không đổi gì). Cùng lệnh tạo bộ thuộc tính nhãn
   hàng hoá mẫu (NĐ 43/2017/NĐ-CP sửa bởi NĐ 111/2021/NĐ-CP) cho một khách; khách tự gắn chúng vào danh mục với ngày
   hiệu lực. Danh sách trong gói tổng hợp từ nguồn thứ cấp ngày 18/11/2026 — cần đối chiếu văn bản gốc / người có
   chuyên môn pháp lý trước khi dùng thật (D-186).
3. **Nhóm thuế** kiểm khi mở bán bằng `TaxEngine::quote` một dòng 0 đồng ở vùng thuế của hồ sơ thuế của nút:
   `tax.category_unknown` → 422 `catalog.tax_category_unknown`. Chưa cài `tax` hay nút chưa có hồ sơ thuế thì không kiểm
   (không biết vùng thuế); thiếu thuế suất không chặn mở bán (chứng từ báo khi tính thuế thật). Khi tạo / sửa sản phẩm
   chỉ kiểm dạng mã.
4. **Giá trị thuộc tính `decimal`** lưu số nguyên nhân theo `scale` (0–6) trong `value_integer`; nhiều chữ số thập phân
   hơn `scale` thì từ chối, không làm tròn ngầm. `options` lưu nhiều dòng, một dòng mỗi giá trị chọn.

Lý do: lọc và đếm sản phẩm chưa mở bán được cần một cột có index (`tenant_id`, `org_id`, `readiness`); tính lúc đọc thì
đếm không dùng được index. Quy tắc luật dùng chung để mọi khách cùng một nguồn và cập nhật một lần.

Đã cân nhắc: nhiều cột cờ (`missing_attributes`, `missing_documents`…) — bộ lọc và số liệu phức tạp hơn mà một sản phẩm
bị cấm thì các lý do khác không còn nghĩa; quy tắc theo khách — mỗi khách phải tự nhập luật.
