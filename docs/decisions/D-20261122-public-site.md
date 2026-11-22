## D-20261122-public-site · Khung website: đầu / chân trang có nháp và đăng, `GET /public/site` cache trong bộ nhớ tới mốc đổi chủ đề, điểm cắm `SiteExtensions` — 2026-11-22 — agent quyết

Chủ đề: Module nội dung, file, liên lạc

Quyết định (bước 67 mục "đầu trang và chân trang của cả website", chi tiết hoá D-234 và mục 3b, 3c của
`docs/steps/67-page-builder.md`):

1. `content_site_regions` (website, `header` / `footer`, ngôn ngữ, ô `base` hay ID bản chủ đề — chữ, không NULL,
   để khoá duy nhất đúng trên hai database). Sửa vào `draft_layout` (`PUT /site-regions?org_id&region&locale&
   variant_id`, kiểm như bố cục trang), `POST /site-regions/{id}/publish` mới thành bản công khai. Quyền
   `content.themes.manage` theo nút.
2. Chọn khu cho website: ô của bản chủ đề đang chạy trước, rồi ô `base`; trong mỗi ô, nút gần website nhất trước
   (cửa hàng đè chuỗi); trong mỗi nút, ngôn ngữ hỏi rồi ngôn ngữ mặc định của website.
3. `GET /public/site?locale` (công khai): token đã ghép bản chủ đề, hiệu ứng, `variant`, `next_change_at`,
   `header` / `footer` (`layout` + `blocks_data`, dữ liệu khối của hai khu trong một lần gọi sổ khối, `id` mang
   tiền tố khu khi gọi), `legal_footer` (khối `compliance.legal_footer`, ngoài bố cục, dưới footer), `extensions`.
4. Cache trong bộ nhớ theo (khách, website, ngôn ngữ hỏi), hết hạn tại `min(now + 30 giây, next_change_at)`; đời
   cache (số đếm) tăng mỗi lần sửa theme / bản chủ đề / khu trên máy này nên đổi thấy ngay, máy khác chậm tối đa
   30 giây. Cache nóng: không câu truy vấn nào của `content` (test đếm câu SQL). `ETag` = dấu vân tay của câu trả
   lời; `Cache-Control: max-age` = số giây tới `next_change_at`, trần 300. Trần 10.000 dòng cache mỗi tiến trình.
5. Điểm cắm `content::api::SiteExtensions` / `SiteExtension` (chỉ thêm vào hợp đồng công khai của `content`):
   module đăng ký sau `content` (vd `compliance` với đồng ý cookie, bước 37) tự cắm phần của mình, trả ở
   `extensions.<module>`; phần lỗi chỉ ghi log, không hỏng khung.
6. Số đo (50 bản chủ đề, bản debug, máy phát triển): SQLite nguội p99 ≈ 6,0 ms, nóng ≈ 1,1 ms; Postgres nguội
   ≈ 15,2 ms, nóng ≈ 0,83 ms — ghi ở `docs/BENCHMARKS.md`.

Lý do: khung dùng cho mọi trang nên phải rẻ khi nóng và đổi đúng lúc chủ đề đổi mà không ai phải xoá cache.

Đã cân nhắc: cache chung giữa các máy (Redis — thêm hạ tầng; 30 giây chậm là chấp nhận được với đặc tả); không
cache mà để CDN lo (mỗi CDN miss tốn 4–5 câu truy vấn, và máy chủ không có CDN thì mọi request đều tốn); dùng
NULL cho ô theme gốc (khoá duy nhất không chặn trùng trên cả hai database).
