## D-20261126-demo-and-native-binaries · `autoxide demo`, binary macOS/Windows và `cargo xtask doctor` — 2026-11-26 — agent quyết

Chủ đề: Phạm vi và mục tiêu

Quyết định (bước 71, G6, cụm không phụ thuộc cách viết module):

1. **`autoxide demo` là `serve` với cấu hình điền sẵn**, không phải đường khởi động thứ hai. Nó đọc `.env` và biến
   môi trường như mọi lệnh, đặt giá trị mặc định của demo bằng `ConfigSource::with_default` (giá trị người dùng đặt
   vẫn thắng), đổi thư mục làm việc sang thư mục demo (mặc định `autoxide-demo`, `--dir`) rồi gọi `serve`. Bí mật
   (khoá ký, khoá gốc, mật khẩu quản trị) sinh một lần, ghi vào `demo-state` (quyền `0600`) để lần chạy sau dùng lại
   cùng dữ liệu. Mật khẩu quản trị được **in ra màn hình** — ngoại lệ có chủ đích cho quy tắc "không ghi mật khẩu
   vào log" của `serve`, vì demo chạy cục bộ và dùng xong bỏ; production vẫn theo D-20261121-admin-password-file.
2. **"Dữ liệu mẫu" của demo là dữ liệu tham chiếu đi kèm** (đơn vị hành chính, thuế, giới hạn khuyến mãi), nạp bằng
   `AUTOXIDE_STARTUP_TASKS`. Không có bản ghi nghiệp vụ giả: module `notes` không nằm trong binary chính thức, và
   core trung lập với nghiệp vụ.
3. **Binary macOS (arm64, x86_64) và Windows (x86_64) dựng trên máy runner của từng hệ điều hành** trong
   `release.yml`, job `native`, đính kèm bản phát hành (`.tar.gz`, `.zip` cho Windows) cùng `SHA256SUMS` và chứng thực
   nguồn gốc build như binary Linux. Job `native` **không chặn phát hành**: nền tảng nào hỏng thì bản phát hành vẫn
   ra với các file còn lại. Chưa ký bằng chứng chỉ của Apple hay Microsoft (cần chứng chỉ trả phí của chủ dự án);
   `docs/GETTING-STARTED.md` nói cách mở file chưa ký. Windows và macOS chưa từng được build trong CI của repo —
   lần chạy đầu của job là lần kiểm đầu tiên; bản `-rc` đầu tiên sau thay đổi này cho biết.
4. **`cargo xtask doctor`** (chỉ đọc) kiểm máy người phát triển: công cụ bắt buộc, nên có, tuỳ chọn, mỗi thứ thiếu
   kèm một lệnh cài; khác `autoxide doctor` (kiểm một bản cài đang chạy).
5. **Mỗi luật của `xtask check` / `check-module` có một dòng "cách sửa"** (`xtask/src/hints.rs`), in dưới vi phạm; test
   chặn thêm luật mà quên dòng đó.
6. `.devcontainer/devcontainer.json` cho Codespaces và VS Code dev container.

Lý do: mục tiêu của chủ dự án ở 71-G9 — tải về tới request đầu tiên dưới 5 phút. Cách ngắn nhất là một lệnh không cần
cấu hình; người không có Docker hay Rust cần binary dựng sẵn.

Đã cân nhắc: demo bằng `docker compose` có sẵn (cần tạo `.env`, vẫn là nhiều bước); nạp bản ghi nghiệp vụ mẫu qua
module `notes` (không nằm trong binary chính thức); ký binary macOS/Windows ngay (cần secret của chủ dự án — báo tên
secret cần có khi họ muốn); cho job `native` chặn phát hành (một runner hỏng sẽ chặn cả bản Linux và image).
