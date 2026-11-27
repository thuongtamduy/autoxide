## D-20261126-public-terms-shown · Danh mục, thẻ chỉ công khai khi có bài đang hiện trên website mang nó — 2026-11-26 — agent quyết

Chủ đề: Module nội dung, file, liên lạc

Quyết định: `GET /api/v1/content/public/terms` chỉ đếm bài đang hiện trên website (không bị ẩn — D-363 —, trên chuỗi
nút của website — D-307); `GET /api/v1/content/public/terms/{taxonomy}/{slug}` trả `404 content.term_not_found` khi chưa
có bài nào như vậy mang term. Đóng khi không chắc: term đang soạn (tên, mô tả, SEO) không lộ ra ngoài. Hệ quả có chủ
ý: mục menu trỏ tới danh mục rỗng ra trang 404 cho tới khi có bài mang danh mục đó (ghi ở `docs/CONTENT.md` EN + VI).
Sổ đối tượng (`TermResolver`) áp cùng luật ở PR sau.

Lý do: PT-11 (đợt test 2026-11) — đọc phân loại công khai lộ thuật ngữ chưa bài đã đăng nào dùng.

Đã cân nhắc: coi term có trong menu công khai là công khai — thêm một đường dò ngược menu cho mỗi lần đọc, và menu
vốn do quản trị viên chọn đưa ra; người cần trang chuyên mục trống thì đăng một bài giới thiệu.
