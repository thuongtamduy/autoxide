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
   - `NaturalOrderPageQuery` (mặc định `order=asc`): danh sách có thứ tự tự nhiên — vị trí, tên, mã. Database: `fetch_page_by`
     theo cột đó + id. Mô tả endpoint nói thứ tự là gì.
   - Thứ tự theo cột chữ (tên) do collation của database quyết; chỉ cam kết ổn định và không trùng / sót trong một lần
     duyệt (khoá phụ id; so sánh cursor và `ORDER BY` cùng collation). Không module nào tự sắp chữ trong bộ nhớ để thay.
   - Đổi `order` giữa chừng chỉ đổi điểm bắt đầu trang sau (cursor không mang `order`); cursor của danh sách khác cột
     sắp hay bị sửa → 422 ở ô `cursor`. Cursor chỉ mang (cột, giá trị, id) và mọi truy vấn lọc theo khách, nên cursor của
     khách khác không lộ gì.
   - Danh sách có tham số `sort` (vd theo thời gian tạo hoặc theo tên): `SortChoicePageQuery` — `order` không có mặc định
     cố định mà theo cột đang chọn: thời gian (tạo, sửa) → `desc` (mới nhất trước); tên, vị trí, mã, kích thước → `asc`
     (A→Z, nhỏ trước). Kiểu này không có `TryFrom`: handler đọc `sort` rồi gọi `into_request(mặc định)` — không nêu mặc
     định thì không có `PageRequest`, không thể lặng lẽ ra `desc` trái tài liệu (`order` client gửi thì giữ). Mô tả
     `sort` của endpoint nêu mặc định của từng giá trị.
   - Khoá keyset của từng giá trị `sort`: `created_at` → `fetch_page` theo id (UUIDv7 sinh trong ứng dụng lúc tạo nên
     thứ tự id là thứ tự tạo; một khoá, index (cột lọc…, id) đã có) — không dùng `fetch_page_by(created_at)`. `updated_at`
     → `fetch_page_by(updated_at)` (`SortValue::Time`; cột `NOT NULL`, index (cột lọc…, updated_at, id)); dòng được sửa
     trong lúc duyệt có thể nhảy sang trang khác — chấp nhận, như mọi danh sách sắp theo giá trị đổi được. Tên, mã, vị
     trí, kích thước → `fetch_page_by` theo cột đó, index (cột lọc…, cột, id). Các endpoint `sort` đang dùng `PageQuery` (vd `media`
     items, `newsletter` subscribers: `sort=name` đang mặc định Z→A, mô tả "newest first" sai) sửa trong PR đại trà của
     module đó.
2b. **Tên kiểu chốt**: `NaturalOrderPageQuery` (không đổi nữa; tên cũ `SortedPageQuery` chỉ có trong bản nháp) — `PageQuery`
   cũng có thứ tự nên tên phải nói đúng chỗ khác: thứ tự tự nhiên. Có `with_count` như `PageQuery`. Ba kiểu query
   (`PageQuery`, `NaturalOrderPageQuery`, `SortChoicePageQuery`) khai tham số `order` trong OpenAPI dạng enum chuỗi
   thuần có mô tả: `{"type": "string", "enum": […], "description": …}`, cộng `"default"` (`desc` / `asc`) ở hai kiểu đầu;
   `SortChoicePageQuery` không có `default` (theo `sort`), mô tả nói rõ. Khai tay bằng `schema_with` (file
   `pagination/order_param.rs`): `inline` + `default` của utoipa ra `allOf` mà SDK TypeScript dịch thành `("asc" | "desc")
   & Record<string, unknown>`, còn `schema_with` thì bỏ qua doc comment của trường nên mô tả đặt trong schema. Test của
   kernel giữ dạng này (kể cả mô tả); enum `Order` mô tả trung lập.
3. **Danh sách trong database phân trang bằng keyset trong database**, không trần đọc: một `.limit(N)` cứng trước khi trả
   danh sách là mất dữ liệu trong im lặng. Giới hạn nghiệp vụ thật (chặn lúc tạo, có mã lỗi) giữ ở chỗ ghi. Mỗi danh sách
   có index khớp: cột lọc (`tenant_id`, `org_id`…) + cột sắp + id; thiếu thì thêm migration. Index mới bao index cũ (index
   cũ là tiền tố của nó) thì xoá index cũ trong cùng migration (`down` tạo lại).
   - Cột sắp phải `NOT NULL` (NULL làm sót dòng, và hai database xếp NULL khác nhau): `fetch_page_by` từ chối cột
     nullable bằng lỗi nội bộ; cột tuỳ chọn không dùng làm cột sắp.
   - Điều kiện keyset của `fetch_page_by` có cận trước: `col >= v AND (col > v OR id > id0)` (giảm dần: `<=`, `<`), để
     database nhảy thẳng tới điểm trong index. Đo bằng EXPLAIN ngày 25/11/2026 trên bảng (tenant_id, org_id, name, id),
     index cùng cột: dạng cũ `col > v OR (col = v AND id > id0)` — SQLite `SEARCH … (tenant_id=? AND org_id=?)` (lọc từng
     dòng), Postgres `Index Cond` chỉ có tenant / org, phần còn lại là `Filter`; dạng mới — SQLite `SEARCH … (tenant_id=?
     AND org_id=? AND name>?)`, Postgres `Index Cond … AND name >= …`. `fetch_page` (chỉ theo id) đã là một điều kiện
     phạm vi nên không đổi.
   - Kiểm cột nullable đọc định nghĩa entity: SeaORM coi cột là nullable khi kiểu trường viết `Option<…>` hay có
     `#[sea_orm(nullable)]`; viết `std::option::Option<…>` thì lọt. Entity trong repo viết `Option<…>`.
   - "Không trần đọc" áp cho danh sách trả qua API / MCP. Việc nội bộ chỉ cần N dòng đầu (ngữ cảnh gửi AI, xem trước)
     được lấy N dòng, nhưng phải `ORDER BY` + `LIMIT` trong SQL (không lấy hết rồi cắt trong bộ nhớ, không `.limit` không
     sắp — kết quả tuỳ database), và không trả danh sách đó ra API như một danh sách đầy đủ.
   - Giới hạn đã biết: danh sách lọc theo CHUỖI nút (`org_id IN (…)`, vd câu trả lời mẫu của `support`) không có thứ tự
     sẵn trong index trên cả chuỗi — database gộp rồi sắp (SQLite `TEMP B-TREE`); đúng, nhưng chi phí theo số dòng của
     chuỗi nút chứ không chỉ cỡ trang.
3b. **Tool MCP danh sách** đổi cùng endpoint REST: tham số `limit` / `cursor` / `order` (+ `count`, + `sort` nếu REST có)
   cùng tên, cùng kiểm (dựng `PageRequest` bằng đúng kiểu query của REST), kết quả `{"data": […], "pagination": {…}}`
   như REST; test gọi cả REST và tool trên cùng dữ liệu, so trang đầu và `next_cursor`. `support` (PR 1) không có tool
   danh sách: PR đại trà đầu tiên chọn một module có tool danh sách làm mẫu và ghi tên module + test vào đây.
4. **`page_of` (phân trang trong bộ nhớ) chỉ cho danh sách không nằm trong database**: dựng từ code (sổ module,
   permission, kind), dữ liệu nền cố định trong bản build, kết quả gộp từ nhà cung cấp. Khoá chữ duy nhất, so theo byte.
   Hai phần tử trùng khoá là lỗi lập trình: lỗi nội bộ (không mất phần tử trong im lặng). Mọi chỗ dùng được liệt kê ở
   cuối mục này (cập nhật theo từng PR).
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

Chỗ dùng `page_of`: chưa có (PR 1: kernel + `support`).

Ngoại lệ (danh sách vẫn `Data<Vec<T>>`): chưa có.
