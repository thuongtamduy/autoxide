## D-20261125-cloud-session-setup · Phiên agent trên cloud: một script cài đặt trong repo, gọi từ setup script của môi trường và SessionStart hook — 2026-11-25 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. **Một nguồn duy nhất: `scripts/cloud-setup.sh`.** Cài toolchain Rust (channel đọc từ `rust-toolchain.toml`, không
   viết cứng lần hai) và ba công cụ ngoài mà gate cần: `cargo-deny` (gate `lint`, kể cả `--quick`), `gitleaks` (gate
   dùng binary nếu có, không thì cần Docker) và `cargo-nextest` (chỉ `--only build` / `--archive`). Chạy lại được,
   bước đã có thì bỏ qua, hỏng thì thoát với thông báo nói rõ bước nào. `--warm` thêm `cargo fetch --locked` và build
   trước `xtask`. Cũng `git fetch --unshallow --tags` khi clone nông: máy cloud clone nông và `cargo xtask api-diff` (bước
   của gate) đòi tag phát hành.
2. **Ghim chuỗi cung ứng (D-20261123-supply-chain):** binary dựng sẵn của nhà phát hành (không `cargo install`: biên dịch
   `cargo-deny` rất lâu), ghim phiên bản **và SHA-256** của file tải về trong script; sai checksum thì không cài. Phiên
   bản giữ khớp CI (`cargo-deny` 0.22.0, `cargo-nextest` 0.9.152, `gitleaks` 8.30.1). SHA-256 của `cargo-deny` và
   `gitleaks` khớp file checksum của nhà phát hành; `cargo-nextest` không có file checksum nên ghim giá trị đo lúc ghim.
3. **Hai chỗ gọi, cùng một script:**
   - *Setup script của môi trường* (chủ dự án dán một lần vào cài đặt môi trường trên claude.ai; môi trường có thể dùng
     chung cho repo khác nên đoạn dán tự tìm repo có script, luôn `exit 0`, xem `docs/CLOUD-SESSIONS.md`): chạy trước khi
     Claude Code khởi động, kết quả được snapshot nếu xong trong khoảng 5 phút, nên phiên sau có sẵn công cụ trên đĩa.
   - *SessionStart hook* trong `.claude/settings.json` (chỉ chạy khi `CLAUDE_CODE_REMOTE=true`): lưới an toàn. Snapshot
     hết hạn sau khoảng 7 ngày và setup script không chạy lại khi đổi nội dung script trong repo, nên khi nâng toolchain
     hay công cụ thì hook là chỗ bắt kịp; khi mọi thứ đã có, nó chỉ tốn dưới nửa giây.
4. Không làm: `sccache` và `mold` trên cloud (chỉ CI dùng, chưa đo lợi trên máy này); `cargo binstall` (thêm một công cụ
   phải ghim mà vẫn không kiểm checksum từng file).

Lý do: mỗi phiên cloud là máy mới; không có script thì mỗi phiên tải toolchain (khoảng 20 giây) và `./gate.sh --quick`
thoát ngay vì thiếu `cargo-deny`. Tài liệu của Claude Code ghi setup script chạy dưới quyền root trước khi Claude Code
khởi động, snapshot khi xong trong khoảng 5 phút, chạy lại khi đổi script hoặc sau khoảng 7 ngày; SessionStart hook chạy
mỗi phiên và nằm trong repo nên đi cùng review. Một script cho cả hai chỗ thì không thể lệch phiên bản giữa chúng.

Đã cân nhắc: chỉ setup script (chủ dự án phải tự dán, repo không bật được, và đổi script trong repo không làm
cache dựng lại); chỉ SessionStart hook (không được snapshot, mỗi phiên mới trả giá đầy đủ ở chính lượt đầu của agent).

Ghi chú vận hành: nâng phiên bản một công cụ ở CI (`gate.yml`, `audit.yml`) thì nâng cả ba dòng của công cụ đó trong
`scripts/cloud-setup.sh`. Chưa có gate tự kiểm hai bên khớp nhau.
