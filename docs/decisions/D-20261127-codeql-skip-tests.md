## D-20261127-codeql-skip-tests · CodeQL bỏ qua code chỉ dùng cho test — 2026-11-27 — chủ dự án quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: workflow CodeQL (`.github/workflows/codeql.yml`) dùng file cấu hình `.github/codeql/codeql-config.yml`, trong
đó `paths-ignore` bỏ qua code chỉ biên dịch khi test:

- `crates/app/src/e2e` (`#[cfg(test)] mod e2e`);
- `crates/testkit` (chỉ được khai ở `[dev-dependencies]`);
- thư mục `tests/` và file `tests.rs`, `*_tests.rs` — đúng tập mà `is_test_file` của `cargo xtask check` coi là test.

Code chạy thật (mọi thứ còn lại trong `crates/`, `modules/`, `sdk/`, `examples/`, workflow) vẫn được quét đủ bộ truy vấn,
không tắt truy vấn nào. Thêm đường dẫn vào `paths-ignore` chỉ khi đó là code chỉ dùng cho test, ghi lý do vào file cấu hình.

Lý do: trong một ngày, các PR liên tục bị CodeQL chặn vì báo nhầm trong code test — mật khẩu mẫu của test
("Hard-coded cryptographic value"), thông báo `assert!` in đường dẫn có UUID ("Cleartext logging of sensitive
information"). Mỗi lần chủ dự án phải vào tab Security dismiss tay, PR đứng chờ. Code test không chạy ở máy chủ thật nên
cảnh báo ở đó không phải lỗ hổng của sản phẩm.

Còn lại: module test viết ngay trong file code chạy thật (`#[cfg(test)] mod … { … }` giữa file, vd `test_support` của
`modules/auth/src/http/mod.rs`) không bỏ qua được theo đường dẫn. Cảnh báo ở đó dismiss tay với lý do "Used in tests"; code
test mới nên nằm trong file test riêng (`*_tests.rs`, `tests/`) như quy ước sẵn có.

Đã cân nhắc:
- Dismiss tay từng cảnh báo: mỗi lần sửa file lại có thể nổi cảnh báo "mới" do dòng xê dịch; tốn thời gian chủ dự án.
- Tắt hẳn các truy vấn hay báo nhầm (`query-filters`): mất luôn phần quét code chạy thật — không chọn.
- Viết lại test để né truy vấn (sinh mật khẩu ngẫu nhiên, bỏ thân response khỏi thông báo assert): làm test khó đọc, khó
  chẩn đoán khi hỏng, chỉ để chiều công cụ.
