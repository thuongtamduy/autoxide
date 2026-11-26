## D-20261126-contract-versioning · Đánh phiên bản event giữa module, webhook gửi đi và phần mở rộng `x-*` của OpenAPI — 2026-11-26 — agent quyết

Chủ đề: HTTP, REST, OpenAPI

Quyết định: Cùng quy tắc với REST (D-059) cho ba hợp đồng: thêm trường tuỳ chọn / event / phần mở rộng mới là không phá vỡ;
bỏ, đổi kiểu, đổi nghĩa, thêm trường bắt buộc là phá vỡ → bản mới chạy song song, bản cũ khai sắp bỏ kèm sunset và cái thay
thế. (1) Event: `EventInfo::payload::<T: Schema>()` khai schema payload có kiểu; `Outbox::record` nhận payload có kiểu;
bản phá vỡ là tên mới hậu tố `.v2`, module phát cả hai tới sunset (`EventInfo::deprecated`); catalog sinh
`docs/events.json`. (2) Webhook: thêm header `webhook-version: 1` (bản của khung thân Standard Webhooks) và `data.schema`
(tên event kèm bản); đăng ký chọn tên event có bản; bản cũ tới sunset thì báo trước 30 ngày rồi tắt; payload khai ở mục
`webhooks` của OpenAPI. (3) Phần mở rộng `x-*`: danh mục ở kernel (tên, JSON Schema giá trị, hạng), `#[endpoint]` chỉ sinh
phần mở rộng có trong danh mục, xuất `docs/openapi-extensions.json`; tên mới dạng `x-autoxide-<tên>`, 7 tên ngắn cũ giữ
nguyên. `xtask contract-diff` (mở rộng `api-diff`) so `openapi.json`, `events.json`, danh mục phần mở rộng với bản phát hành
gần nhất; phần hạng thử nghiệm chỉ báo. Tên và schema tool MCP do phiên AI agent làm theo cùng quy tắc.

Lý do: 71-G13 (D-20261126-release-1-0-readiness) đòi mọi hợp đồng có phiên bản. Payload event hiện là `Value` không schema,
webhook không có số bản, 9 phần mở rộng không có danh mục: đổi một trường lặng lẽ làm vỡ module khác, người nhận webhook và
client sinh giao diện từ `x-permissions` mà không gate nào thấy. Tên `.v2` (thay vì số bản trong payload) để bên nhận chọn
bản lúc đăng ký và không bao giờ nhận hình dạng lạ.

Đã cân nhắc: số bản trong payload, một tên event (bên nhận phải xử lý mọi bản cùng lúc); đổi tên 7 phần mở rộng cũ sang
`x-autoxide-` (phá client hiện có, không được gì). Thiết kế: §13 của `docs/research/kernel-contract-1-0.md`.
