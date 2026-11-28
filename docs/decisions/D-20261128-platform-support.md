## D-20261128-platform-support · Cam kết nền tảng: PostgreSQL 16–18 theo vòng đời của hãng, SQLite ≥ 3.45, danh sách hệ điều hành / CPU, `doctor` cảnh báo — 2026-11-28 — agent quyết

Chủ đề: Vận hành và hiệu năng

Quyết định: mục "Nền tảng được hỗ trợ" của `docs/COMPATIBILITY.md` (EN + VI) là danh sách cam kết: PostgreSQL các bản
major hãng còn bảo trì từ 16 trở lên (16, 17, 18; thêm bản mới khi gate đầy đủ xanh trên nó, bỏ bản cũ chỉ sau ngày EOL
của hãng, báo trước một bản minor), SQLite 3.45 trở lên (binary chính thức nhúng 3.54), image `linux/amd64` + `linux/arm64`,
binary Linux glibc 2.41+ (build trong Debian 13), macOS 11+, Windows 10+. CI giữ ma trận hiện có: PostgreSQL 16 ở mọi PR,
16 và 18 (cũ nhất và mới nhất) trên `main` và trước phát hành — không thêm job. `autoxide doctor` có bước
`database_version`: hỏi bản database thật, ngoài danh sách hay đã quá ngày EOL thì **cảnh báo** (không làm hỏng), bảng
bản major + ngày EOL nằm ở `crates/app/src/commands/doctor/database_version.rs`, sửa cùng tài liệu và ma trận CI.

Lý do: G14 (D-20261126-operations-baseline) cần người vận hành biết chạy trên bản nào. 16 là bản thấp nhất CI đã test từ
trước (STACK.md); thêm 17 vào ma trận sẽ làm CI chậm hơn mà cặp cũ nhất / mới nhất đã bắt lỗi khác biệt giữa các bản. Cảnh
báo thay vì chặn: người dùng bản 19 mới ra hay bản 15 còn chạy được thì không nên bị khoá ngoài.

Đã cân nhắc: đọc bảng EOL từ mạng lúc chạy `doctor` (máy chủ không được tự gọi ra ngoài khi chẩn đoán, và ngày EOL của hãng
cố định từ lúc ra bản); chặn khởi động khi bản ngoài danh sách (quá cứng, nhất là với bản mới hơn chưa kịp thêm).
