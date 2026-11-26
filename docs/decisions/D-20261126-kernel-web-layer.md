## D-20261126-kernel-web-layer · Lớp HTTP của kernel: `#[endpoint]`, `routes!`, `#[derive(Schema)]`, `#[derive(Validate)]`, extractor và kiểu thời gian riêng — 2026-11-26 — agent quyết

Chủ đề: HTTP, REST, OpenAPI

Quyết định: Module không khai `axum`, `utoipa`, `utoipa-axum`, `validator`, `jiff`, `async-trait`. Crate proc-macro
mới `autoxide-macros` (`syn` 2, `quote`, `proc-macro2`; kernel xuất lại) giữ ngữ pháp module viết; mã sinh ra gọi thư
viện qua `::autoxide_kernel::__private` (`#[doc(hidden)]`, gate cấm viết tay). (1) `#[endpoint(…)]` thay
`#[utoipa::path]`: gọi lại macro của utoipa bên trong một hàm ẩn có `use` cục bộ, ngữ pháp là tập con đang dùng cộng
`permission = "…"` (sinh `x-permissions`, `security`, 401/403 chuẩn), `operation_id` mặc định là tên hàm, `etag`.
(2) `routes![…]` trả `Routes<S>` của kernel, thay `OpenApiRouter` trong `Module`. (3) `#[derive(Schema)]` / `Params`:
bản sao "bóng" của struct (chép doc, `serde`, `schema`, `deprecated`, `cfg`; gắn `as` = tên thật) derive `utoipa::ToSchema`
trong khối `const _`, kiểu thật chuyển tiếp `schema()`, `name()`, `schemas()`; generic giữ tham số. Nâng utoipa: `openapi.json`
không đổi từng byte, đổi thì là thay đổi hợp đồng. Impl trait thư viện trên kiểu ổn định khai là ngoài hợp đồng. `forms` dùng
schema động kiểu `JsonSchema` của kernel; `auth` dùng `context.response_hook` thay `map_response`. (4)
`#[derive(Validate)]` của kernel với năm luật đang dùng (`length`, `range`, `email`, `url`, `nested`), cùng
`FieldError` và mã i18n; extractor `Input<T>`, `ValidJson` cũ `#[deprecated]`. (5) Extractor `Path`, `Query`, `State`,
`Headers`, `Body` (lỗi đọc đầu vào thành `Problem` có `code`); response `NoContent`, `Download`, `Redirect`; `Response`
thô chỉ ở `unstable`. (6) `time::{Duration, TimeZone, TimeOfDay, Weekday, DateTime}` thay `jiff` trong module. (7) Kernel
xuất `autoxide_kernel::async_trait`. Thư viện từ vựng được dùng thẳng: `serde`, `serde_json`, `uuid`, `tokio` (không có
trong chữ ký ổn định; việc nền qua `task::spawn`), `tracing`, `futures-core`, `bytes`; `futures-util` là thư viện riêng của
module, `tower` chỉ ở `[dev-dependencies]`. Thời gian `cargo check` của workspace không tăng quá 5% vì bản sao bóng; vượt thì sinh `ToSchema` thẳng
bằng builder.

Lý do: macro của utoipa sinh mã gọi `utoipa::…` theo tên nên chỉ xuất lại macro là không đủ; giữ ngữ pháp ở macro của mình
thì bản lớn mới của thư viện chỉ sửa một chỗ. Đã thử trên crate nháp với đúng `utoipa` 6.0.0, `utoipa-gen` 6.0.1,
`utoipa-axum` 0.3.0: crate dùng chỉ khai kernel + `serde` vẫn ra đủ path và schema. Các đường tắt (`permission`, `etag`)
xoá hàng trăm dòng lặp và làm OpenAPI đồng đều hơn cho agent.

Đã cân nhắc: tự viết bộ sinh OpenAPI (lớn; để dành, đổi được về sau mà module không biết); xuất lại macro thư viện (module vẫn
phải khai thư viện và theo ngữ pháp của nó); ngữ pháp mới hoàn toàn (chuyển 1.200 endpoint đắt mà không lợi gì). Thiết kế đầy đủ, API mẫu và thứ tự PR: `docs/research/kernel-contract-1-0.md`.
