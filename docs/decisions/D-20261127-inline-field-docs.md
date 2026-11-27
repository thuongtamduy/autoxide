## D-20261127-inline-field-docs · Trường `#[schema(inline)]` không mang `///`; bộ nối thoại tắt thì xoá bản ghi báo lỗi, không xoá dấu vết — 2026-11-27 — agent quyết

Chủ đề: HTTP, REST, OpenAPI

Quyết định:

1. Trường có `#[schema(inline)]` (thường vì tool MCP không đọc được `$ref`) **không** viết doc `///` hay `default` ở trường:
   utoipa gộp schema được inline với phần mô tả của trường thành `allOf` mà phần thứ hai có `type: object`. Với kiểu chuỗi
   (`Realm`) điều đó thành "chuỗi giao object" — không giá trị nào hợp lệ theo JSON Schema, SDK TypeScript sinh
   `("staff" | "customer") & Record<string, unknown>` không gán được; với kiểu object (`Money`) thì nới kiểu thành nhận
   khoá lạ. Mô tả của trường chuyển lên doc của kiểu chứa nó (như `RoleGrant::scope` đã làm). Sửa hai chỗ còn trên `main`:
   `ProviderRequest.realm` (auth, `POST /api/v1/auth/oidc-providers`, tool `auth.create_oidc_provider`) và
   `CatalogPriceInput.amount` (catalog). api-diff báo `realm` "object | string → string" là phá vỡ: cho phép trong
   `xtask/api-breaking-allow.json` vì server chưa bao giờ nhận object (`Realm` chỉ đọc chuỗi) — chỉ tài liệu sai.
2. `conversations` đang bật mà bộ nối của nhà cung cấp thoại giữ bản ghi không bật: `erase_recording` (xoá theo yêu cầu,
   dọn theo hạn lưu, xoá tài khoản) báo `503 conversations.voice_provider_unavailable` **trước** khi đụng tới nơi lưu hay
   database, giữ nguyên `provider_recording_id`; job dọn theo hạn bỏ qua dòng lỗi, dọn các dòng khác, rồi báo lỗi đầu tiên
   để được thử lại. Giống đường "nghỉ" (`RecordingEraser::Dormant`, D-20261125-dormant-modules): không báo xong khi bản
   ghi còn nằm ở nhà cung cấp mà không còn dấu vết để xoá sau.
3. `./gate.sh` (cả gate nhẹ `GATE_LOCAL=off`) chạy `cargo xtask sdk --extract-examples` + `tsc --noEmit` như job `sdk`
   của CI khi có Node, và ở gate nhanh chỉ khi SDK, `docs/openapi.json`, `docs/guides`, `xtask/src/sdk.rs` hay `gate.sh`
   đổi; bản TypeScript thấp nhất cam kết (5.0.4) chỉ kiểm trên CI vì cần tải gói.

Lý do: OpenAPI là hợp đồng cho máy; một trường không có giá trị hợp lệ làm hỏng client sinh tự động và agent kiểm đầu vào
bằng schema. Xoá dấu vết bản ghi khi chưa xoá được ở nhà cung cấp làm dữ liệu cá nhân nằm lại không ai biết. Kiểu SDK hỏng
lọt tới CI (#517) tốn một vòng CI mỗi lần.

Đã cân nhắc: bỏ `inline` để dùng `$ref` (tool MCP không đọc được `$ref`, `autoxide openapi` từ chối); sửa ở bộ sinh SDK để
lờ phần `allOf` thừa (OpenAPI vẫn sai cho client khác); để `erase_recording` bỏ qua nhà cung cấp và ghi cảnh báo (bản ghi
nằm lại không dấu vết — đúng lỗi cần sửa).
