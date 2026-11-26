## D-20261126-list-pages-cluster-3 · Danh sách cụm 2: cột id cho wishlist và mẫu thông báo, chỗ dùng `page_of` — 2026-11-26 — agent quyết

Chủ đề: HTTP, REST, OpenAPI

Cách đánh số: "cụm" đếm các lượt chuyển module (cụm 1 = #527, cụm 2 = PR này, changelog
`p9-list-pages-cluster-2`); hậu tố của ID quyết định đếm các quyết định của đợt (D-20261126-list-pages-cluster-2 là
quyết định của #529, không chuyển module nào), nên quyết định của cụm 2 mang hậu tố `cluster-3`.

Quyết định: ghi lại các lựa chọn của PR đại trà cụm 2 (bước 70 P9 mục 3: plans, tax, organizations, analytics, kernel
`/me/*`, `customers/me/wishlist`, `notifications/templates`), bổ sung D-20261125-list-pages:

1. **Cột `id` UUIDv7 cho bảng khoá ghép** (`customers` m0013, `notifications` m0011): thêm cột nullable, điền theo lô
   500 dòng `id IS NULL` bằng `Uuid::now_v7()` sinh trong ứng dụng (wishlist theo `added_at` rồi khoá chính, mẫu theo
   kind / kênh / ngôn ngữ — `now_v7` tăng dần trong một tiến trình nên thứ tự id giữ thứ tự cũ), rồi unique (tenant_id,
   id) + index trang. Postgres đổi cột thành `NOT NULL`; SQLite để nullable (đổi phải dựng lại bảng), như tiền lệ
   `comments_targets.uid` — code luôn ghi id khi tạo, entity khai `id: Uuid`. `id` chưa trả ra API.
2. **Chỗ dùng `page_of`** (danh sách không nằm trong database, hay bị chặn theo cấu trúc):
   - dựng từ code: hạn mức `plans` usage, cài đặt `organizations` (khách và nút), loại event và phễu `analytics`,
     `kernel` `/me/permissions`;
   - bị chặn theo cấu trúc: tổ tiên của nút `organizations` (độ sâu cây);
   - dữ liệu từ database, có lý do: năm bảng thuế (`tax` jurisdictions / categories / rates / withholding-rates /
     rules — dữ liệu nền chung đã nằm trọn trong bộ nhớ đệm của bộ máy tính thuế).
   - Năm bảng xếp hạng `analytics` (pages / sources / campaigns / types / objects) KHÔNG dùng `page_of`: keyset ngay
     trong SQL trên tổng của nhóm — `HAVING SUM(x) < n OR (SUM(x) = n AND value > v) ORDER BY SUM(x) DESC, value
     LIMIT limit + 1` (đảo chiều khi `order=desc`), chạy được trên cả hai database; `count=true` đếm trên subquery
     `GROUP BY`. Con trỏ dựng bằng `text_key_cursor` / `PageRequest::text_key_after` mới của kernel (khoá chữ
     `<tổng>\u{1f}<giá trị>`) — dùng cho mọi danh sách sắp theo một biểu thức không phải cột. Số nhóm của khoảng ngày
     theo dữ liệu khách (đường dẫn trang, đối tượng — có thể hàng trăm nghìn) nên không được kéo hết về bộ nhớ.
3. **Thứ tự đổi có chủ đích** (changelog `p9-list-pages-cluster-2.changed.md`): `plans` overrides mới nhất trước,
   store products theo product_id; `tax` packs gói nhập gần nhất trước; `organizations` cửa hàng công khai cùng
   `position` thì theo id nút; mẫu thông báo theo loại rồi thứ tự ghi lần đầu; xếp hạng `analytics` `limit` theo quy
   ước chung (1–200, mặc định 50).
4. `organizations` unit-types không có cột id (khoá chính tenant_id + code): keyset theo `code` với cột phụ `tenant_id`
   — đúng vì mọi truy vấn chỉ lọc một khách và `code` duy nhất trong khách.

Lý do: giữ một hình dạng danh sách cho mọi endpoint (D-20261125-list-pages) mà không đổi tính đúng; bảng khoá ghép
không có cột phá hoà ổn định cho keyset nên cần id.

Đã cân nhắc: keyset theo (`added_at`, product) cho wishlist — loại: gộp có thể ra hai dòng cùng `added_at`, sót dòng ở
mép trang; dựng lại bảng SQLite để có `NOT NULL` — hoãn: rủi ro migration lớn hơn lợi ích khi code đã luôn ghi id.
