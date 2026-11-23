## D-20261123-content-types-later · Loại nội dung, phần sau: trang chi tiết do `content` điền, lọc công khai theo index của bản đăng, hạn mức theo gói — 2026-11-23 — agent quyết

Chủ đề: Module nội dung, file, liên lạc

Quyết định (bước 67 mục "loại nội dung tự định nghĩa, phần sau"; tiếp D-20261122-content-types, chi tiết hoá D-235):

1. **Trang chi tiết dựng sẵn:** cột `content_types.detail_layout` (migration `m0031_type_detail_layout`, JSON trong
   `text`), đặt bằng `POST /types` / `PATCH /types/{key}` (`null` bỏ), chỉ loại `routable`; kiểm bằng đúng bộ kiểm bố
   cục của trang. Khối `content.entry_field {field, label?, style?}` chỉ nhận trường có sẵn (`title`, `excerpt`,
   `body`, `featured_media`, `published_at`, `authors`) hay trường tự định nghĩa đang bật của loại; đặt khối này
   trong thân trang thường → 422 `block_not_allowed`; mô hình AI không thấy nó. `GET /public/resolve` của mục thuộc
   loại đang bật có bố cục mẫu trả thêm `detail {layout, blocks_data}`; dữ liệu khối `content.entry_field` là
   `{field, value}` do `content` điền (nó biết mục đang xem) — không đổi `BlockRequest` của kernel.
2. **Khối `content.type_list {type, filters[], sort, limit ≤ 24, card_fields[], title?, layout?}`** (`has_data`): dữ
   liệu qua `BlockData` của `content`, mỗi khối một lần đọc danh sách công khai của loại (cùng service với
   `GET /public/types/{key}/entries`); thẻ chỉ có trường `public` được chọn; loại tắt, không còn hay bộ lọc hết hợp
   lệ → `null` (trang vẫn trả). Mô hình AI dựng trang thấy tối đa 20 loại `listable` kèm trường công khai.
3. **Lọc `cf.<key>` ở danh sách công khai** chỉ trên trường `public` + `filterable` (trường nội bộ trả như không có),
   theo giá trị của **bản đăng**: bảng index riêng `content_published_custom_index` (migration
   `m0032_published_custom_index`, tạo bằng `custom_fields::index_migration` với tên `content_published`), ghi
   cùng transaction lúc đăng (`record_id` = ID mục), bỏ lúc gỡ / xoá. Index `content_custom_index` sẵn có giữ giá trị
   nháp cho danh sách quản trị. Bản đăng trước migration chưa có index: đăng lại thì lọc được.
4. **AI gợi ý bộ trường:** `POST /types/ai/suggest {description, locale}` (`content.types.manage`, tính năng
   `content.ai_suggestions`, AI bật) trả bản nháp thân `POST /types` và thân từng `POST /kernel/custom-fields` — đã
   lọc lại theo luật của hai endpoint (khoá, biểu tượng, kiểu dữ liệu, lựa chọn); không ghi gì.
5. **Gói:** tính năng `content.custom_types` (tạo loại) và hạn mức đếm `content.custom_types` (giữ chỗ / xác nhận /
   trả như `notes`, bộ đếm thật cho job đối soát), bên trong trần máy chủ 50 loại.

Lý do: trang chi tiết cần mục đang xem — chỉ `content` có nó, nên không cần mở rộng hợp đồng kernel; lọc theo giá trị
nháp sẽ lộ chỉnh sửa chưa đăng và trả sai kết quả; bảng index thứ hai dùng lại nguyên bộ ghi và bộ lọc của kernel,
chạy như nhau trên SQLite và Postgres.

Đã cân nhắc: thêm "mục đang xem" vào `BlockRequest` của kernel (đổi hợp đồng công khai cho một khối của `content`);
lọc trên cột JSON `custom_fields` của bản đăng (hàm JSON khác nhau giữa hai database); ghi index của bản đăng vào
`content_custom_index` với ID khác (tách / gộp khách không theo được dòng cha).
