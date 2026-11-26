## D-20261126-list-pages-cluster-2 · Danh sách: kiểm quyền trước tham số trang, tool có `count`, đính chính cụm 1 — 2026-11-26 — agent quyết

Chủ đề: HTTP, REST, OpenAPI

Quyết định: bổ sung D-20261125-list-pages và D-20261125-list-pages-kernel (bước 70 P9 mục 3, PR đại trà thứ hai), theo
vòng kiểm của phiên điều phối cho #527:

1. **Kiểm quyền trước khi đọc tham số trang.** Handler danh sách gọi `principal.require(&<permission>)?` (có ở bất kỳ
   độ sâu nào — phạm vi theo nút vẫn do service lọc) TRƯỚC khi dựng `PageRequest`, để người không có quyền nhận 403
   `permission_denied` chứ không phải 422 của `limit` / `cursor` / `order`. Tool MCP đã đúng sẵn: bộ điều phối kiểm
   `requires` trước khi đọc input. Bài test độ phủ phân quyền (`crates/app/src/e2e/access.rs`) gửi thêm `limit=0` cho
   mọi operation có tham số `cursor`; 55 endpoint đang sai (đo 26/11/2026, ngoài các cụm đã đổi) nằm trong
   `PAGE_BEFORE_PERMISSION` của file đó — mỗi PR đại trà sửa và xoá dòng của cụm mình, danh sách không dài thêm.
2. **Tool danh sách nhận `count`** như REST (mục 3b của D-20261125-list-pages). Luật gate `mcp-page-count` (`cargo xtask
   check`, trên `docs/mcp-tools.json`): tool có input `cursor` phải có `count`. Tool có sẵn trước luật ghi ở
   `xtask/api-shape-allow.txt` dạng `mcp-page-count TOOL <tên>`, xoá dần theo cụm. `PageInput::with_count` của kernel
   cho tool dùng `PageInput`.
3. **Loại số liệu `reporting` duy nhất**: `Reporting::new` gộp các khai giống hệt (một module khai một loại ở hai
   event). Khai khác nhau hay hai module cùng tên đã bị `FactCatalog::new` của kernel chặn lúc khởi động, nên không
   có loại nào bị bỏ trong im lặng.
4. **Đính chính D-20261125-list-pages-kernel** (mục 2 ghi "35 tham số thiếu sẵn"): 35 là số đo trước khi đổi cụm 1;
   PR đó tự thêm mô tả cho 15 tham số, nên `xtask/api-shape-allow.txt` khi lên `main` có 20 dòng `api-param-description`.
5. **Hai ngoại lệ chờ chủ dự án duyệt** (`comments/public/counts`, `inventory/public/availability`, tra cứu theo id client
   gửi): giữ dòng `api-unpaged-list` với lý do thật thay cho "có trước luật"; được duyệt thì PR cuối chép vào danh sách
   ngoại lệ của D-20261125-list-pages.

Lý do: 403 trước 422 là thứ tự chuẩn (không lộ hình dạng tham số cho người không có quyền, và agent biết ngay phải xin
quyền chứ không sửa tham số); `count` thiếu ở tool làm tool kém REST cùng permission (luật kiến trúc 8); luật gate và
danh sách cho phép giữ việc này không tái diễn ở các cụm sau.

Đã cân nhắc: middleware của kernel kiểm `x-permissions` của route trước mọi handler — sửa cả 65 chỗ một lần nhưng đổi
cách mọi endpoint kiểm quyền (quyền thay thế, `x-hides-existence` trả 404), vượt phạm vi đợt này; ghi để cân nhắc
riêng. Sửa cả 65 endpoint trong PR này — loại: chạm module của các cụm sau và của phiên khác.
