## D-20261123-english-rustdoc · Doc comment của phần công khai viết tiếng Anh, gate `doc-language` — 2026-11-23 — chủ dự án quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: doc comment (`///`, `//!`) của phần code là hợp đồng với người viết module ở repo riêng
(D-20261121-external-modules) viết bằng **tiếng Anh**: `crates/kernel/src`, `crates/testkit/src`,
`crates/app/src/lib.rs`, và `api` của mỗi module (`modules/<tên>/src/api.rs`, `modules/<tên>/src/api/…`). Comment `//`
thường — giải thích VÌ SAO cho chủ dự án, ghi chú cài đặt — vẫn viết tiếng Việt được. Phần còn lại của
`modules/*/src` (không phải hợp đồng) và tài liệu markdown trong `docs/` giữ luật cũ. Luật `doc-language` của
`cargo xtask check` chặn dòng `///` / `//!` có chữ có dấu tiếng Việt ở các đường dẫn trên, không có danh sách miễn
trừ; chữ trong `code span` và chuỗi `"…"` trong khối code của doc được bỏ qua (dữ liệu ví dụ như từ khoá tìm kiếm
`áo thun`, không phải lời văn). Toàn bộ doc tiếng Việt đã có (khoảng 8.900 dòng) được dịch sang tiếng Anh cùng lúc;
đoạn nào thực chất là ghi chú cài đặt cho người bảo trì thì chuyển thành `//` tiếng Việt thay vì dịch.

Lý do: core làm cho cộng đồng; người lạ đọc docs.rs hay `cargo doc` để viết module cho Autoxide phải hiểu được hợp
đồng mà không biết tiếng Việt. OpenAPI và README đã là tiếng Anh; phần rustdoc công khai là chỗ còn thiếu (mục ghi
nợ ở P9 của bước 70). Gate giữ cho phần viết mới không trôi lại tiếng Việt.

Đã cân nhắc: dịch cả comment `//` — bỏ, vì comment giải thích cho chủ dự án (mới học Rust) cần tiếng Việt và không
hiện trên docs.rs; áp cho mọi file của module — bỏ, phần ngoài `api` không phải hợp đồng; gate theo danh sách miễn
trừ cho file cũ rồi dịch dần — bỏ, dịch một lần đủ nhanh và danh sách miễn trừ dễ thành vĩnh viễn; kiểm cả chữ trong
code span — bỏ, chặn nhầm dữ liệu ví dụ tiếng Việt có chủ đích.

Thay: phần "comment … viết tiếng Việt" của D-013 (riêng doc comment của phần công khai)
