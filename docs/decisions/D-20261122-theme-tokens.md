## D-20261122-theme-tokens · Bộ giao diện website: token phẳng theo nút, ghi đè kế thừa, danh sách font cho phép, tương phản chỉ cảnh báo — 2026-11-22 — agent quyết

Chủ đề: Module nội dung, file, liên lạc

Quyết định (bước 67 mục "bộ giao diện", chi tiết hoá D-229 và mục 3 của `docs/steps/67-page-builder.md`):

1. Token là object **phẳng** tên → giá trị (`color.primary`, `font.body`, `radius`…), 19 token cố định: 10 màu
   `#rrggbb`, `font.heading` / `font.body` trong danh sách 12 font có đủ dấu tiếng Việt (`system`, `Inter`,
   `Roboto`, `Open Sans`, `Noto Sans`, `Noto Serif`, `Be Vietnam Pro`, `Montserrat`, `Lora`, `Merriweather`,
   `Playfair Display`, `Source Sans 3`), `font.size_base` 12–24, `content_width` 640–1920, `radius`, `spacing`,
   `button.style` theo danh sách, `logo_media_id` / `background_media_id` là ảnh công khai của `media`. Tên lạ,
   giá trị sai: 422, mọi lỗi một lần (D-083). Không CSS, không HTML.
2. `content_themes` một dòng mỗi (khách, nút) chỉ giữ **phần ghi đè** của nút; token hiệu lực = mặc định của hệ
   thống ⊕ nút gốc ⊕ … ⊕ nút của website (theo `Site::lineage`, một câu truy vấn). Mặc định của hệ thống luôn đủ
   19 token và đạt tương phản AA, nên website chưa chỉnh gì vẫn đúng.
3. Tương phản WCAG 2.x mức AA (4,5:1) kiểm trên **token đã ghép** cho 5 cặp chữ / nền; không đạt thì trả
   `warnings`, vẫn lưu (đặc tả: cảnh báo, không chặn).
4. `GET` / `PUT /api/v1/content/themes/{org_id}`, permission mới `content.themes.manage` (theo nút, như
   `content.banners`), `PUT` bắt buộc `If-Match` (nút chưa có dòng: phiên bản mốc 0 hay `*`); hai người cùng tạo dòng
   đầu tiên → bên sau 412. Event `content.theme_changed`. Bảng đi theo nút khi tách khách (`by_org`).
5. Đọc công khai token hiệu lực để sau, trong `GET /public/site` (mục "đầu trang và chân trang" của bước 67) —
   cùng chỗ với chủ đề theo lịch và ETag tới mốc đổi chủ đề.

Lý do: token phẳng gán thẳng thành biến CSS, AI và giao diện chỉ cần biết tên; lưu phần ghi đè (không chép cả bộ)
để đổi ở chuỗi là mọi cửa hàng chưa đè đổi theo; danh sách font cố định tránh tải font lạ và font thiếu dấu.

Đã cân nhắc: dùng cài đặt theo khách của kernel (chỉ theo khách, không theo nút); lưu cả bộ token ở mỗi nút (đổi
ở chuỗi không lan xuống); chặn lưu khi tương phản kém (đặc tả chỉ đòi cảnh báo, và màu thương hiệu có khi buộc
phải dùng).
