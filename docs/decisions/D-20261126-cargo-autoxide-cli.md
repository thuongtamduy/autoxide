## D-20261126-cargo-autoxide-cli · Công cụ dòng lệnh độc lập tên `cargo-autoxide`, cài bằng git tag tới khi đăng được crates.io — 2026-11-26 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định (bước 71, G6, mục "công cụ dòng lệnh độc lập"):

1. Công cụ tạo và kiểm module không cần tải mã nguồn core là crate `cargo-autoxide`, dùng như lệnh con của cargo:
   `cargo autoxide new|check|doctor`. Không đặt tên `autoxide new` / `autoxide check`: `autoxide` đã là binary máy chủ
   (và là binary của người viết module ghép module vào), thêm lệnh phát triển vào đó trộn hai đối tượng người dùng.
2. Mã sinh khung (`xtask/src/scaffold.rs`) và luật `check-module` (`xtask/src/external.rs`) chuyển thành một crate thư viện
   dùng chung cho `xtask` và `cargo-autoxide`: một cách làm, không hai bản.
3. Chưa đăng được lên crates.io (cần tài khoản của chủ dự án). Tới lúc đó hướng dẫn cài là
   `cargo install --git https://github.com/thuongtamduy/autoxide --tag vX.Y.Z cargo-autoxide`; khi đăng được thì đổi
   sang `cargo install cargo-autoxide`.
4. Mẫu sinh ra nhắm vào hợp đồng module mới của 1.0 (G1/G2), nên phần code của mục này chờ PR 5 của G1.

Lý do: người mới và người code bằng AI cần tạo và kiểm một module trong vài phút mà không clone repo lớn; tên theo chuẩn
cargo để `cargo install` ra đúng lệnh con.

Đã cân nhắc: `autoxide new` (trùng tên binary máy chủ); giữ `cargo xtask` cho người ngoài (buộc phải có bản checkout của
core, trái mục tiêu); crate đa năng chứa cả máy chủ (kéo toàn bộ core vào công cụ tạo khung).
