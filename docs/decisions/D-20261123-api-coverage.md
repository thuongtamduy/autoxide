## D-20261123-api-coverage · Đo độ phủ endpoint và tool MCP bằng test, mục tiêu 100% trước 1.0 — 2026-11-23 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: testkit ghi lại endpoint và tool MCP mà test gọi thành công khi biến môi trường `TEST_API_COVERAGE` trỏ tới
một thư mục: `TestRequest::send` ghi `http <METHOD> <path template>` khi trả lời dưới 400 (template tra trong hợp đồng
OpenAPI mà testkit đã dùng để kiểm response), `McpClient` ghi `tool <tên>` khi `tools/call` không trả `isError`. Mỗi
process một file `<pid>.txt`, mỗi dòng ghi một lần. `cargo xtask coverage-api --run` chạy `cargo xtask test-all` với
biến đó (thư mục `target/api-coverage`) rồi so với `docs/openapi.json` và `docs/mcp-tools.json`, in theo module những
endpoint và tool chưa phủ; không `--run` thì chỉ đọc lần đo trước. Đo trên SQLite là đủ vì mọi test chạy trên cả hai
database ở gate đầy đủ. Lệnh **chỉ báo, chưa chặn**; mục tiêu là phủ 100% trước 1.0 (P11 của bước 70: "dùng được mọi
thứ đã dựng").

Đề xuất cho sau khi đạt 100%: thêm `--deny` (đỏ khi còn endpoint hay tool chưa phủ) và chạy nó trong gate đầy đủ trên
CI — endpoint mới không có test gọi thành công thì không merge được. Chưa bật vì lúc này còn nhiều chỗ chưa phủ, bật
sớm thì phải có danh sách miễn trừ, mà danh sách miễn trừ dễ thành vĩnh viễn.

Lý do: "ít nhất phải dùng được tất cả những gì đã build" cần một con số kiểm được, không phải cảm giác. Ghi ở testkit
thì test của module (`modules/*/tests`) và e2e của binary cùng được tính, không phải viết lại test có sẵn. Khi biến
không đặt, chi phí là một lần đọc `OnceLock` mỗi request — gate không chậm đi.

Đã cân nhắc: đọc `MatchedPath` của axum qua một layer — bỏ, layer ngoài cùng chưa có `MatchedPath` khi router lồng
nhau, còn template từ hợp đồng OpenAPI là thứ cần so; ghi cả request lỗi — bỏ, gọi được mà chỉ nhận 4xx không chứng
minh endpoint dùng được; đo bằng công cụ phủ dòng code (llvm-cov) — bỏ, đắt, và phủ dòng không trả lời câu "endpoint
này có ai gọi thành công chưa".
