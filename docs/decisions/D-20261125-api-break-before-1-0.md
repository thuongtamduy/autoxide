## D-20261125-api-break-before-1-0 · Trước 1.0, HTTP API cũng có thể bị phá vỡ ở bản minor; từ 1.0 luật "không phá vỡ `/api/v1`" áp trọn vẹn — 2026-11-25 — chủ dự án quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. **Chính sách công khai** (`docs/COMPATIBILITY.md` + `.vi.md`, câu mở đầu): trước 1.0 (`0.x`) một bản minor được
   phép phá vỡ mọi hợp đồng, **kể cả HTTP API**; mỗi chỗ phá vỡ ghi ở mục **Changed** hay **Removed** của changelog
   kèm việc cần làm thay thế. Từ `1.0.0` toàn bộ luật trong trang đó áp trọn vẹn, không đổi một chữ: không gì trong
   `/api/v1` bị phá vỡ, thay đổi phá vỡ làm thành endpoint mới, endpoint cũ khai deprecation với sunset ít nhất sáu
   tháng.
2. **Luật kiến trúc số 6 của `CLAUDE.md` vẫn là mặc định** ("thay đổi phá vỡ thì làm endpoint mới, không sửa hành vi
   endpoint cũ"). Trước 1.0, sửa tại chỗ một endpoint, bỏ một mã lỗi hay đổi hình dạng response chỉ được làm khi có
   một quyết định nêu rõ thay đổi đó và lý do; agent không tự coi "còn 0.x" là giấy phép. Từ 1.0 không có ngoại lệ.
3. **Hai thay đổi đầu tiên dùng tới quyết định này** (mỗi cái có quyết định riêng nêu chi tiết): mọi endpoint trả
   danh sách đổi sang `Page<T>` tại chỗ (bước 70 P9, chủ dự án quyết ngày 25/11/2026: "xử lý triệt để, làm một lần
   cho tất cả"), và bỏ mã lỗi `tenant_move_modules_off` khi module tắt vẫn tham gia vòng đời khách (bước 70 P9).

Lý do: câu mở đầu cũ tự mâu thuẫn — vế trước cho phép bản minor `0.x` phá vỡ mọi hợp đồng, vế sau lại nói HTTP API
đã theo luật "không phá vỡ" từ trước 1.0. Chủ dự án chọn sửa lời hứa thay vì giữ: dự án chưa tới 1.0 và chưa có
người dùng bên ngoài phụ thuộc, nên bây giờ là lúc rẻ nhất để sửa hợp đồng cho đúng; sau 1.0 mỗi chỗ chưa đúng phải
mang theo một endpoint cũ ít nhất sáu tháng và API có hai cách làm cho cùng một việc, trái tiêu chí "học một endpoint
là biết các endpoint còn lại". Giữ luật 6 làm mặc định và buộc có quyết định cho từng lần phá vỡ để việc này không
thành thói quen.

Đã cân nhắc: giữ lời hứa cũ và làm phân trang bằng endpoint mới kèm deprecation — loại, chủ dự án muốn làm một lần
cho xong và không để API mang hai kiểu danh sách sau 1.0. Chỉ ghi ngoại lệ trong quyết định của từng thay đổi mà
không sửa trang chính sách — loại, trang công khai sẽ nói một đằng trong khi changelog làm một nẻo.

Thay: phần "áp từ 1.0.0" của D-20261123-compatibility-policy được nói rõ thêm (trước 1.0 HTTP API không được miễn);
phần còn lại của mục đó giữ nguyên. Bổ sung D-059 (không thay): luật "endpoint mới thay vì sửa" áp trọn từ 1.0.
