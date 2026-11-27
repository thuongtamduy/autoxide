## D-20261127-mcp-sdk-ci-check · Kiểm đăng nhập OAuth của MCP bằng SDK client MCP chính thức trong CI — 2026-11-27 — agent quyết

Chủ đề: MCP, AI, agent dùng API

Quyết định: CI có job `mcp-oauth` chạy `scripts/mcp-oauth-e2e.sh`: dựng `autoxide serve` thật (binary debug) trên SQLite
rồi PostgreSQL, và chạy `scripts/mcp-oauth/client.mjs` — client của SDK TypeScript chính thức
`@modelcontextprotocol/sdk` (ghim `1.32.2`, `package-lock.json`, chỉ ở thư mục đó) — qua trọn luồng của
D-20261126-mcp-oauth: khám phá qua `/.well-known`, đăng ký động VÀ client tài liệu metadata, PKCE, trang consent (script
đóng vai giao diện), đổi mã, `tools/list` chỉ có tool đã duyệt, gọi tool, refresh khi access token bị từ chối, thu hồi.
Job chạy ở push `main` và ở PR chạm phần liên quan (module `auth`, HTTP / MCP / access của kernel, script, workflow,
`Cargo.lock`).

Lý do: test Rust của `auth` dùng client tự viết, nên chỉ chứng minh máy chủ khớp cách hiểu của chính repo về đặc tả;
client chính thức là thứ Claude, IDE và framework agent dựa vào, nên lệch đặc tả (header `WWW-Authenticate`, metadata,
`resource`, `iss`, refresh) lộ ra ở đây. Chọn TypeScript vì CI đã có Node cho job SDK và đây là SDK tham chiếu của MCP;
viết `.mjs` thuần để không cần bước biên dịch. Thư mục riêng (không phải `sdk/typescript`) để gói SDK của Autoxide vẫn
không có phụ thuộc chạy (job `licenses`). SDK chỉ nhận `client_id` dạng tài liệu metadata khi là `https`, nên ca tài
liệu metadata đưa sẵn `client_id` (`http://127.0.0.1:…`, mở bằng `AUTOXIDE_AUTH_OAUTH_ALLOW_PRIVATE_NETWORKS` chỉ trong
bài kiểm) thay vì để SDK tự chọn.

Đã cân nhắc: client Python (`mcp`) — CI chưa có Python cho việc này, thêm một chuỗi cung ứng nữa; chạy trong `cargo
test` bằng client tự viết — đã có, không thay được client thật; chạy trong job `sdk` — job đó không chạy máy chủ và cần
nhanh; chỉ SQLite — luật số một đòi cả hai database.
