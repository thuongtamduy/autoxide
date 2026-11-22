## D-20261122-site-health · Kiểm tra sức khoẻ website: chạy theo yêu cầu, một dòng kết quả mỗi website, nguồn hộp việc — 2026-11-22 — agent quyết

Chủ đề: Module nội dung, file, liên lạc

Quyết định (bước 67 mục "kiểm tra sức khoẻ website", phần đầu; chi tiết hoá mục 8 của `docs/steps/67-page-builder.md`):

1. `POST /api/v1/content/site-health/{org_id}/run` (`content.write` ở nút, tối đa một lần mỗi giờ mỗi website →
   429 `content.site_health_too_soon`) quét tối đa 5.000 bản đã đăng của website bằng vài câu truy vấn cho cả
   website: thiếu mô tả SEO (trừ trang `noindex`), tiêu đề trùng trong cùng ngôn ngữ, ảnh thiếu `alt` (thân `rich`
   và khối ảnh / ảnh bìa / bộ ảnh), link nội bộ gãy (đường dẫn `/…` không công khai, không có chuyển hướng), bản
   ghi nhúng không còn công khai (một lần hỏi sổ đối tượng mỗi loại), trang công khai có khối đếm ngược đã qua. Mỗi
   phát hiện kèm cách sửa bằng endpoint sẵn có; tối đa 200 mỗi loại.
2. Kết quả lần gần nhất lưu một dòng mỗi website (`content_site_health`), đọc bằng `GET /site-health?org_id`
   (`content.read`). Nguồn hộp việc `content.site_health` đếm phát hiện của các website trong phạm vi
   `content.write` của người gọi.
3. Để phần sau (mục riêng trong ROADMAP): job hằng ngày cho mọi website, đếm đường dẫn 404 (`content_not_found_paths`,
   cộng dồn theo ngày, không lưu IP) để gợi ý chuyển hướng, tool MCP.

Lý do: người dùng nhận việc cần sửa ngay mà không thêm việc cho đường đọc công khai; job hằng ngày và đếm 404 cần
ghi trên đường nóng `GET /public/resolve`, làm riêng với số đo.

Đã cân nhắc: kiểm mỗi lần đăng bài (chậm đường ghi, không bắt được link gãy do trang khác bị gỡ); lưu mỗi phát hiện
một dòng (nhiều dòng phải dọn, không cần cho hộp việc).
