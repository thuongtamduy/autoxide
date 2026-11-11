# Autoxide

[English](README.md) · **Tiếng Việt**

Autoxide là một lõi backend mã nguồn mở viết bằng Rust. Core được dựng trước;
module nghiệp vụ cắm vào sau. Một bộ code chạy được trên **cả SQLite và
PostgreSQL**, có REST API mô tả bằng OpenAPI, và được thiết kế để AI agent dùng
được như con người.

Autoxide là nền cho sản phẩm mới. Nó không gắn với một công ty hay một ngành nào:
core không chứa khái niệm nghiệp vụ — những thứ đó thuộc về module.

## Trạng thái

Đang ở giai đoạn đầu. **Bước 0 của lộ trình (khung dự án) đã xong**: workspace
Cargo, bộ luật lint chung, bộ thư viện đã ghim phiên bản và được kiểm trên cả hai
database, kiểm ranh giới tự động, và gate chất lượng. Chưa có máy chủ để chạy; phần
đó tới ở bước 1.

Core sẽ gồm những gì, theo thứ tự dựng (xem [`docs/ROADMAP.md`](docs/ROADMAP.md)):

1. Nền: cấu hình, truy cập database, migration theo module, hợp đồng module, một
   kiểu lỗi chung, HTTP dưới `/api/v1`, OpenAPI, health check, log, metrics,
   tracing, CLI
2. Đăng nhập, role và permission, API key cho máy và AI agent
3. MFA (TOTP và recovery code)
4. Event và audit log chỉ thêm
5. Job queue lưu trong database (không cần Redis)
6. i18n (tiếng Anh, tiếng Việt) và idempotency key
7. Email, đặt lại mật khẩu, xác minh email
8. Lưu file (ổ đĩa cục bộ và S3)
9. MCP endpoint để AI agent gọi hệ thống như công cụ
10. Một module mẫu và hướng dẫn viết module
11. Công cụ và deploy: chép dữ liệu giữa SQLite và PostgreSQL, đo hiệu năng, Docker,
    backup và restore, CI

## Yêu cầu

- [Rust](https://rustup.rs) — `rustup` tự cài đúng toolchain ghim trong
  `rust-toolchain.toml` ở lần đầu bạn chạy `cargo`
- Một trình biên dịch C (SQLite được biên dịch từ mã nguồn và đóng gói kèm binary)
- Để chạy gate chất lượng đầy đủ: [Docker](https://docs.docker.com/get-docker/) và
  [`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny)
  (`cargo install cargo-deny --locked`)

## Bắt đầu

```bash
git clone <repository-url> autoxide
cd autoxide

# Biên dịch và chạy giao diện dòng lệnh
cargo run -p autoxide -- --help

# Chạy test trên SQLite (không cần cài đặt gì thêm)
cargo test --workspace

# Chạy gate chất lượng đầy đủ: định dạng, lint, kiểm ranh giới, kiểm thư viện,
# test trên SQLite, test trên PostgreSQL (dựng trong một container Docker dùng xong bỏ)
./gate.sh
```

Nếu bạn sẽ commit, bật Git hook một lần. Hook quét secret trong phần thay đổi sắp
commit (khi đã cài [`gitleaks`](https://github.com/gitleaks/gitleaks)) và kiểm định
dạng code:

```bash
git config core.hooksPath .githooks
```

### Chạy test trên PostgreSQL của riêng bạn

Test chọn database theo biến `TEST_DATABASE_URL`. Không đặt nghĩa là SQLite. Để dùng
PostgreSQL 16 trở lên, trỏ biến này tới một database bất kỳ trên máy chủ mà người
dùng có quyền `CREATE DATABASE` — mỗi test tự tạo một database trống riêng và xoá
nó sau khi chạy xong:

```bash
TEST_DATABASE_URL=postgres://user:password@localhost:5432/postgres cargo test --workspace
```

`./gate.sh` làm việc này giúp bạn bằng Docker. Muốn gate dùng một máy chủ có sẵn
thì đặt `GATE_POSTGRES_URL` bằng một URL cùng dạng.

## Bố cục dự án

| Đường dẫn        | Là gì                                                                   |
| ---------------- | ----------------------------------------------------------------------- |
| `crates/kernel`  | Nền mà mọi module dựa vào. Không biết module nào.                       |
| `crates/app`     | Binary `autoxide`: nơi duy nhất đăng ký module.                         |
| `crates/testkit` | Đồ dùng cho test: mỗi test một database riêng, trên SQLite hoặc PostgreSQL. |
| `modules/<tên>`  | Mỗi module một crate (chưa có module nào).                              |
| `xtask`          | Công cụ cho người phát triển: `cargo xtask check` kiểm ranh giới workspace. |
| `gate.sh`        | Gate chất lượng. Một thay đổi chỉ xong khi lệnh này xanh.               |

Module sở hữu bảng của mình, chỉ gọi nhau qua trait đã công bố, và không bao giờ
dùng foreign key xuyên module. Gate tự động kiểm các luật này.

## Tài liệu

- [`STACK.md`](STACK.md) — dùng thư viện nào và vì sao
- [`docs/ROADMAP.md`](docs/ROADMAP.md) — dựng gì, theo thứ tự nào
- [`docs/DECISIONS.md`](docs/DECISIONS.md) — nhật ký quyết định
- [`CLAUDE.md`](CLAUDE.md) — luật cho người đóng góp, là người hay AI agent

Bốn tài liệu này điều hành dự án và được viết bằng tiếng Việt, thuật ngữ kỹ thuật
giữ nguyên tiếng Anh. Tài liệu cho người dùng (README này, hướng dẫn cài đặt và sử
dụng, mô tả API) viết bằng tiếng Anh, kèm bản dịch tiếng Việt song song.

## Đóng góp

Chạy `./gate.sh` trước khi commit; một thay đổi chỉ được nhận khi gate xanh trên cả
SQLite và PostgreSQL. Không bao giờ commit secret — chỉ `.env.example` được nằm
trong Git, `.env` thì không.

## License

Phát hành theo một trong hai license, bạn tự chọn:

- Apache License, phiên bản 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

Trừ khi bạn nêu rõ điều ngược lại, mọi đóng góp bạn chủ ý gửi để đưa vào dự án này,
theo định nghĩa trong license Apache-2.0, sẽ được cấp phép kép như trên, không kèm
thêm điều khoản hay điều kiện nào.
