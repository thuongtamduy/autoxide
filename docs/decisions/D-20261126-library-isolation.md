## D-20261126-library-isolation · Core không để thư viện bên thứ ba quyết định nhịp nâng cấp: module chỉ dùng thư viện qua kernel — 2026-11-26 — chủ dự án quyết

Chủ đề: Kiến trúc module và kernel

Quyết định:

1. **Bỏ chính sách "thư viện đổi bản lớn thì Autoxide đổi bản lớn".** Bản lớn mới của một thư viện (SeaORM, axum,
   utoipa, validator, sqlx…) không được buộc Autoxide ra bản lớn và không được buộc module của người khác sửa code. Sửa
   `docs/COMPATIBILITY.md` và `docs/MODULE_GUIDE.md` (EN + VI) theo quyết định này.
2. **Kernel sở hữu bề mặt module dùng.** Module viết code qua các lớp của kernel (truy vấn và entity, route và schema
   API, kiểm đầu vào, migration, kiểu thời gian và tiền đã có sẵn) thay vì gọi thẳng thư viện. Kiểu của thư viện không xuất
   hiện trong chữ ký công khai hạng ổn định của kernel và testkit; chỗ chưa bọc được phải nằm trong hạng thử nghiệm và có
   lộ trình bọc.
3. **Gate giữ luật.** `cargo xtask check` (và `check-module` cho module ngoài repo) chặn module khai phụ thuộc trực tiếp vào
   thư viện đã có lớp của kernel.
4. **Chuyển dần, không đập đi.** Làm lớp của kernel trước, chuyển module theo cụm, giữ đường cũ có `#[deprecated]` trong
   thời gian chuyển; xong trước 1.0 (D-20261126-road-to-1-0).

Lý do: chủ dự án yêu cầu nâng cấp phải dễ như bấm một nút và không bị thư viện bên thứ ba níu chân. Rà soát 26/11/2026:
module gọi thẳng SeaORM (~1.970 chỗ), utoipa (~1.500), axum (~650); kernel lộ kiểu SeaORM, axum, utoipa ở hàng trăm chữ
ký công khai. Repo đã làm đúng cách này ba lần (thời gian bọc `jiff` sau kiểu của kernel, bộ migration riêng thay
`sea-orm-migration`, máy chủ MCP riêng thay `rmcp`) và các lần nâng thư viện đó không chạm module.

Đã cân nhắc: giữ chính sách cũ và ghim phiên bản thư viện lâu (chậm vá bảo mật, và người viết module bị khoá theo); bọc
toàn bộ ngay một lần (rủi ro lớn, PR khổng lồ); chỉ bọc kiểu ở chữ ký kernel mà vẫn cho module gọi thẳng thư viện (không
giải quyết được chuyện module phải sửa khi thư viện đổi).
