## D-20261118-step41-storefront · `catalog`: storefront dẫn xuất và đọc công khai — 2026-11-18 — agent quyết

Chủ đề: Thương mại

Quyết định:

1. **Bảng dẫn xuất `catalog_storefront_entries`**: một dòng mỗi (nút bán, sản phẩm) ghi sẵn hiện / ẩn, giá thấp
   nhất / cao nhất trên các biến thể đang bật, giá gạch, cột sắp xếp (`published_at`, `best_selling`,
   `rating_x100`) — đọc công khai lọc và sắp theo giá có index, không giải giá lúc đọc (D-222). **Nút bán** là gốc,
   mọi nút có bảng giá đang bật và mọi nút có tuỳ biến ẩn / hiện; website của nút S đọc dòng của nút bán gần nhất
   trong chuỗi tổ tiên của S. Tiền tệ của nút bán là tiền tệ của bảng giá chung (không lọc nhóm khách, công ty,
   kênh, thị trường) ở nút gần nhất; không có thì mọi sản phẩm ẩn ở đó.
2. **Giữ bảng dẫn xuất đúng**: job `catalog.refresh_storefront` (theo sản phẩm, theo biến thể, hay cả khách) xếp
   CÙNG transaction với mọi thay đổi ảnh hưởng — tạo / sửa / đổi trạng thái / xoá sản phẩm, sửa / thêm / xoá biến
   thể, thêm dòng giá (tay, suy ra, hàng loạt), tạo / sửa bảng giá, tuỳ biến cửa hàng. Job tính lại từ đầu nên chạy
   lại không sai; giữ nguyên cột do module khác ghi (bán chạy, đánh giá, giá sau giảm). Lịch
   `catalog.apply_scheduled_prices` mỗi phút dựng lại sản phẩm có dòng giá bắt đầu / hết hạn trong hai phút qua và
   phát `catalog.price_effective`. Task `catalog.rebuild_storefront` dựng lại cả khách theo lô 200 sản phẩm.
3. **Website thấy gì**: sản phẩm của chính nút website, của nút cấp trên (như `content`) và của nút bên dưới (người
   bán trên website của chuỗi — `seller.own = false`). Cửa hàng ẩn sản phẩm của chuỗi bằng tuỳ biến của nút; ẩn ở
   nút X áp cho X và các nút dưới X. `seller=<id nút>` lọc theo nhánh người bán; `sold_by=own|marketplace` tách hàng
   của website với hàng người bán. Người bán nêu bằng ID nút (nút chưa có slug), như `site=`.
4. **Đọc công khai**: chỉ sản phẩm có bản dịch bằng ngôn ngữ hỏi (không lấy ngôn ngữ khác thế vào, như `content`).
   Lọc thuộc tính `attr.<mã>=<giá trị>[,…]` chỉ nhận thuộc tính `filterable`, giá trị theo kiểu (lựa chọn theo mã,
   thập phân dạng chữ); tối đa 10 bộ lọc. Lọc giá theo `min_price_minor` ("giá từ"). Mọi bộ lọc là subquery có
   index; sắp bằng `fetch_page_by`; `sort=price_asc|price_desc` đặt luôn `order`. `ETag` là dấu vân tay SHA-256 của
   ID khách và cả câu trả lời (như `content`), 304 do middleware của kernel trả.
5. **Chưa làm ở mục này**: `facets=true` và gợi ý tìm (mục kế, D-201); cột giá sau giảm `sale_price_minor` /
   `sale_ends_at` có sẵn và `on_sale` đã lọc theo, nhưng chỉ `promotions` (bước 43) ghi vào; huy hiệu "đã xác thực"
   của người bán chờ `marketplace` (bước 46).

Lý do: website gọi danh sách nhiều hơn mọi endpoint khác; giải giá theo cây cho từng sản phẩm lúc đọc là N câu
query mỗi trang và không sắp theo giá được. Bảng dẫn xuất đổi một job sau mỗi lần ghi lấy một câu query có index
mỗi lần đọc.

Đã cân nhắc: một dòng mỗi (cửa hàng, sản phẩm) cho mọi nút — nhân số dòng theo số cửa hàng dù đa số dùng chung giá
chuỗi; giải giá lúc đọc kèm cache — vẫn không sắp / lọc theo giá bằng database được.
