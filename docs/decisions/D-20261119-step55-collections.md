## D-20261119-step55-collections · Bước 55: bộ sưu tập sản phẩm dùng bộ lọc storefront sẵn có, permission `catalog.collections.manage` — 2026-11-19 — agent quyết

Chủ đề: Thương mại

Quyết định — cách làm bộ sưu tập sản phẩm (bước 55 mục 3):

1. **Quy tắc thành bộ lọc của danh sách công khai** (`catalog_storefront_entries`, D-222): danh mục (gồm con),
   thuộc tính, khoảng giá, hàng mới (`published_at`), đang khuyến mại (`sale_price_minor`). Không câu SQL riêng,
   không index mới; cùng kết quả và thứ tự (sắp theo cột rồi ID) trên hai database.
2. **Giới hạn quy tắc:** tối đa 10 điều kiện, chỉ `match: all` (`any` cần ghép nhiều tập, để sau nếu cần);
   `on_promotion` cần nguồn giá khuyến mại của `promotions` (`catalog.rule_needs_module` khi thiếu).
3. **Permission mới `catalog.collections.manage`** (scoped): đặc tả ghi `catalog.manage` nhưng module không có
   permission đó; bộ sưu tập là việc bày hàng, tách khỏi quyền sửa sản phẩm. Đọc dùng `catalog.products.read`.
4. **Đường dẫn công khai** `/collections/<slug>` (sổ đối tượng `catalog.collection`), không thêm biến cấu hình.
5. **Chọn tay, sắp `manual`:** phân trang trên bảng mục theo `position`; tối đa 500 sản phẩm.
6. **Để lại:** số bán 30 ngày từ event của `orders` (`catalog_product_stats`, lịch `catalog.roll_sold_30d`) — cột
   `best_selling` của storefront đã có và kiểu sắp `best_selling` dùng nó; ghi số bán làm ở cụm sau của bước 55.

Lý do: một cách lọc sản phẩm công khai cho mọi nơi (danh sách, bộ sưu tập), index đã đo ở bước 41; không mở rộng
quyền sửa sản phẩm cho người chỉ bày hàng.
