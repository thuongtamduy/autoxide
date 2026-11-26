# acme-bookmarks

Một module [Autoxide](https://github.com/thuongtamduy/autoxide) ở repo riêng, và máy chủ `acme` chạy nó cùng
các module chính thức. [English](README.md)

| Đường dẫn | Là gì |
| --- | --- |
| `bookmarks/` | module: bảng, service, route REST có OpenAPI, tool MCP, job, cấu hình, test |
| `server/` | binary: các module chính thức cộng module này |
| `check.sh` | định dạng, lint, và mọi test trên SQLite và PostgreSQL |
| `.github/workflows/ci.yml` | CI chạy `check.sh` trên cả hai database |
| `AGENTS.md` | luật cho AI agent làm việc ở đây (`CLAUDE.md` liên kết tới nó); bản ngắn cho Cursor (`.cursor/rules/`) và Copilot (`.github/copilot-instructions.md`) |

## Chạy

```bash
./check.sh                                  # chỉ SQLite
POSTGRES_URL=postgres://postgres:postgres@127.0.0.1:5432/postgres ./check.sh
cargo run -p acme-server -- migrate up
cargo run -p acme-server -- serve
```

Tài liệu API ở `/api/v1/openapi.json`, MCP endpoint ở `/mcp`.

## Tiếp theo

1. Viết lại bảng, service, mô tả và `bookmarks/locales/vi.json` cho tính năng của bạn, và đổi `acme` thành
   tên của bạn.
2. Kiểm luật kiến trúc từ một bản checkout của Autoxide:
   `cargo xtask check-module <đường dẫn>/bookmarks`.
3. Nâng Autoxide: đổi `tag` ở ba dòng `autoxide*` trong `Cargo.toml`.

Mọi phần được giải thích trong hướng dẫn
[Viết module ở repo riêng](https://github.com/thuongtamduy/autoxide/blob/main/docs/guides/writing-a-module.vi.md).
