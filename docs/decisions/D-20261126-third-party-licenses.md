## D-20261126-third-party-licenses · Danh sách giấy phép bên thứ ba đi kèm mỗi bản phát hành, sinh bằng cargo-about — 2026-11-26 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: Mỗi bản phát hành kèm `THIRD-PARTY-LICENSES.html` (văn bản giấy phép đầy đủ của mọi crate Rust trong binary,
sinh bằng `cargo-about` từ `about.toml`, cùng danh sách giấy phép được chấp nhận như `deny.toml`): nằm trong tarball
binary, trong image Docker (`/usr/share/doc/autoxide/`) và đính kèm trang Release (có trong `SHA256SUMS`, có chứng
thực). `cargo deny check licenses` (đã có) tiếp tục chặn giấy phép ngoài danh sách (MIT, Apache-2.0, BSD, ISC, Zlib…);
job `licenses` của CI chạy `cargo about generate` để mẫu và `about.toml` không hỏng. Gói SDK npm không có phụ thuộc chạy
(chỉ `devDependencies`); job `licenses` của CI kiểm `dependencies` của `sdk/typescript/package.json` rỗng, nên danh sách
bên thứ ba của SDK là rỗng.

Lý do: MIT, BSD, Apache-2.0 buộc giữ thông báo bản quyền và văn bản giấy phép khi phân phối bản nhị phân; SBOM (đã có) chỉ
liệt kê tên, không thay được. `cargo-about` là công cụ chuẩn của hệ Rust cho việc này, ghim phiên bản.

Đã cân nhắc: chỉ `cargo deny list` (có tên và mã SPDX, không có văn bản giấy phép, không đủ cho nghĩa vụ giữ thông báo);
chép tay văn bản (lệch dần).
