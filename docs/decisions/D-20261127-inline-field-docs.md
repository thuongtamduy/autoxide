## D-20261127-inline-field-docs · Trường `#[schema(inline)]` có kiểu là thẳng một struct / enum không mang `///` — 2026-11-27 — agent quyết

Chủ đề: HTTP, REST, OpenAPI

Quyết định:

1. Trường có `#[schema(inline)]` (thường vì tool MCP không đọc được `$ref`) mà **kiểu của trường là thẳng một struct
   hay enum** (`realm: Realm`, `amount: Money`) **không** viết doc `///` hay `default` ở trường: utoipa gộp schema được
   inline với phần mô tả của trường thành `allOf` mà phần thứ hai chỉ là `{description, type: object}`. Với kiểu chuỗi
   (`Realm`) điều đó thành "chuỗi giao object" — không giá trị nào hợp lệ theo JSON Schema, SDK TypeScript sinh
   `("staff" | "customer") & Record<string, unknown>` không gán được; với kiểu object (`Money`) thì nới kiểu thành nhận
   khoá lạ. Mô tả của trường chuyển lên doc của kiểu chứa nó (như `RoleGrant::scope` đã làm), ở trường viết `//`.
   Trường `Option<T>` hay `Vec<T>` có `inline` **vẫn viết `///` được**: utoipa đặt mô tả cạnh `oneOf` / `items`, không
   sinh `allOf` (đã đối chiếu `docs/openapi.json`: `CatalogPriceInput.compare_at: Option<Money>` giữ mô tả, không có
   `allOf`).
2. Sửa hai chỗ còn trên `main`: `ProviderRequest.realm` (auth, `POST /api/v1/auth/oidc-providers`, tool
   `auth.create_oidc_provider`) và `CatalogPriceInput.amount` (catalog). api-diff báo `realm` "object | string → string"
   là phá vỡ: cho phép trong `xtask/api-breaking-allow.json` vì server chưa bao giờ nhận object (`Realm` chỉ đọc chuỗi)
   — chỉ tài liệu sai.
3. Luật `api-inline-field-doc` của `cargo xtask check` (`xtask/src/api_shape.rs`) chặn tái phạm: trên
   `docs/openapi.json` và `docs/mcp-tools.json`, không `allOf` nào có phần chỉ gồm `{description?, type: "object"}`.
   Hình của `#[serde(flatten)]` một map (phần object rỗng đứng đầu, mọi phần sau là thân struct có `properties`, như
   `PersonalDataResponse`) được nhận ra theo cấu trúc, không bằng danh sách cho phép; luật không có dòng cho phép.

Lý do: OpenAPI là hợp đồng cho máy; một trường không có giá trị hợp lệ làm hỏng client sinh tự động và agent kiểm đầu vào
bằng schema. Kiểm bằng luật thay vì nhớ: lỗi chỉ lộ ra ở SDK hay ở client của người khác.

Đã cân nhắc: bỏ `inline` để dùng `$ref` (tool MCP không đọc được `$ref`, `autoxide openapi` từ chối); sửa ở bộ sinh SDK để
lờ phần `allOf` thừa (OpenAPI vẫn sai cho client khác); cấm `///` trên mọi trường `inline` (thừa: `Option<T>` / `Vec<T>`
không bị); miễn `PersonalDataResponse` bằng danh sách cho phép (dễ thành chỗ lọt cho lỗi mới).
