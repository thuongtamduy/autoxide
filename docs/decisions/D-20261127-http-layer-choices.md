## D-20261127-http-layer-choices · Lớp HTTP của kernel: bốn chỗ làm khác bản thiết kế — 2026-11-27 — agent quyết

Chủ đề: HTTP, REST, OpenAPI

Quyết định: PR 4 của hợp đồng kernel (D-20261126-kernel-web-layer, thiết kế `docs/research/kernel-contract-1-0.md` §4) làm
khác bản thiết kế ở bốn chỗ:

1. **Một trait kiểm đầu vào, không hai extractor.** `autoxide_kernel::http::Validate` có impl chung cho mọi kiểu cài
   `validator::Validate`. `#[derive(Validate)]` của kernel sinh đúng mã của `validator_derive` 0.20 qua đường
   `__private`. `ValidJson` và `Tool::new` chỉ đòi trait của kernel. Module cũ không phải sửa, không cần extractor
   `Input<T>` thứ hai.
2. **`Module::endpoints() -> Routes`** (có cài mặc định, D-038) thay cho bí danh tạm `ModuleRouter` đánh `#[deprecated]`.
   Kernel ghép kết quả với `Module::routes`; manifest (PR 5) thay cả hai.
3. **Luật `wrapped-libraries` tách hai:** `[dependencies]` và `[dev-dependencies]` (`wrapped-libraries-dev`), mỗi luật một
   baseline chỉ giảm. Module "đã chuyển" (luật `wrapped-paths` áp dụng) xét theo phụ thuộc thường. `tower` ở
   `[dev-dependencies]` được phép (thiết kế §1.1).
4. **`Path`, `Query`, `State` của kernel bọc extractor cùng tên của axum và trả nguyên rejection.** Lớp `problem` của
   kernel đổi rejection thành `Problem` như trước, nên hành vi không đổi. utoipa nhận extractor theo tên kiểu, nên
   OpenAPI cũng không đổi.
5. **Kiểm route xung đột bằng chính `matchit`** (bộ khớp route của axum, khai thẳng và ghim đúng bản axum ghim) thay vì
   tự đoán luật khớp: `/{id}` với `/{key}`, hay `/{id}` với `/{*rest}`, xung đột với axum dù khác method. Lúc khởi động
   kernel báo lỗi rõ thay vì để axum dừng tiến trình lúc ghép router.

Lý do:
1. Hai extractor buộc mỗi module chọn một trong lúc chuyển, và kernel phải giữ cả hai tới 1.0. Impl chung cho cùng kết
   quả với một đường. Test so lỗi của hai derive trên cùng đầu vào (`http::input::tests`).
2. Bí danh `#[deprecated]` buộc module dùng nó phải tắt cảnh báo `deprecated`, mà luật cấm tắt lint để qua gate.
3. Testkit còn lộ kiểu `http` (`StatusCode`, `Method`) trong API công khai. Nếu gộp chung một luật, module đã chuyển trông
   như chưa chuyển chỉ vì test của nó.
4. Đổi mã lỗi của lỗi đọc đường dẫn hay query là thay đổi client thấy; để PR riêng nếu cần.

Đã cân nhắc:
- Hai extractor `ValidJson` (cũ) và `Input<T>` (mới), như §4.3 của thiết kế.
- Bí danh `ModuleRouter` `#[deprecated]`, như phiên điều phối chốt trước khi thấy vướng lint.
- Một luật chung, bắt testkit bọc `http` ngay trong PR 4: PR to gấp đôi, đụng test của mọi module.
- `Path`/`Query` của kernel tự dựng `Problem` có mã riêng.
