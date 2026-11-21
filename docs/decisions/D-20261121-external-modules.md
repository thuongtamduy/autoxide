## D-20261121-external-modules · Module ở repo riêng: crate `autoxide` là thư viện, ví dụ chạy thật, cam kết tương thích — 2026-11-21 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định (bước 70 P2, theo hướng "core cho cộng đồng trước" của chủ dự án, D-20261121-community-first):

- **Crate `autoxide` (`crates/app`) là thư viện có một điểm vào công khai: `autoxide::Autoxide`.**
  `Autoxide::new().module(<hàm tạo module>).run()` chạy đúng CLI và máy chủ của binary chính thức, cộng module
  thêm; binary chính thức là `Autoxide::new().run()`. Module thêm đăng ký SAU mọi module chính thức, theo thứ tự
  thêm. `.module` nhận hàm tạo (`fn() -> impl Module`), không nhận giá trị: `Module` không `Clone` mà chương trình
  dựng danh sách module nhiều lần. Mọi thứ khác của crate (lệnh, cấu hình, khởi động) là riêng tư.
- **Ví dụ chạy thật** ở `examples/external-module/`: một workspace Cargo riêng (không thuộc workspace gốc) gồm module
  `bookmarks` và binary `acme`; test trên cả SQLite và Postgres. CI chạy nó ở một job song song của gate
  (`./gate.sh --only example`); gate đầy đủ trên máy cũng chạy, gate nhanh chỉ chạy khi `examples/` đổi. Ví dụ phụ
  thuộc core bằng path và dùng `Cargo.lock` của core (script chép sang) để kiểm đúng bộ thư viện core đã kiểm.
- **Luật cho module ngoài:** `cargo xtask check-module <thư mục>` chạy các luật đọc mã nguồn của `cargo xtask check`
  (bề mặt crate, route, handler không chạm database, lint, cấu hình, SQL viết tay, bản dịch, `tenant_id`, lọc theo
  khách, tool MCP theo phạm vi) trên một crate ngoài repo. Luật nằm trong cấu trúc database do test của testkit kiểm
  (`schema_ownership_violations`, `schema_tenancy_violations`, `leak_check`). Luật bố cục workspace của repo này
  (license chung, vị trí crate, một chương trình test) không áp cho người ngoài.
- **Hai giai đoạn phụ thuộc.** Giai đoạn 1 (dùng ngay): phụ thuộc bằng git tag
  (`autoxide-kernel = { git = "…", tag = "vX.Y.Z" }`). Giai đoạn 2 (crates.io): mọi crate trừ `xtask` và module mẫu
  `notes` sẵn sàng đăng — metadata đủ, phụ thuộc nội bộ có cả `path` lẫn `version = "=X.Y.Z"` (cả bộ phát hành cùng
  nhau, ghim đúng phiên bản), `cargo xtask publish-check` đóng gói và build thử từng crate. **`publish = false` của
  workspace giữ nguyên tới khi chủ dự án bảo đăng** (đăng lên crates.io không hoàn tác được, cần tài khoản của họ;
  phần "chủ dự án quyết" của D-164 còn hiệu lực). Trình tự đăng ở `docs/RELEASING.md`.
- **Cam kết tương thích.** API công khai cho module ngoài gồm: mọi thứ `pub` của `autoxide-kernel` và
  `autoxide-testkit`; `autoxide::Autoxide`; `api` của module chính thức (`autoxide_<tên>::api`); và các thư viện có
  kiểu lộ ra trong đó (bảng ở `docs/MODULE_GUIDE.md`). Trước 1.0: thay đổi phá vỡ chỉ đến ở bản minor (`0.x` →
  `0.(x+1)`), liệt kê ở **Changed** / **Removed** và mục `### Breaking (Rust)` của `CHANGELOG.md`; bản patch không
  phá vỡ; thứ sắp bỏ `#[deprecated]` ít nhất một bản minor trước; `Module` chỉ thêm hàm có thân mặc định; kiểu sẽ lớn
  thêm là `#[non_exhaustive]`; tên migration của bảng phụ trợ kernel không đổi. Từ 1.0: theo SemVer — phá vỡ chỉ ở
  bản major, `cargo semver-checks` chặn (`cargo xtask semver` tự chuyển sang chặn từ `1.0.0`). Nâng phiên bản Rust
  tối thiểu là thay đổi ghi ở **Changed** của bản minor.
- **`cargo-semver-checks` vào CI của PR phát hành:** job `semver` của `ci.yml` chạy `cargo xtask semver --markdown`
  khi PR đổi `version` của `Cargo.toml` gốc và ghi kết quả vào tóm tắt của job, để mục `### Breaking (Rust)` có
  trước khi merge. Còn `0.x` thì chỉ báo cáo (D-242).

Lý do: trước đây muốn chạy module của mình phải thêm crate vào một bản sao của repo (`crates/app/Cargo.toml`,
`modules.rs`) — tức là fork, và mỗi lần core ra bản mới phải gộp lại. Một điểm vào nhỏ giữ được quyền đổi mọi thứ
bên trong binary; hàm tạo thay vì `Vec<Box<dyn Module>>` vì `settings::describe` và `settings::load` mỗi bên dựng
danh sách một lần. Module thêm giữ ở một biến toàn tiến trình (`registry.rs`) vì mọi lệnh đọc cấu hình qua
`settings::load`: luồn tham số qua 16 lệnh chỉ để tới một chỗ thì ồn hơn lợi. Ví dụ là workspace riêng vì đó đúng là
hoàn cảnh của người ngoài: nằm trong workspace gốc thì thừa hưởng lint, phiên bản thư viện và feature của core, che
mất lỗi họ sẽ gặp.

Đã cân nhắc: `Autoxide::modules(Vec<Box<dyn Module>>)` — không dựng lại được danh sách; cho bỏ bớt module chính
thức ở builder — việc của cấu hình bật / tắt module, không của binary; re-export kernel, axum, SeaORM từ `autoxide` —
derive macro sinh đường dẫn cố định nên module vẫn phải khai trực tiếp (D-164); luồn danh sách module qua tham số
từng lệnh; để ví dụ trong workspace gốc; chỉ đăng `autoxide-kernel` và `autoxide-testkit` lên crates.io — binary của
người ngoài cần crate `autoxide`, crate này cần mọi module chính thức, và trộn kernel từ crates.io với `autoxide` từ
git thành hai bản kernel khác kiểu; đặt `publish = true` ngay — một lệnh gõ nhầm với token là không rút lại được.

Thay: phần "To run it, add the crate to a checkout of Autoxide" và "`cargo xtask check` only runs inside this repository" của D-167
