## D-20261119-step55-banners · Bước 55: banner trỏ tới đối tượng qua sổ đối tượng, permission `content.banners`, phiên bản bằng `updated_at` — 2026-11-19 — agent quyết

Chủ đề: Module nội dung, file, liên lạc

Quyết định — những chỗ cài đặt khác đặc tả `docs/steps/55-storefront.md` (viết trước khi có `menus`, D-238):

1. **Không làm điểm cắm `ContentLinks` / `LinkTarget`.** Link của banner và `page_targets` dùng **sổ đối tượng của
   kernel** (`ResourceDirectory`, D-123) như mục menu của `menus` (D-238): link là `url` hoặc `{kind, id}` với `kind`
   là loại đã đăng ký (`content.entry`, `content.term`, `catalog.product`, `catalog.collection`,
   `promotions.promotion`…). Lưu banner: loại chưa module nào đăng ký → `content.link_kind_unavailable`, đối tượng
   không có → `content.link_target_missing`. Đọc công khai: `resolve_many` theo lô mỗi loại một lần; đối tượng không
   còn `public` (sản phẩm ngừng bán, chương trình hết hạn) thì bỏ banner. Module đăng ký sau chỉ cần đăng ký loại
   của mình vào sổ — một cách làm cho mọi module, không thêm trait mới. Chủ sở hữu đối tượng cho banner gian hàng
   (D-233, mục 7) sẽ thêm vào sổ khi làm mục đó.
2. **Permission mới `content.banners`** (scoped) cho banner, thay vì dùng `content.manage`. `content.manage` không
   scoped và đang mở danh mục, thẻ, tác giả, chuyển hướng cho cả khách; `Principal::require` cho qua ở mọi độ sâu,
   nên đổi nó thành scoped sẽ để người của một cửa hàng sửa chuyển hướng của cả khách. Vị trí (cấu hình chung của
   khách) vẫn cần `content.manage`. Role mẫu chủ gian hàng (D-233) sẽ nhận `content.banners` độ sâu `unit`.
3. **Phiên bản là `updated_at`** (ETag / `If-Match` của kernel, `autoxide_kernel::version`), không thêm cột
   `version` — như mọi bảng khác của repo.
4. **`org_id` của banner luôn có giá trị**: không gửi = nút gốc (= ID khách, "mọi cửa hàng của khách"), như menu
   (D-238). Nhờ vậy `by_org` của tách / gộp khách và lọc theo nhánh dùng một cột không NULL.
5. **Đọc công khai dùng website của request** (`Site`, tham số `site` hay tên miền) thay cho `org_id`: banner của
   website đó và mọi nút tổ tiên, như mọi endpoint công khai khác của `content`.
6. **Bộ vị trí chuẩn** gieo bằng `Module::seed_tenant` chỉ khi khách chưa có vị trí nào (một transaction), nên chạy
   lại không trùng và không gieo lại vị trí khách đã xoá. Khách có sẵn (vd khách mặc định của bản cài một khách) tự
   thêm bằng `POST /api/v1/content/placements/presets` (chỉ thêm khoá còn thiếu).
7. **Lựa chọn đặc tả để agent tự quyết:** banner chỉ ảnh (không video) ở bước này; quy tắc bộ sưu tập tối đa 10
   điều kiện; chính sách mặc định của banner gian hàng: duyệt trước, 5 banner.
8. **Hai mục để lại** đúng như đặc tả cho phép: khối `content.placement` (chờ sổ khối của bước 67) và banner gian hàng
   người bán (chờ bước 46).

Lý do: một cách làm cho mỗi việc (sổ đối tượng đã giải bài toán "link tới đối tượng của module đăng ký sau" cho
menu), không mở rộng quyền ngoài ý muốn, giữ quy ước phiên bản và `org_id` sẵn có.

Thay: phần `ContentLinks` của D-219 và của đặc tả bước 55.
