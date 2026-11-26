## D-20261126-list-pages-done · Xong "mọi danh sách là `Page<T>`": luật gate, ngoại lệ chờ duyệt, trang của `ReferenceSource`, trần mới — 2026-11-26 — agent quyết

Chủ đề: HTTP, REST, OpenAPI

Quyết định: đóng bước 70 P9 mục 3 (D-20261125-list-pages), PR cuối của đợt:

1. **Gate giữ luật** (`cargo xtask check`, `xtask/src/api_shape.rs`): `api-unpaged-list` (GET trả `{"data": [...]}`
   không có `pagination`), `api-page-params` (danh sách `Page<T>` nhận đủ `limit` / `cursor` / `order` / `count`),
   `api-param-description` (mọi tham số có mô tả), `mcp-page-count` (tool nhận `cursor` cũng nhận `count`). Sau PR này
   `xtask/api-shape-allow.txt` không còn dòng `api-param-description` hay `mcp-page-count` nào (mười một tham số của
   `newsletter`, `shipping`, `support` có `///`; 31 tool danh sách nhận `count`).
2. **Ba ngoại lệ được cho phép** — chủ dự án giao phiên điều phối quyết, 26/11/2026. Ba endpoint sau giữ
   `Data<Vec<T>>` (dòng `api-unpaged-list` trỏ về mục này): chúng là tra cứu theo khoá client gửi, không phải danh
   sách lớn dần theo dữ liệu khách, nên `next_cursor` không có nghĩa và thứ tự trả lời là thứ tự hỏi.
   - `GET /api/v1/catalog/variants?code=`: một mã (`code`) cho tối đa một biến thể theo SKU và một theo mã vạch — tối
     đa 2 phần tử.
   - `GET /api/v1/comments/public/counts`: số bình luận của tối đa 100 đối tượng client nêu, giữ thứ tự hỏi.
   - `GET /api/v1/inventory/public/availability`: tình trạng còn hàng của tối đa 100 biến thể client nêu.
   Điều kiện xem lại (khi đó làm endpoint mới trả `Page<T>` và khai endpoint cũ qua `Module::deprecations`): số phần tử
   không còn bị chặn trên bởi số khoá client gửi (vd tra theo mã trả nhiều biến thể khớp, hay bỏ trần 100 khoá), hoặc
   endpoint thêm bộ lọc trả theo dữ liệu thay vì theo khoá. Hai dòng `api-page-params` của tìm theo độ khớp
   (`content/search`, `content/public/search`: một trang tốt nhất) giữ nguyên.
3. **`/kernel/records/{kind}/{id}/usages` thành `Page<Referrer>`** (thay mục 7 "chưa đổi" của
   D-20261126-list-pages-auth-kernel): trait công khai `ReferenceSource` thêm `referrers_page` **có cài đặt mặc định**
   (D-038) trả `Page<KeyedReferrer>` — khoá UUID của dòng trỏ tới, duy nhất trong nguồn. Kernel đọc mỗi nguồn người gọi
   đọc được với cùng `PageRequest` rồi gộp lần lượt bằng `merge_id_pages` (như hoạt động của bản ghi và uỷ quyền nhận):
   mỗi nguồn cho `limit` dòng đầu sau con trỏ nên `limit` dòng đầu của bản gộp là trang đúng; `total` là tổng các nguồn.
   Cài đặt mặc định chỉ để tương thích (doc nói rõ): đọc tối đa `REFERRERS_PAGE_FALLBACK_LIMIT` = 10 000 dòng qua
   `referrers`, khoá là 16 byte đầu của SHA-256 các trường (ổn định, không theo thời gian), cắt trong bộ nhớ, chạm trần
   thì bỏ `total`. `content` (bài, khoá id bài; index (tenant_id, id) có sẵn, tìm trong thân bài vẫn quét như trước)
   và `menus` (mục menu, khoá id mục; migration m0009 thêm index (tenant_id, ref_kind, ref_id, id) và (tenant_id,
   feature_kind, feature_id, id) cho hai nhánh `OR`) cài keyset trong database. Thứ tự đổi: trước bài theo
   `updated_at` mới nhất, nay mọi nguồn theo id (lúc tạo dòng trỏ tới), mới nhất trước. Tham số `limit` riêng (1–200)
   thay bằng tham số trang chung (cùng khoảng).
4. **Trần thay cho "đọc hết rồi `page_of`"**:
   - `compliance` mục đích xử lý: trần mới `AUTOXIDE_COMPLIANCE_PURPOSES_TENANT_MAX` (mặc định 200, 1–10 000) mục đích
     riêng mỗi khách, kiểm lúc tạo — 409 `compliance.purpose_limit_reached`; bản ghi đè mục đích mặc định đếm vào nhưng
     tự có trần là số mục đích mặc định, sửa không bị chặn. Hai request tạo cùng lúc có thể vượt một dòng (trần để giữ
     sổ nhỏ cho `GET /purposes` và `GET /public/purposes`, không phải hạn mức chính xác).
   - `promotions` `GET /promotions/{id}/conflicts`: không thêm trần — tập đọc là chương trình cùng nút, chưa nháp / huỷ,
     trùng thời gian và có chung đối tượng áp dụng, đúng tập mà bước kiểm xung đột lúc kích hoạt (`conflict_check`) đã
     đọc trọn; chương trình đang chạy của khách còn được bộ máy giá nạp hết vào bộ nhớ (`compiled.rs`) cho mỗi giỏ.
     Nếu sau này cần trần thì trần đặt ở chỗ kích hoạt, cho cả ba chỗ đọc.
   - `page_of` trả lỗi nội bộ khi hai phần tử trùng khoá; `compliance/retention` dùng khoá (module, tên) nên kernel nay
     chặn ngay lúc khởi động module khai hai `RetentionPolicy` trùng tên (lỗi `modules: module \`x\` declares the
     retention policy \`y\` twice`), thay vì trang lỗi 500 mỗi lần đọc.
5. **Lọc loại con của trường tự định nghĩa phân biệt hoa thường**: `SUBSTR(target, 1, n) = '<loại>.'` thay `LIKE
   '<loại>.%'` (SQLite so `LIKE` không phân biệt hoa thường với chữ ASCII, nên `Shop.kind.x` lọt vào lượt quét và tốn
   ngân sách quét). `SUBSTR` có trên cả hai database, đếm ký tự trên cả hai.
6. Kiểm quyền trước tham số trang ở `compliance` `documents/{id}/acceptances`, `orders` `returns` và `reviews`
   (403 trước 422, khớp `x-permissions`).
7. Hướng dẫn nâng cấp cho client: `docs/guides/list-pages-upgrade.md` + `.vi.md`; changelog
   `p9-list-pages-final.changed.md` / `.added.md`.

Lý do: giữ một hình dạng danh sách cho mọi endpoint, luật gate không còn dòng nợ "có trước luật"; danh sách gộp từ
module khác vẫn phân trang trong database của từng module mà không phá module viết ngoài repo.

Đã cân nhắc: thêm trường khoá vào `Referrer` (kiểu công khai có trường `pub`) — loại: phá code dựng `Referrer` bằng
struct literal của module ngoài; nối các nguồn tuần tự với con trỏ (module, con trỏ của nguồn) — loại: khác cách gộp
của hai danh sách gộp đã có, và không cho `order=asc` / `desc` một nghĩa chung; trần riêng cho xung đột khuyến mãi —
hoãn, xem mục 4.
