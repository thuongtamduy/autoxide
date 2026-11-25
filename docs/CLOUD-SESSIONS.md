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
| Docker         | Postgres/S3 của gate đầy đủ              | có CLI, nhưng daemon không chạy sẵn (`docker info` lỗi) | —  |

`scripts/cloud-setup.sh` cài ba công cụ thiếu bằng binary dựng sẵn, kiểm SHA-256 ghim trong script, và cài đúng toolchain.
Máy cloud còn clone nông (shallow) nên `cargo xtask api-diff` không thấy tag phát hành; script chạy `git fetch --unshallow --tags` khi cần.
Chạy lại nhiều lần được; thứ đã có thì bỏ qua. Không cần secret.

```bash
scripts/cloud-setup.sh          # toolchain + công cụ
scripts/cloud-setup.sh --warm   # thêm cargo fetch và build trước xtask
```

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
