# Phiên agent trên cloud (Claude Code)

Mỗi phiên trên cloud là một máy mới, repo được clone lại. Mục tiêu: mở phiên là chạy được `./gate.sh --quick` ngay.
Quyết định: D-20261125-cloud-session-setup.

## Thứ gì cần cho gate

| Công cụ        | Cần cho                                  | Có sẵn trên máy cloud? | Phiên bản (khớp CI) |
| -------------- | ---------------------------------------- | ---------------------- | ------------------- |
| toolchain Rust | mọi thứ                                  | chỉ `stable`; bản ghim phải tải | `rust-toolchain.toml` |
| `cargo-deny`   | `./gate.sh --quick` và gate đầy đủ (lint) | không                  | 0.22.0              |
| `gitleaks`     | gate (binary; không có thì gate cần Docker, mà daemon không chạy sẵn) | không                  | 8.30.1              |
| `cargo-nextest`| `--only build`, `--archive` (CI)         | không                  | 0.9.152             |
| `cargo-semver-checks` | `cargo xtask semver` (hợp đồng Rust của kernel) | không | 0.51.0 |
| Docker         | Postgres/S3 của gate đầy đủ              | có CLI, nhưng daemon không chạy sẵn (`docker info` lỗi) | —  |
| PostgreSQL 16  | test trên Postgres lúc làm việc (D-263)  | có sẵn (`pg_ctlcluster`), nhưng tắt; script bật | 16 (của máy) |

`scripts/cloud-setup.sh` cài bốn công cụ thiếu bằng binary dựng sẵn, kiểm SHA-256 ghim trong script, và cài đúng toolchain.
Máy cloud còn clone nông (shallow) nên `cargo xtask api-diff` không thấy tag phát hành; script chạy `git fetch --unshallow --tags` khi cần.
Script còn sinh mục lục quyết định `docs/decisions/INDEX.md` (`cargo xtask decisions`): file này không nằm trong git, nên máy mới không có sẵn (D-20261125-decision-index-untracked).
Chạy lại nhiều lần được; thứ đã có thì bỏ qua. Không cần secret.

```bash
scripts/cloud-setup.sh          # toolchain + công cụ
scripts/cloud-setup.sh --warm   # thêm cargo fetch, build trước xtask và test của workspace
```

## Chạy test trên Postgres ở máy cloud

Máy cloud có sẵn PostgreSQL 16 (không cần Docker). `scripts/cloud-setup.sh` bật cụm `main`, đặt mật khẩu cho role `postgres`
(chỉ nghe loopback, container dùng xong bỏ) và in giá trị cần đặt. Test chạm database lấy `TEST_DATABASE_URL` (mỗi test một
database trống, tự tạo và xoá):

```bash
export TEST_DATABASE_URL=postgres://postgres:autoxide@127.0.0.1:5432/postgres
CARGO_INCREMENTAL=0 cargo test -p autoxide-<crate> --locked      # chạy riêng bài vừa viết trên Postgres
```

Chưa bật (vd script chưa chạy): `pg_ctlcluster 16 main start`, rồi
`su postgres -c "psql -c \"alter user postgres password 'autoxide'\""`. Không cần `initdb` thư mục riêng. Gate đầy đủ
(`./gate.sh`) tự dựng Postgres bằng Docker; không có daemon thì đặt `GATE_POSTGRES_URL` bằng URL trên.

## Build lạnh và `--warm`

Lần biên dịch đầu của một máy lạnh là phần tốn nhất của gate nhanh. Đo trên máy 4 CPU, `CARGO_INCREMENTAL=0` như gate
(D-20261125-cloud-dev-friction): `cargo test --workspace --no-run` mất **13 phút 28 giây** và chiếm **~12 GB** `target/`;
`cargo clippy --workspace --all-targets` thêm 5 phút 41 giây (gate nhanh bỏ clippy). Có `target/` nóng thì dựng lại không đổi gì
mất **4,6 giây**. `--warm` build sẵn phần test (bỏ qua khi đĩa trống dưới 25 GB). Đừng build với `CARGO_INCREMENTAL` bật rồi
chạy gate: cache incremental thêm ~11 GB và không dùng chung được với gate.

## Hai chỗ gọi script

1. **Setup script của môi trường** (chủ dự án làm một lần, repo không tự bật được). Mở môi trường trên claude.ai
   (menu môi trường ở thanh tiêu đề phiên → Edit → **Setup script**) và dán:

   ```bash
   #!/bin/bash
   # Tìm repo có scripts/cloud-setup.sh (Autoxide); repo khác dùng chung môi trường thì không đụng tới.
   for d in "$PWD" /home/user/*/; do
     if [ -f "$d/scripts/cloud-setup.sh" ] && [ -f "$d/rust-toolchain.toml" ]; then
       (cd "$d" && bash scripts/cloud-setup.sh --warm) || echo "cloud-setup lỗi, bỏ qua (hook SessionStart sẽ thử lại)"
       break
     fi
   done
   exit 0
   ```

   Script chạy với quyền root, bắt đầu ở thư mục ngay trên repo, trước khi Claude Code khởi động; repo đã được clone
   trước đó. Dòng `exit 0` cuối là cố ý: setup script thoát khác 0 thì phiên không khởi động được, và môi trường này có
   thể dùng chung cho repo khác.
   Xong trong khoảng 5 phút thì Anthropic snapshot đĩa và các phiên sau bỏ qua bước này; snapshot dựng lại khi đổi nội
   dung setup script hoặc sau khoảng 7 ngày. Mạng: mức **Limited** với ô **Allow package managers** đã bật là đủ
   (`static.rust-lang.org`, `crates.io`, `github.com` và `release-assets.githubusercontent.com` nằm trong danh sách mặc định).
2. **SessionStart hook** (đã có trong `.claude/settings.json`, chỉ chạy khi `CLAUDE_CODE_REMOTE=true`): gọi cùng script
   mỗi phiên. Khi mọi thứ đã có thì dưới nửa giây; khi snapshot cũ hay chưa dán setup script thì nó cài bù.

Đổi script trong repo (nâng phiên bản công cụ) không làm snapshot dựng lại; hook bắt kịp ở phiên kế tiếp.
Nâng công cụ ở CI thì nâng cả ba dòng (phiên bản, URL, SHA-256) của công cụ đó trong script.

## Giới hạn của máy cloud (theo tài liệu Claude Code)

4 vCPU, 16 GB RAM, 30 GB đĩa; lệnh foreground tối đa 10 phút (lệnh dài hơn chuyển sang nền, tối đa thêm 30 phút).
`./gate.sh` đầy đủ nên chạy nền và đọc log.
