## D-20261128-codeql-rust-placeholder · PR không chạm Rust tải lên kết quả CodeQL Rust rỗng thay vì bỏ cấu hình — 2026-11-28 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: matrix của job `analyze` trong `.github/workflows/codeql.yml` luôn có đủ `rust`, `javascript-typescript`,
`actions`. Khi job `plan` in đúng chuỗi `false` (PR không chạm Rust, theo `scripts/rust-changed.sh`), phần `rust` không
chạy `init`/`analyze` mà ghi một file SARIF có một run công cụ `CodeQL` với `results: []` rồi tải lên bằng
`github/codeql-action/upload-sarif` (cùng phiên bản ghim) với `category: /language:rust`. Mọi trường hợp khác (`plan` đỏ,
bị bỏ qua, output khác `false`, push `main`, lịch tuần, chạy tay) quét Rust thật như D-20261128-codeql-rust-on-change.

Lý do: Code scanning so các cấu hình phân tích (workflow + job + giá trị matrix + category) của PR với nhánh gốc; khi PR
thiếu cấu hình `analyze (rust)` mà `main` có, check "Code scanning results / CodeQL" hiện đỏ "1 configuration not found"
— dấu đỏ giả trên mọi PR chỉ sửa tài liệu hay SDK sau #610, che dấu đỏ thật. `upload-sarif` lấy matrix của chính job
làm khoá cấu hình (input `matrix` mặc định `toJson(matrix)`), nên tải lên từ đúng phần `rust` của job `analyze` khớp cấu
hình của `main`; action chấp nhận run có mảng `results` rỗng (`countResultsInSarif` chỉ đòi có mảng). PR không đổi file
`.rs` nên kết quả Rust thật giống nhánh gốc, đã quét khi vào `main`. Chi phí: một runner khoảng nửa phút thay cho 21–27
phút quét. Hệ quả chấp nhận: trên PR đó, cảnh báo Rust đang mở ở `main` (nếu có) hiện là "đã sửa trong PR" — chỉ trong
giao diện PR, không đóng cảnh báo trên `main`, lần push `main` kế tiếp quét thật.

Đã cân nhắc:
- Để nguyên, coi là ghi chú: check đỏ trên PR làm người và agent quen bỏ qua check bảo mật.
- Tải lên từ một job riêng: khoá cấu hình gồm tên job và matrix, job khác không khớp cấu hình của `main`.
- Quét Rust với bộ lọc `paths` rỗng hay trỏ tới thư mục không có: CodeQL báo lỗi "no source code seen" và job đỏ.
- Quét Rust thật trên mọi PR (bỏ D-20261128-codeql-rust-on-change): trái quyết định của chủ dự án, tốn 20+ phút mỗi PR.
- Xoá cấu hình Rust trên `main`: mất quét Rust trên `main`.

Thay: phần "Bỏ quét ở mức step … Ngôn ngữ bị bỏ không sinh check" của D-20261128-codeql-rust-on-change — điều kiện bỏ quét
giữ nguyên (chỉ khi `plan` in đúng `false`), chỉ đổi cách phần `rust` kết thúc.
