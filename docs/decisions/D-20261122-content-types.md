## D-20261122-content-types · Loại nội dung tự định nghĩa: mục `kind = custom` trên bảng mục sẵn có, trường qua đích con `content.type.<key>` của kernel — 2026-11-22 — agent quyết

Chủ đề: Module nội dung, file, liên lạc

Quyết định (bước 67 mục "loại nội dung tự định nghĩa", phần chính; chi tiết hoá D-235 và mục 4a của
`docs/steps/67-page-builder.md`):

1. `content_types` (khoá `snake_case`, nhãn số ít / số nhiều theo ngôn ngữ, biểu tượng trong danh sách 12 tên,
   `routable` + `path_prefix`, `body` `none` / `rich` / `layout`, `has_seo`, `listable`, `active`, website). Quyền
   mới `content.types.manage`. Tối đa 50 loại mỗi khách (trần máy chủ; hạn mức theo gói để phần sau). Đổi khoá khi
   loại đã có mục → 409 `content.type_key_locked`; xoá loại còn mục → 409 `content.type_in_use`; tiền tố trùng trong
   một website → 409 `content.path_prefix_taken`.
2. Mục của loại là `content_entries` với `kind = custom` và cột mới `type_id` (cả trên `content_published`), tạo bằng
   `POST /entries` (`type_id` bắt buộc) — dùng nguyên nháp, revision, duyệt, hẹn giờ, bản dịch, SEO, chuyển hướng.
   `parent_key` của mục là ID loại, nên slug không trùng trong một loại.
3. Trường riêng: kernel thêm `CustomFieldTarget::with_subtypes()` (chỉ thêm vào hợp đồng, D-038) — đích khai một lần
   (`content.type`) nhận loại con `content.type.<key>` do khách tạo lúc chạy, mỗi loại con một bộ trường, cùng
   permission. Không có bộ trường thứ hai trong `content`.
4. Đường dẫn công khai: loại có trang → `/<path_prefix>/<slug>`; loại không có trang → đường dẫn nội bộ `~<key>/<slug>`
   (không bắt đầu bằng `/`): khoá duy nhất của đường dẫn vẫn đúng, `GET /public/resolve` (chỉ nhận `/…`) và sitemap
   không bao giờ trả nó. Loại tắt: `resolve`, sitemap và danh sách công khai lọc bằng subquery loại đang bật — mục
   ngừng công khai, không mất dữ liệu.
5. `GET /public/types/{key}/entries?locale` (công khai, `Page`): chỉ loại đang bật và `listable`, bản đã đăng của
   website đang xem, mới nhất trước; trường tự định nghĩa trả đúng trường `public` của loại.
6. Để phần sau (mục riêng trong ROADMAP): trang chi tiết dựng sẵn (`detail_layout`, khối `content.entry_field`),
   khối `content.type_list`, lọc `cf.<key>` ở danh sách công khai, AI gợi ý bộ trường, hạn mức theo gói.

Lý do: một bảng mục cho mọi loại giữ mọi cơ chế biên tập sẵn có mà không nhân đôi code; trường đi qua kernel như D-235
đã chốt.

Đã cân nhắc: bảng mục riêng cho loại tự định nghĩa (phải làm lại nháp, revision, dịch, hẹn giờ); đích trường theo ID
loại (khoá trường của kernel phải là `snake_case`, và khoá loại dễ đọc hơn cho người làm giao diện); loại không có trang
không có đường dẫn (cột `path` của bản công khai là khoá duy nhất bắt buộc).
