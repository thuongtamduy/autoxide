## D-20261125-docs-csp-pinned · CSP của `/api/docs` chỉ cho đúng file Scalar đã ghim, bỏ jsdelivr khỏi style và font — 2026-11-25 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: `DOCS_CSP` (`crates/kernel/src/http/security_headers.rs`) đổi `script-src https://cdn.jsdelivr.net` thành
URL đầy đủ của đúng file `@scalar/api-reference@1.76.4/dist/browser/standalone.js`; `style-src` chỉ còn
`'unsafe-inline'`, `font-src` chỉ còn `https://fonts.scalar.com data:`. URL viết một lần trong macro
`scalar_script_url!` (`crates/kernel/src/http/mod.rs`), dùng cho cả trang (`DOCS_HTML`) và CSP — đổi phiên bản là đổi
cả hai. Sửa L-9 của `docs/security/review-2026-11.md`.

Lý do: CSP cho cả host CDN thì kẻ tiêm được HTML vào trang có thể nạp bất kỳ gói npm nào trên jsdelivr; Subresource
Integrity chỉ bảo vệ thẻ `<script>` của chính trang. Đã thử trên Chromium (Playwright) với cả CSP cũ và mới, có mở
tìm kiếm, bấm các nút "Test Request", "Ask AI", "Configure", "Generate SDK", cuộn trang: gói 1.76.4 chỉ nạp chính nó
và ba font từ `fonts.scalar.com`, không nạp style nào từ CDN; trang hiện đủ. Các vi phạm CSP còn lại giống hệt ở CSP
cũ: `connect-src` tới `api.scalar.com` (tính năng tuỳ chọn gọi máy chủ của Scalar) và một lần thử `eval` — chặn có
chủ đích, không ảnh hưởng tới việc đọc tài liệu.

Đã cân nhắc: CSP hash (`'sha384-…'`) cho script ngoài — chỉ một số trình duyệt nhận hash cho script có `src`; tự host
file Scalar trong binary — thêm ~3 MB vào binary và việc nâng cấp, trong khi trang tắt theo mặc định.

Thay: phần "CSP của `/api/docs`" (mục 3) của D-320 — nguồn script và style không còn là cả host jsdelivr.
