## D-20261120-docker-chef · Image Docker tách lớp thư viện ngoài bằng cargo-chef để cache gha giữ được — 2026-11-20 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Bối cảnh: Dockerfile cũ dùng `--mount=type=cache,target=/src/target`. Cache mount chỉ sống trên đúng máy chạy; runner GitHub
là máy mới mỗi lần, và `cache-to: type=gha` chỉ lưu LỚP, không lưu cache mount. Nên mỗi lần build image (workflow Release,
Release image) biên dịch cả workspace bản release từ đầu (~14 phút mỗi kiến trúc).

Quyết định:

1. Dockerfile nhiều stage: `chef` (cài `cargo-chef` 0.1.78, một lớp độc lập mã nguồn) → `planner` (`cargo chef prepare`
   sinh `recipe.json`: Cargo.toml/Cargo.lock đã gọt, không mã nguồn) → `build` (`cargo chef cook --release --locked -p
   autoxide` = lớp thư viện ngoài, rồi `COPY . .` và `cargo build --release --locked -p autoxide`). Bỏ cache mount của `target/`
   (nằm ngoài lớp nên không cache được); giữ nguyên stage chạy. Khai lại `ARG RUST_VERSION` trong stage (bản cũ dùng
   `${RUST_VERSION}` trong `ENV` mà không khai lại nên rỗng).
2. `recipe.json` không đổi khi chỉ đổi mã nguồn hay số phiên bản phát hành: đã kiểm bằng `cargo chef prepare` trước và sau khi
   đổi `version` trong `Cargo.toml` và `Cargo.lock` (cmp giống hệt). Vậy mỗi lần phát hành chỉ build lại crate của workspace.
3. Workflow `Release image (existing tag)` lấy Dockerfile từ commit của workflow (`.ci-files/Dockerfile`), mã nguồn từ tag:
   tag cũ (v0.14.0) mang Dockerfile cũ không tách lớp. `release.yml` build ở chính tag nên dùng Dockerfile của tag (tag mới đã có
   chef); cả hai dùng chung `cache-from/to: type=gha,scope=<nền tảng>` nên chia sẻ lớp thư viện.
4. `cargo-chef` ghi vào `STACK.md` (công cụ, chỉ chạy trong Docker build).

Đã cân nhắc: build ngoài Docker có `rust-cache` rồi `COPY` binary (hai nhánh build khác môi trường libc/linker so với image,
khó đảm bảo cùng kết quả; arm64 cần máy arm riêng đã có nhưng thêm bước); `cache-to: type=registry` (không bị trần 10 GB của
cache Actions nhưng cần quyền ghi thẻ cache vào ghcr).

Hệ quả: lớp thư viện (~1–2 GB mỗi kiến trúc) chiếm chỗ trong 10 GB cache Actions của repo, đẩy bớt cache của `rust-cache`.
Đổi `Cargo.lock` (thư viện) thì lớp thư viện build lại một lần. Số đo ở mô tả PR.
