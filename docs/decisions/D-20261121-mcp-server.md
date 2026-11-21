## D-20261121-mcp-server · MCP server tự viết trong kernel thay `rmcp`, phục vụ cả bản `2026-07-28` lẫn bản cũ — 2026-11-21 — agent quyết

Chủ đề: MCP, AI, agent dùng API

Quyết định: endpoint `/mcp` và `/mcp/<module>` do kernel tự cài theo đặc tả MCP công khai (`crates/kernel/src/http/mcp/`:
`protocol.rs` — tin nhắn JSON-RPC 2.0, phiên bản, lỗi; `transport.rs` — lớp HTTP), thay thư viện `rmcp`. Hành vi giữ như
cấu hình `rmcp` trước đây: không giữ phiên, luôn trả một response JSON (không SSE), xác thực / permission / gói / chỉ đọc /
nhật ký / deprecated / `no-store` như cũ (D-068, D-374). Phục vụ **hai thế hệ** trên cùng endpoint (đặc tả 2026-07-28,
"Versioning"):

- **Bản mới `2026-07-28`** — request mang `_meta` `io.modelcontextprotocol/protocolVersion` (hay là `server/discover`):
  không bắt tay; `_meta` thiếu `protocolVersion` / `clientCapabilities` → 400, -32602; phiên bản lạ → 400, -32022 kèm
  `supported`; header `MCP-Protocol-Version`, `Mcp-Method`, `Mcp-Name` (giải `=?base64?…?=`) phải khớp thân → không thì
  400, -32020; method không có → 404, -32601; không còn `ping`; có `server/discover`. Kết quả mang `resultType: "complete"`,
  `_meta` `io.modelcontextprotocol/serverInfo`, và `ttlMs: 0` + `cacheScope: "private"` ở danh sách / `resources/read`
  (kết quả phụ thuộc quyền người gọi).
- **Bản cũ `2024-11-05` … `2025-11-25`** — `initialize` chọn đúng bản client xin (lạ hay bản mới thì `2025-11-25`);
  header `MCP-Protocol-Version` lạ → 400; thiếu → coi là `2025-03-26`; lỗi JSON-RPC trả HTTP 200 như trước.
- Chung: `Origin` có mặt mà không thuộc `AUTOXIDE_HTTP_CORS_ORIGINS` → 403; `GET` / `DELETE` → 405; `Accept` thiếu
  `application/json` hay `text/event-stream` → 406; `Content-Type` khác JSON → 415; thông báo → 202; thân > 4 MiB → 413.
- Lỗi JSON-RPC giữ nguyên thân qua lớp Problem của kernel (dấu `JsonRpcBody` trong extensions, `http/problem.rs`): đặc tả
  đòi 400 / 404 của bản mới mang lỗi JSON-RPC — client hai thế hệ đọc nó để biết máy chủ nói bản mới. (Trước đây lớp
  Problem thay thân đó, nên client hai thế hệ lùi về `initialize`.)
- Client cho test `autoxide_testkit::mcp::McpClient` (gửi thẳng vào router): mặc định nói bản mới; `McpClient::legacy`
  bắt tay bản cũ. Tên hàm / trường như client `rmcp` (`call_tool`, `structured_content`…), và tự kiểm hình dạng kết quả
  theo thế hệ — mọi test gọi MCP cũng là test tương thích giao thức.

Lý do: `rmcp` 3.11 (bản mới nhất) phụ thuộc `chrono` bắt buộc (`Annotations.last_modified`), chặn việc gỡ thư viện bị bỏ
hoang (D-20261121-time-library); chủ dự án yêu cầu "không có thư viện tốt thì tự viết". Phần kernel dùng chỉ là server
không giữ phiên, trả JSON — vài trăm dòng theo đặc tả, có test cho từng luật trên.

Đã cân nhắc: chờ `rmcp` bỏ `chrono` (không có mốc); fork `rmcp` (`deny.toml` chỉ nhận crates.io, phải tự theo dõi bản vá);
SDK MCP Rust khác (không có SDK nào khác đủ chính thức, và cũng phải kiểm phụ thuộc); trả SSE (không tool nào gửi tin
nhắn giữa chừng; `subscriptions/listen` chưa cần vì danh sách tool không đổi khi máy chủ chạy).

Thay: phần "`rmcp` ở kernel" của D-068
