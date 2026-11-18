## D-20261118-step41-catalog-design · `catalog`: giá chỉ thêm, thứ tự chọn bảng giá, hợp đồng `api` cho bước 42–44 — 2026-11-18 — agent quyết

Chủ đề: Thương mại

Quyết định:

1. **Dòng giá chỉ thêm** (`catalog_prices`): không có endpoint sửa hay xoá dòng giá. Giá hiệu lực lúc `t` = dòng có
   `valid_from` lớn nhất ≤ `t` mà `valid_to` NULL hay > `t`; hai dòng cùng `valid_from` thì dòng thêm sau (UUIDv7 lớn
   hơn) thắng. "Giá ngay trước `t`" (`Catalog::reference_prices`) dùng `valid_from < t` và `valid_to` NULL hay ≥ `t`.
   Thời điểm cắt về micro giây trước khi lưu và so (Postgres lưu micro giây).
2. **Thứ tự chọn bảng giá** (D-222) chốt: nút gần cửa hàng nhất → khớp `company_id` → `customer_group` → `channel` →
   `market` → `priority` **lớn hơn** thắng → `id`. Mỗi biến thể lấy giá từ bảng đầu tiên trong thứ tự có giá cho nó
   (bảng cửa hàng thiếu giá một biến thể thì biến thể đó lấy giá chuỗi). Tổ tiên lấy từ `OrgTree::ancestors` của kernel
   (không gọi `organizations`); nút chưa có trong cây thì chuỗi là nút đó rồi gốc.
3. **Một lần thêm giá một `reference`**: dòng thứ `i` lưu `reference = <reference>#<i>` (cột 160 ký tự), cùng dấu vân
   tay của cả lần thêm; gửi lại cùng nội dung trả các dòng cũ, khác nội dung → `reference_reused`. Dấu vân tay tính
   trên nội dung client gửi, trước khi điền `valid_from` = bây giờ.
4. **Sàn giá** (`catalog_price_guards`) có cột `currency`; `Catalog::price_floors` nhận thêm `currency` (đặc tả 41 đã
   sửa theo). Quy tắc của nút gần nhất thắng, trong một nút quy tắc của danh mục gần nhất thắng quy tắc không danh
   mục. Biên lợi nhuận tính bằng số nguyên, làm tròn lên đơn vị nhỏ nhất. Giá vốn đọc `catalog_variant_costs`, ghi
   từ event `inventory.cost_changed` (`Subscription::optional`), payload `{"after": {"org_id", "variant_id",
   "unit_cost": <Money>}}`.
5. **Luật "VND + gồm thuế"** cho bảng giá `consumer` chỉ áp khi nút có hồ sơ thuế `VN` (`TaxEngine::profile`); không
   cài `tax` hay chưa có hồ sơ thì không áp (khuyến nghị của đặc tả). Căn cứ: Luật Giá 16/2023/QH15 (hiệu lực
   01/7/2024), điều về niêm yết giá — cần đối chiếu văn bản gốc / người có chuyên môn pháp lý.
6. **Tuỳ chọn và biến thể**: thêm tuỳ chọn mới thì biến thể cũ nhận giá trị đầu của tuỳ chọn đó (như các nền tảng
   lớn); bỏ giá trị biến thể còn dùng → 409 `catalog.option_value_in_use`. Sản phẩm luôn còn ít nhất một biến thể
   (`catalog.last_variant`). Tối đa 3 tuỳ chọn, 100 giá trị mỗi tuỳ chọn.
7. **Mã vạch**: chỉ kiểm chữ số kiểm tra khi mã toàn số dài 8 / 12 / 13 / 14 (GTIN); chuỗi rỗng là NULL.
8. **Mô tả sản phẩm** là chữ thuần (đoạn ngăn bằng dòng trống), giao diện hiển thị Markdown tối giản; không lưu HTML.
9. **Slug** duy nhất theo (khách, ngôn ngữ) cho sản phẩm, và riêng cho danh mục — không theo cửa hàng.
10. **Hạn mức `catalog.products`** đếm cả sản phẩm lưu trữ (lưu trữ không trả hạn mức); xoá sản phẩm nháp trả lại.
11. Cây danh mục khoá mọi dòng danh mục của khách khi đổi cây (Postgres `FOR UPDATE`; SQLite đã giữ quyền ghi cả
    database): hai lần dời chéo nhau chạy nối đuôi, cây không có vòng. Sâu tối đa 7 tầng.

Lý do: đọc giá là đường nóng của giỏ hàng, đơn và khuyến mại: một câu lấy bảng, một câu lấy dòng, chọn trong Rust —
không tính công thức lúc đọc. Lịch sử giá là dòng thật nên "giá ngay trước khuyến mại" (trần 50% của bước 43) đọc
thẳng. Thêm `currency` vào sàn giá vì một nút có thể bán nhiều tiền tệ (bước 63).

Đã cân nhắc: sửa dòng giá tại chỗ — mất lịch sử; giá theo bảng rẻ nhất — D-222 đã loại; gọi `OrgDirectory` của
`organizations` lấy tổ tiên — thêm phụ thuộc trong khi kernel đã có bảng tổ tiên.
