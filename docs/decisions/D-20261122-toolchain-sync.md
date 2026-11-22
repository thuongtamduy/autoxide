## D-20261122-toolchain-sync · Ví dụ module ngoài dùng đúng toolchain của core; `cargo xtask check` chặn khi lệch — 2026-11-22 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định (bước 70 P10; phiên điều phối duyệt):

- `examples/external-module/rust-toolchain.toml` ghim cùng `channel` với `rust-toolchain.toml` gốc (nay 1.100.0, trước
  đó lệch ở 1.99.1). Khung `cargo xtask new-module --external` sinh từ ví dụ nên cũng theo.
- Luật `toolchain` mới của `cargo xtask check` (`xtask/src/toolchain.rs`): `channel` của ví dụ, `rust-version` của
  `examples/external-module/Cargo.toml` (so với `Cargo.toml` gốc) và `ARG RUST_VERSION` của `Dockerfile` (so với
  `channel` gốc) phải khớp; lệch thì gate đỏ, chỉ đúng file. Nâng toolchain là sửa mọi chỗ ghim trong cùng một lượt.

Lý do: lần nâng toolchain trước quên ví dụ; máy chạy `./gate.sh --only example` phải tải thêm một toolchain, và ví dụ
— hình mẫu cho người viết module — build bằng compiler cũ hơn core.

Đã cân nhắc: bỏ `rust-toolchain.toml` của ví dụ để thừa hưởng của gốc — khung sinh ra ở repo riêng sẽ không ghim gì,
mỗi máy build bằng một bản khác; chỉ ghi nhắc trong `docs/RELEASING.md` — dễ quên như lần này.
