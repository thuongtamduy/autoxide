## D-20261125-list-pages · Mọi danh sách trả `Page<T>`, sửa tại chỗ trước 1.0 — 2026-11-25 — chủ dự án quyết

Chủ đề: HTTP, REST, OpenAPI

Quyết định: chủ dự án quyết ngày 25/11/2026, nguyên văn: "chọn cách nào xử lý triệt để, làm 1 lần cho tất cả để về sau
khỏi làm lại và vướng phần này nữa". Phần diễn giải dưới đây là của phiên điều phối, agent làm theo:

1. **Sửa tại chỗ, trước 1.0** (bước 70 P9 mục 3): mọi endpoint GET trả danh sách giữ nguyên đường dẫn và đổi từ
   `Data<Vec<T>>` sang `Page<T>` (`{"data": […], "pagination": {…}}`) với `limit` / `cursor` / `order` (+ `count`). Không
   có đường dẫn `/page`, không giai đoạn hai chế độ, không việc để lại cho ngày sunset. Đây là ngoại lệ có chủ đích cho
   luật kiến trúc 6, chỉ cho đợt này, được phép theo D-20261125-api-break-before-1-0; từ 1.0 luật 6 áp nguyên vẹn. Tool MCP
   tương ứng đổi tại chỗ (cùng tên, thêm tham số trang, kết quả có `pagination`).
2. **Chọn tham số trang** (luật, để phiên sau không chọn tuỳ ý):
   - `PageQuery` (mặc định `order=desc`, mới nhất trước): danh sách không có thứ tự tự nhiên nào khác ngoài thời điểm tạo.
     Database: `fetch_page` (khoá UUIDv7).
   - `SortedPageQuery` (mặc định `order=asc`): danh sách có thứ tự tự nhiên — vị trí, tên, mã. Database: `fetch_page_by`
     theo cột đó + id. Mô tả endpoint nói thứ tự là gì.
   - Thứ tự theo cột chữ (tên) do collation của database quyết; chỉ cam kết ổn định và không trùng / sót trong một lần
     duyệt (khoá phụ id; so sánh cursor và `ORDER BY` cùng collation). Không module nào tự sắp chữ trong bộ nhớ để thay.
   - Đổi `order` giữa chừng chỉ đổi điểm bắt đầu trang sau (cursor không mang `order`); cursor của danh sách khác cột
     sắp hay bị sửa → 422 ở ô `cursor`. Cursor chỉ mang (cột, giá trị, id) và mọi truy vấn lọc theo khách, nên cursor của
     khách khác không lộ gì.
3. **Danh sách trong database phân trang bằng keyset trong database**, không trần đọc: một `.limit(N)` cứng trước khi trả
   danh sách là mất dữ liệu trong im lặng. Giới hạn nghiệp vụ thật (chặn lúc tạo, có mã lỗi) giữ ở chỗ ghi. Mỗi danh sách
   có index khớp: cột lọc (`tenant_id`, `org_id`…) + cột sắp + id; thiếu thì thêm migration.
4. **`page_of` (phân trang trong bộ nhớ) chỉ cho danh sách không nằm trong database**: dựng từ code (sổ module,
   permission, kind), dữ liệu nền cố định trong bản build, kết quả gộp từ nhà cung cấp. Khoá chữ duy nhất, so theo byte.
   Mọi chỗ dùng được liệt kê ở cuối mục này (cập nhật theo từng PR).
5. **Không tái diễn**: gate chặn endpoint GET mới trả `{"data": [...]}` không có `pagination`, trừ danh sách ngoại lệ có
   tên + lý do ở mục này (PR cuối của đợt). `CLAUDE.md` ("Luật code") nói "mọi danh sách là `Page<T>`" (phiên điều phối
   đưa vào cùng PR cuối).
6. **Giảm rủi ro cho client cũ**: changelog mục Changed ghi "Breaking" và liệt kê đủ endpoint / tool đổi kèm cách sửa;
   hướng dẫn nâng cấp EN + VI; SDK TypeScript sinh lại.

Lý do: hai kiểu danh sách trái tiêu chí "học một endpoint là biết các endpoint còn lại"; 140 endpoint `Data<Vec<T>>` (đo
25/11/2026) có danh sách lớn dần theo dữ liệu khách mà không phân trang, và danh sách bọc trần đọc mất dữ liệu trong im
lặng. Sau 1.0 không còn cơ hội sửa tại chỗ.

Đã cân nhắc: đường dẫn mới `<cũ>/page` + deprecation (kế hoạch đầu của agent) — loại: để lại hai kiểu song song và việc
dọn ở ngày sunset; giữ đường dẫn, `limit` tuỳ chọn, không truyền thì trả hết kèm header Deprecation — loại: hai hợp đồng
trên một endpoint tới ngày sunset; giữ `Data<Vec<T>>` cho danh sách có trần / cố định — loại theo lời chủ dự án ("một lần
cho tất cả"), trừ ngoại lệ có lý do về nghĩa.

Đính chính D-20261125-dormant-modules (mục 2 và 3b ghi "có `<module>_migrations`"): module "đã cài" ở một database
nghĩa là bảng lịch sử của nó có ít nhất một dòng (`migration::installed`); `migrate down` hết thì coi như chưa cài.

Chỗ dùng `page_of`: chưa có (PR 1: kernel + `support`).

Ngoại lệ (danh sách vẫn `Data<Vec<T>>`): chưa có.
