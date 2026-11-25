## D-20261125-list-pages-kernel · Tham số trang: mô tả ở parameter, ba kiểu query chung một chỗ, luật `sort` — 2026-11-25 — agent quyết

Chủ đề: HTTP, REST, OpenAPI

Quyết định: bổ sung D-20261125-list-pages trước khi đổi đại trà các danh sách còn lại (bước 70 P9 mục 3, PR đại trà
đầu), theo vòng kiểm của phiên điều phối cho #517:

1. **Tài liệu OpenAPI của `limit` / `cursor` / `order` / `count` dựng tay ở một chỗ** (`pagination/params.rs`, cài
   `IntoParams` cho `PageQuery`, `NaturalOrderPageQuery`, `SortChoicePageQuery`): mô tả nằm ở `parameter.description`
   (chỗ Swagger UI / Scalar hiển thị và agent đọc); `order` là enum chuỗi thuần, có `default` ở hai kiểu đầu.
   Derive không làm được: `inline` + `default` ra `allOf` hỏng kiểu SDK, `schema_with` bỏ qua doc comment.
2. **Luật gate `api-param-description`** (`cargo xtask check`): mọi tham số (đường dẫn, query, header) của `/api/v1`
   có `description` không rỗng. 35 tham số thiếu sẵn trên `main` (25/11/2026: `status`, `kind`, `org_id`…) ghi ở
   `xtask/api-shape-allow.txt` dạng `api-param-description GET <đường dẫn>?<tên>`; PR đại trà của module nào thêm mô
   tả và xoá dòng của module đó; PR cuối của đợt làm rỗng phần này.
3. **Một khai báo trường cho ba kiểu**: `NaturalOrderPageQuery` và `SortChoicePageQuery` bọc `PageQuery`
   (`#[serde(transparent)]`, trường riêng tư). Thêm tham số trang sau này sửa `PageQuery` + `params.rs`. Trường riêng
   tư nên `SortChoicePageQuery` chỉ ra `PageRequest` qua `into_request(mặc định)`, không vòng qua `PageQuery::new`.
   `PageQuery` giữ trường `pub` (`#[non_exhaustive]` đã chặn dựng bằng literal ngoài kernel): `catalog` và `comments`
   đang gán `paging.order` để đổi mặc định theo `sort` — đúng việc của `SortChoicePageQuery`; khi hai module đó đổi
   sang nó trong PR đại trà, PR cuối của đợt cân nhắc đóng trường (ghi trong quyết định của PR đó).
4. **`page_of` nhận khoá `impl Fn(&T) -> K` với `K: AsRef<str>`** (`&'static str`, `String`…), không bắt `String`:
   danh sách dựng từ hằng không cấp phát, và chữ ký không phải đổi sau 1.0.
5. **Luật `sort`** (bổ sung mục 2 của D-20261125-list-pages):
   - Giá trị `sort` khi client không gửi: giá trị `#[default]` của enum `sort` của endpoint; mô tả tham số `sort` ghi
     "(the default)" ở giá trị đó và thứ tự mặc định của từng giá trị ("newest first" / "A→Z unless `order` says
     otherwise").
   - Enum `sort` có hàm `default_order()` (thời gian → `Order::Desc`; tên, vị trí, mã, kích thước → `Order::Asc`);
     handler và tool truyền nó vào `into_request`.
   - Điều kiện của "`created_at` → `fetch_page` theo id": `created_at` luôn đặt lúc sinh id của dòng (cùng một lần
     tạo). Bảng nào có `created_at` đặt khác (nhập dữ liệu giữ thời điểm gốc, dời khách giữ thời điểm cũ, sửa tay) thì
     danh sách `sort=created_at` của nó dùng `fetch_page_by(created_at)` với index (cột lọc…, created_at, id).
   - Endpoint mẫu cho `SortChoicePageQuery`: `media` — `GET /api/v1/media` (`created_at` / `name` / `size`),
     `/media/folders`, `/media/tags` (mặc định `name`, nay A→Z; trước theo `PageQuery` là Z→A); `content` —
     `/content/entries` (hai giá trị đều là thời gian).
6. **Mẫu tool MCP danh sách**: `media.list` — cùng tham số với REST (`sort`, `limit`, `cursor`, `order`), dựng
   `PageRequest` bằng cùng hàm với REST, kết quả `Page`; test `tool_danh_sach_tra_dung_trang_nhu_rest`
   (`modules/media/src/http/tests/pages.rs`) so `data` và `pagination` của tool với REST rồi theo `next_cursor` của
   tool tới hết.

Lý do: thiếu `parameter.description` thì trang tài liệu và agent không thấy mặc định (vòng kiểm thứ tư #517: số tham
số thiếu mô tả từ 35 lên 201); ba struct trùng trường thì thêm tham số phải sửa ba chỗ và có đường vòng qua
`into_request`; khoá `String` của `page_of` là hợp đồng công khai khó đổi sau 1.0; chép mẫu ra hơn trăm endpoint trước
khi chốt các điểm này thì phải sửa lại cả trăm chỗ.

Đã cân nhắc: bước `Modify` của OpenAPI chép mô tả lên parameter — loại: sửa sau khi sinh, xa chỗ khai, và vẫn cần
derive đúng; đóng trường của `PageQuery` ngay — hoãn: phải sửa `catalog` / `comments` ngoài cụm của PR này; khoá
`Cow<str>` cho `page_of` — loại: `AsRef<str>` nhận cả `Cow`, `&str`, `String`.
