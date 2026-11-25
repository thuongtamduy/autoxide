## D-20261125-ci-skip-rust · Pull request không chạm Rust bỏ phần biên dịch và test nặng theo danh sách cho phép — 2026-11-25 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. **Phân loại bằng danh sách CHO PHÉP** `scripts/rust-free-paths.txt` (mẫu đường dẫn, `*` khớp mọi chuỗi, `!` là
   ngoại lệ, dòng đầu tiên khớp quyết định), đọc bởi `scripts/rust-changed.sh <base>` — in `false` khi MỌI file đổi kể
   từ chỗ rẽ nhánh với `<base>` khớp danh sách, còn lại `true`. File không khớp dòng nào là chạm Rust. Danh sách ban
   đầu: `sdk/typescript/*`, `changelog.d/*`, `docs/decisions/*`, `docs/DECISIONS.md`, `docs/ROADMAP.md`,
   `docs/steps/*`, `docs/research/*`, tài liệu ở gốc (README, CONTRIBUTING, SECURITY, CODE_OF_CONDUCT kèm bản `.vi`,
   `STACK.md`, `CLAUDE.md`), `docs/*.md` trừ `docs/guides/*` và `docs/MODULES*`, mẫu issue/PR, `dependabot.yml`, `.claude/*` (cấu hình và skill của agent).
   Không có: `Cargo.*`, `rust-toolchain.toml`, `deny.toml`, `clippy.toml`, `rustfmt.toml`, `gate.sh`, `scripts/`,
   `.github/workflows/`, `examples/`, `xtask/`, `.env.example`, `docs/openapi.json`, `docs/mcp-tools.json`,
   `docs/guides/`, `CHANGELOG.md`, `Dockerfile`, mọi thứ trong thư mục crate (migration, `locales/`…).
2. **Chỉ pull request** dùng phân loại này; push lên `main`, tag (release.yml) và chạy tay luôn chạy đủ. Ở PR mà
   `false`: job `gate / build tests`, `gate / external module example` và mọi shard `gate / sqlite k/n`,
   `gate / postgres 16 k/n` bỏ các bước nặng **ở mức bước** (như `avif`, `upgrade`, `semver` đã làm) nên vẫn báo
   `success` cùng tên; shard đọc quyết định của `build tests` (`needs.build.outputs.run`) và không dựng service
   Postgres/S3 (image rỗng). `gate / lint` vẫn chạy fmt, `cargo xtask check`, api-diff, `cargo xtask sdk --check`,
   cargo-deny, gitleaks; chỉ bỏ clippy và cache của clippy. Job `sdk` (tsc), `workflows`, `secrets` không đổi.
3. **`./gate.sh --pr` và `--quick`** bỏ clippy khi `scripts/rust-changed.sh $GATE_BASE` (mặc định `origin/main`, tính
   cả thay đổi chưa commit) là `false`. Trên CI `GATE_BASE` = nhánh gốc của PR.
4. **Lưới giữ danh sách đúng**: luật `rust-free-paths` trong `cargo xtask check` — chạy cả ở PR bị bỏ phần Rust — đỏ khi
   một file git theo dõi khớp danh sách mà `cargo xtask test-filter` cho là kéo test của crate nào (trong thư mục
   crate, file dùng chung, `include_str!`/`include_bytes!`, file test đọc lúc chạy, và nay cả symlink trong crate trỏ ra
   ngoài — vd `crates/kernel/CHANGELOG.md` → `CHANGELOG.md`, trước đây `test-filter` bỏ sót). Test của xtask so bản bash
   với bản Rust của luật khớp và chạy trên danh sách file thật của PR #460 và #497.

Lý do: nhiều phiên agent song song, mỗi phiên một PR; PR chỉ sửa SDK hay tài liệu vẫn chạy gần trọn CI Rust. Đo trên
GitHub (runner `ubuntu-24.04`, 2026-11-24, PR #460 chỉ sửa `sdk/typescript/package*.json`): lint 6 phút, build tests
9 phút 42 giây, external module example 9 phút 49 giây, rồi 10 shard test 1–3 phút mỗi cái dù filter là `none()`; từ
lúc mở tới xanh 16 phút. Với file trong danh sách, mô hình của `test-filter` vốn đã không chạy test nào (D-20261118-ci-pr-affected-tests),
nên bỏ build và shard không bớt test nào so với trước; clippy và biên dịch không thể đổi kết quả vì không file Rust nào
đọc chúng (luật ở mục 4 bảo đảm). Kiểm rẻ mà vẫn có nghĩa cho các file đó (`cargo xtask check` cho quyết định và
changelog, `sdk --check`, tsc, gitleaks) vẫn chạy: biên dịch xtask từ đầu mất ~15 giây, `cargo xtask check` ~18 giây
(máy 4 nhân, 2026-11-25).

Đã cân nhắc: `paths:`/`paths-ignore:` ở trigger (job biến mất, kẹt check bắt buộc; D-20261121-ci-batch-release-faster);
danh sách loại trừ "bỏ mọi thứ ngoài crate" (file mới chưa ai nghĩ tới sẽ bị bỏ qua — trái "nghi ngờ thì chạy"); phân
loại bằng `cargo xtask` trên CI (phải cài toolchain và biên dịch xtask trước khi quyết, mất cái lợi "xong trong vài
giây"); job `changes` riêng trong ci.yml truyền đầu vào cho gate.yml (thêm một job vào đường găng của mọi PR); skip
`upgrade` theo luật này (nó đã có luật hẹp hơn riêng, giữ nguyên).
