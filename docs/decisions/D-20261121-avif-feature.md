## D-20261121-avif-feature · AVIF thành feature cargo `avif`, không bật mặc định — 2026-11-21 — agent quyết

Chủ đề: Module nội dung, file, liên lạc

Quyết định: bộ mã hoá AVIF (`ravif` / `rav1e`) của module `media` chỉ có khi build với feature cargo `avif`
(`cargo build --release --features avif`; feature `avif` của `autoxide` chuyển tiếp sang `autoxide-media/avif`). Bản
build mặc định — gồm bản phát hành và image Docker — không có nó. `AUTOXIDE_MEDIA_AVIF=true` trên bản build không có
feature thì máy chủ từ chối khởi động, thông báo nói cách build. Mọi thứ khác của D-20261119-step39-avif giữ nguyên
(job nền, chất lượng, giới hạn, phục vụ bản AVIF đã có). Gate (bước lint) chạy thêm `cargo clippy` và test AVIF của
`media` với `--features avif`, để phần đó không mục khi không ai build nó.

Lý do: `rav1e` 0.8.1 (bản mới nhất) dùng proc-macro `paste`, bị RustSec gắn "không còn bảo trì" (RUSTSEC-2024-0436);
không có bộ mã hoá AVIF Rust thuần nào khác. `cargo deny` xét feature mặc định nên bỏ `rav1e` khỏi bản mặc định là gỡ
được dòng bỏ qua trong `deny.toml` (D-20261121-time-library). AVIF vốn tắt mặc định lúc chạy, nên đa số bản cài không
mất gì; ai cần thì tự build có feature.

Đã cân nhắc: bỏ hẳn AVIF (website muốn ảnh nhẹ hơn WebP một phần ba mất lựa chọn); bọc `libavif` / `aom` (thư viện C,
cần công cụ build hệ thống); `[patch]` `paste` bằng `pastey` (`deny.toml` chỉ nhận crates.io và `[patch]` không đổi tên
crate được); giữ dòng bỏ qua tới khi `rav1e` đổi (không có mốc).

Thay: phần "Ngoại lệ `cargo deny`" của D-20261119-step39-avif
