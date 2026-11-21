## D-20261121-stability-marks · Hạng module trong hợp đồng: `x-stability` chỉ gắn cho thử nghiệm — 2026-11-21 — agent quyết

Chủ đề: HTTP, REST, OpenAPI

Quyết định: Mọi operation OpenAPI của module thử nghiệm mang extension `x-stability: experimental`
và một câu `**Experimental**…` ở cuối mô tả, do lớp HTTP của kernel gắn lúc dựng tài liệu (module
không tự viết). Operation KHÔNG có `x-stability` thuộc module chính thức. Tool MCP của module thử
nghiệm có cùng câu đó trong mô tả; `docs/mcp-tools.json` có trường `stability` (`official` /
`experimental`) cho mọi tool. Dời, tách, gộp khách bị từ chối (`tenant_move_modules_off`) khi khách
còn dữ liệu của module đang tắt; task khởi động của module đang tắt được bỏ qua kèm cảnh báo.

Lý do: Agent và người viết client đọc hợp đồng đầy đủ trong repo (mọi module) cần biết endpoint nào
có thể đổi mà không phải tra bảng ở `docs/MODULES.md`. Chỉ gắn cho thử nghiệm để tài liệu của hơn
hai mươi module chính thức không đổi, và để module lên chính thức chỉ là gỡ dấu. Gắn ở kernel theo
hạng module khai (`Module::stability`) nên không thể lệch với hạng thật.

Đã cân nhắc: gắn `x-stability: official` cho mọi operation còn lại — thêm hàng nghìn dòng vào
`openapi.json` mà không thêm thông tin; tag OpenAPI riêng cho module thử nghiệm — client sinh mã
theo tag sẽ đổi tên lớp khi module lên chính thức.
