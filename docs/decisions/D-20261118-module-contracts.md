## D-20261118-module-contracts · OpenAPI và MCP theo từng module, trỏ từ `/llms.txt` — 2026-11-18 — agent quyết

Chủ đề: MCP, AI, agent dùng API

Quyết định:

1. **`GET /api/v1/openapi/{file}`** với `file` = `<module>.json` (công khai, `Cache-Control: public, max-age=300`): chỉ các
   đường dẫn dưới `/api/v1/<module>` và các schema chúng dùng (đi hết `$ref` lồng nhau), cùng `info` (quy ước chung, thêm câu
   nói đây là một phần), `servers`, `securitySchemes` và tag của module. Tách một lần lúc dựng router từ tài liệu chung đã
   hoàn tất, nên từng chữ khớp bản chung (test e2e so từng đường dẫn và schema, và mọi `$ref` có đích). Module không có
   đường dẫn nào, hay tên không đuôi `.json`: `404 openapi_module_not_found`. Đường dẫn dạng `{file}` vì router của axum
   (matchit 0.8) không nhận tham số có đuôi cố định như `{module}.json`.
2. **`/mcp/<module>`**: endpoint MCP riêng của mỗi module có tool, resource hay prompt — cùng lớp xác thực, cùng luật lọc
   theo permission và gói của khách như `/mcp`, nhưng chỉ có phần của module đó (`tools/call` tool module khác: "Unknown
   tool", như tool không có). Chuẩn MCP không có tham số lọc cho `tools/list`; endpoint riêng thì client MCP nào cũng dùng
   được, không cần mở rộng giao thức. Hướng dẫn lúc bắt tay (`instructions`) nhắc tới nó.
3. **`/llms.txt`**: dưới mỗi module một dòng con: link `[OpenAPI](/api/v1/openapi/<module>.json)`, `[MCP](/mcp/<module>)` và
   tên các tool; mục API nêu dạng đường dẫn chung. Test giữ mọi link không 404.
4. Không sinh sẵn `docs/openapi/<module>.json` trong repo: sẽ chép lại phần lớn `docs/openapi.json` (hơn 130 nghìn dòng) thành
   hàng chục file phải giữ khớp; endpoint trên máy chủ đã phục vụ đúng bản của phiên bản đang chạy.

Lý do: `docs/openapi.json` hơn 130 nghìn dòng và `tools/list` liệt kê 182 tool một lượt — agent chỉ cần làm việc với một
module phải nạp cả hai, tốn ngữ cảnh và dễ chọn nhầm tool. Tách theo module là ranh giới đã có (đường dẫn `/api/v1/<module>`,
tên tool `<module>.<tên>`), không cần khai gì thêm ở module.

Đã cân nhắc: tham số riêng trong `_meta` của `tools/list` (client MCP thông thường không gửi được); resource MCP liệt kê tool
theo module (agent vẫn phải gọi `tools/list` đầy đủ mới gọi được tool); lọc tài liệu theo tag (module được tự đặt tag,
đường dẫn thì không đổi được).
