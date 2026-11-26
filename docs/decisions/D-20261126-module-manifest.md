## D-20261126-module-manifest · Trait `Module` bốn hàm: `manifest` khai mọi thứ tĩnh, `start` đăng ký mọi thứ cần service — 2026-11-26 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định: `Module` còn `manifest() -> Manifest`, `configure`, `start`, `dormant`. Thứ biết được khi chưa khởi động
(migration, permission, event, cài đặt, bản dịch, hạn mức, phụ thuộc, hạng…) khai bằng builder `Manifest`
(`#[non_exhaustive]`, mỗi khái niệm một hàm; khái niệm thử nghiệm sau `manifest.unstable()`). Thứ cần service đăng ký trong
`start`: `routes`, `tools`, `subscribe`, `provide`/`require`/`later`, `contribute::<dyn Trait>` (thay ~20 hàm đăng ký
riêng như `work_source`, `summary_metric`, `personal_data`), `kernel::<Handle>()` (thay ~25 getter như `outbox()`,
`sync_log()`). Chuyển: `manifest()` có cài đặt mặc định dựng từ 45 hàm cũ; hàm cũ `#[deprecated]`; `xtask check` đếm
module còn cài hàm cũ theo baseline chỉ giảm; baseline rỗng thì gỡ hàm cũ (trước 1.0).

Lý do: 49 hàm trait + ~50 hàm của `StartContext` cho nhiều khái niệm hai đường (khai ở trait hay đăng ký trong `start`), và
`routes()`/`tools()`/`subscriptions()` phải đọc `Option<service>` vì chạy sau `start`. Manifest là dữ liệu: in được thành
JSON cho manifest module của hệ sinh thái (71-G8), kiểm được, agent đọc một chỗ.

Đã cân nhắc: trait con theo nhóm (Rust không tự biết module cài trait con nào nếu không có specialization — phải thêm
`as_x()` cho mỗi nhóm); giữ nguyên và chỉ ghi tài liệu (không đạt "một cách làm"). Thiết kế đầy đủ, API mẫu và thứ tự PR: `docs/research/kernel-contract-1-0.md`.

Thay: phần "thêm một hàm vào `Module` kèm cài đặt mặc định" của D-038 (từ nay thêm một hàm vào `Manifest`)
