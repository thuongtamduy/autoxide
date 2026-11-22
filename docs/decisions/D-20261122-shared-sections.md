## D-20261122-shared-sections · Khu dùng chung: mở rộng lúc đọc công khai, đếm trang dùng bằng quét chữ khi sửa / xoá, không lồng — 2026-11-22 — agent quyết

Chủ đề: Module nội dung, file, liên lạc

Quyết định (bước 67 mục "khu dùng chung", chi tiết hoá mục 4 của `docs/steps/67-page-builder.md`):

1. `content_shared_sections` (theo nút, quyền `content.read` / `content.write` theo nút như bài viết) giữ một bố cục
   khối, kiểm như thân `layout` của trang; khu **không chứa khu** (`layout: nested`) nên không có lồng hay vòng.
2. Trang đặt khu bằng khối `content.shared_section {id}`; lưu trang kiểm khu có thật (cùng chỗ kiểm bản ghi nhúng).
   `GET /public/resolve` thay khối bằng nội dung **hiện tại** của khu — một câu truy vấn cho mọi khu của trang —
   nên sửa khu là mọi trang đổi theo, không đăng lại trang. Khối con mang `id` = `<id khối khu>.<id trong khu>`:
   `blocks_data` không trùng khi một khu đặt hai lần; ảnh trong khu vào `body_media`.
3. **Không có bảng `content_shared_section_uses`** như bảng dữ liệu của đặc tả: số trang dùng khu chỉ cần khi sửa
   (event `content.shared_section_changed` mang `pages`) và khi xoá (409 `content.shared_section_in_use` kèm
   `count`), tính bằng tìm ID khu trong thân trang của khách (bản nháp và bản đăng, `LIKE`, chạy như nhau trên hai
   database). Hai việc này hiếm và không ở đường đọc công khai; giữ bảng phụ thì mọi đường ghi thân trang (sửa,
   khôi phục revision, dịch, tạo từ mẫu, xoá) đều phải cập nhật nó.

Lý do: đúng yêu cầu "sửa khu là mọi trang đổi, xoá đang dùng → 409" với ít chỗ phải giữ đồng bộ nhất.

Đã cân nhắc: bảng `uses` (đếm nhanh, nhưng dễ lệch khi quên một đường ghi); chép nội dung khu vào trang lúc lưu
(sửa khu phải đăng lại mọi trang); cho khu lồng khu (cần chặn vòng và giới hạn độ sâu thêm một lớp).
