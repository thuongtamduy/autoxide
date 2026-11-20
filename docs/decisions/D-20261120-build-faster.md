## D-20261120-build-faster · Build nhanh hơn: archive test của CI dùng opt-level 0 cho thư viện ngoài; không dùng cargo-hakari; đề xuất tách kernel — 2026-11-20 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Bối cảnh: sau D-20261120-ci-nextest-archive, phần chạy test chỉ còn ~36 giây mỗi shard; đường găng của CI là job
`build tests` (build từ đầu 11–13 phút trên runner 4 CPU). Đo trên một máy 4 CPU, `CARGO_INCREMENTAL=0`,
`CARGO_PROFILE_DEV_DEBUG=0`, `cargo test --workspace --no-run --locked --timings`, target sạch (số tuyệt đối chỉ để so
sánh với nhau; chỉ số tương đối có ý nghĩa).

Số đo (build từ đầu, 4 CPU):

| Cấu hình | Build từ đầu | So với baseline |
| --- | --- | --- |
| Baseline (`opt-level = 1` cho thư viện ngoài, D-299) | 814 giây | — |
| + cargo-hakari (workspace-hack) | 783 giây | −4% (trong sai số) |
| Thư viện ngoài `opt-level = 0` | 630 giây | **−23%** |

Số đo (build lại sau khi sửa, `touch` file `lib.rs`, build `--workspace`; hakari không đổi các số này):

| Sửa | Thời gian |
| --- | --- |
| module lá (`notes`) | 15 giây |
| `orders` | 64 giây |
| `kernel` | 557 giây (gần bằng build từ đầu) |

Phân tích `--timings` (baseline): 625 đơn vị biên dịch, tổng 3119 giây CPU, trong đó crate của workspace 1842 giây (59%),
thư viện ngoài 1277 giây. Máy 4 CPU nên build bị chặn bởi CPU (3119/4 ≈ 780 ≈ 814), không phải đường phụ thuộc. Mỗi
crate của workspace biên dịch hai lần (bản lib cho crate phụ thuộc, bản `--test` cho unit test); `autoxide-kernel` một
mình 265 giây CPU (73 nghìn dòng; `data` 26 nghìn, `ops` 16 nghìn, `http` 6,5 nghìn) và mọi module đứng sau nó.
`cargo tree -d`: bản trùng còn lại (`read-fonts`/`skrifa` do krilla, `rsa`/`rand 0.8` do webauthn_rp, `tower-http 0.6`
do reqwest, `syn 2` và `darling 0.20` do sea-orm/sea-query) đều do thư viện cấp trên ghim, chung ~200 giây CPU
(~6%) và không gỡ được khi chưa đổi thư viện đó. Feature thừa: không còn — `tokio` đã không "full", `rmcp`,
`object_store`, `webauthn_rp`, `krilla`, `image`, `sea-orm` đã tỉa feature (client của `rmcp` chỉ ở dev-dependencies).

Quyết định:

1. **Archive test của CI build thư viện ngoài ở `opt-level = 0`.** `./gate.sh --only build` truyền
   `--config 'profile.dev.package."*".opt-level=0'` cho `cargo nextest archive` và doc-test khi chạy trên GitHub Actions
   (biến `GATE_DEPS_OPT_LEVEL` đè được; đặt rỗng thì không đổi; máy dev và `cargo xtask test-all` giữ `opt-level = 1`
   của D-299). `argon2` và `blake2` vẫn tối ưu hết cỡ (khai riêng trong Cargo.toml thắng `"*"`). Giá: test chạy chậm hơn
   27–43% (`kernel` integration 21,8 → 27,8 giây; `auth` 32,5 → 46,5 giây, 4 thread SQLite). Archive CI chạy mỗi test một
   lần trên 4 shard mỗi database nên phần chạy chỉ thêm ~10–15 giây mỗi shard, còn build bớt ~3 phút: đường găng ngắn đi.
   Với máy dev thì ngược lại (chạy test lặp đi lặp lại), nên không đổi D-299 ở đó.
2. **Không dùng cargo-hakari.** Đã dựng thử (crate `crates/workspace-hack`, `manage-deps` thêm dòng vào 38 `Cargo.toml`):
   build từ đầu −4% và sửa một module không đổi, dưới ngưỡng 10%. Lợi ích duy nhất đo được là khi xen kẽ `cargo test -p X`
   với `--workspace` trên cùng `target/` (`-p autoxide-notes` build lại 224 giây thư viện ngoài, `-p autoxide` 300
   giây; với hakari: 0 giây), mà CI chỉ dùng `--workspace` còn Docker build `--release` riêng. Cái giá: thêm một crate sinh
   tự động, đổi 38 manifest, mỗi lần đổi dependency phải chạy `cargo hakari generate` và gate phải cài thêm công cụ để
   `verify` — trái "một cách làm cho mỗi việc". Nếu sau này vòng dev (xen `-p` và `--workspace`) thành chỗ tốn, mở lại
   quyết định này với số đo đó; tệp cấu hình thử không được giữ.
3. **Đề xuất, chưa làm: tách `autoxide-kernel`.** Sửa kernel build lại 557 giây vì mọi crate phụ thuộc nó; ba hướng cắt
   ít rủi ro nhất: (a) `contracts` + `values` + `error` + `i18n` (kiểu thuần, ít phụ thuộc nặng) thành crate nền
   `autoxide-types`; (b) `data` (26 nghìn dòng, kéo sea-orm) thành `autoxide-data`; (c) `ops`/`http`/`mcp` giữ lại.
   Module chỉ phụ thuộc phần nó cần nên sửa một phần không build lại toàn bộ. Cần một quyết định riêng khi làm (đổi luật
   bố cục crate trong CLAUDE.md và xtask, mọi `Cargo.toml` của module); không làm ở đây.

Đã cân nhắc: hạ `opt-level` cho cả máy dev (làm test chậm hơn 30–40% trong vòng viết → test, mà D-299 đã đo là đáng giá);
`-Zshare-generics`/`codegen-units` (cần nightly hoặc đổi hồ sơ release; chưa có số đo); thay linker (rust-lld đã là mặc
định của toolchain ghim).

Hệ quả: job `build tests` của CI ngắn đi cỡ 3 phút trên runner 4 CPU (số tương đối −23% build từ đầu); sau khi merge cần
một lần chạy trên main để cache `test` được dựng lại với cấu hình mới (khoá cache theo cờ build, lần đầu nguội).
