## D-20261123-site-health-daily · Sức khoẻ website phần sau: job hằng ngày, đếm 404 theo ngày không lưu người xem, tool MCP của trang dựng từ khối — 2026-11-23 — agent quyết

Chủ đề: Module nội dung, file, liên lạc

Quyết định (bước 67, phần để sau của D-20261122-site-health):

1. **Đếm 404.** `GET /public/resolve` trả `content.not_found` thì cộng một lượt vào `content_not_found_paths`
   (website, SHA-256 của ngôn ngữ + đường dẫn, ngày UTC dạng chuỗi `YYYY-MM-DD`, `hits`): một câu `UPDATE` cộng
   dồn (`rows::adjust`), lần đầu trong ngày thì `INSERT … ON CONFLICT DO NOTHING` — chạy giống nhau trên SQLite và
   Postgres. Không lưu IP, user agent hay gì về người xem. Đường dẫn bỏ `?…`/`#…` và `/` cuối, dài quá 512 thì
   không đếm; mỗi website tối đa 1.000 đường dẫn khác nhau mỗi ngày (bot quét ngẫu nhiên không làm bảng phình);
   giữ 30 ngày. Lỗi khi đếm chỉ ghi log, không đổi câu trả lời 404. Số đo `content_not_found_total` không có nhãn.
2. **Gợi ý chuyển hướng.** Kiểm tra sức khoẻ thêm loại `not_found_path`: đường dẫn có ít nhất
   `AUTOXIDE_CONTENT_SITE_HEALTH_NOT_FOUND_MIN_HITS` (mặc định 5) lượt trong 7 ngày, chưa có chuyển hướng, vẫn chưa
   có trang, và có trang công khai cùng ngôn ngữ với slug gần (Levenshtein ≤ nửa độ dài) — `entry_id` là trang
   gợi ý, `fix` là lời gọi `POST /redirects`. Người bấm tạo; không tự tạo chuyển hướng. Không có trang gần thì
   không báo (`Finding.entry_id` giữ bắt buộc, không đổi hợp đồng API).
3. **Job hằng ngày** `content.site_health_daily` (lịch 24 giờ): mọi website có bài đã đăng hay có lượt 404 được
   kiểm như `POST /site-health/{org_id}/run` (`checked_by = null`), trừ website đã kiểm trong 20 giờ; một website
   lỗi không chặn website khác; rồi xoá dòng đếm cũ hơn 30 ngày.
4. **Tool MCP** (cùng service, cùng kiểm quyền theo nút với REST — luật kiến trúc 8): `content.list_blocks`,
   `get_page_layout`, `update_page_layout` (cả trang hay JSON Patch, `version` như `If-Match`),
   `list_theme_variants`, `save_theme_variant` (luôn là nháp; bật cần người), `list_types`, `save_type`,
   `compose_page`, `edit_page` (AI, tính hạn mức như REST), `site_health`. Thân bản chủ đề / loại nội dung nhận
   object `fields` đọc bằng đúng kiểu request của REST (schema của tool không được có `$ref`).

Lý do: website cần biết đường dẫn nào người ta hỏi mà không có (link cũ, gõ sai) để sửa bằng chuyển hướng, mà
không lưu dữ liệu cá nhân (Nghị định 13/2023/NĐ-CP, Luật Bảo vệ dữ liệu cá nhân 2025 — cần đối chiếu văn bản gốc);
đếm theo ngày cộng dồn giữ ghi trên đường công khai ở một câu, bảng có trần.

Đã cân nhắc: gom đếm trong bộ nhớ rồi ghi định kỳ (mất số khi tiến trình dừng, nhiều tiến trình phải gộp); lưu
từng lượt (bảng phình, lộ thói quen người xem); tự tạo chuyển hướng (có thể chuyển nhầm trang).
