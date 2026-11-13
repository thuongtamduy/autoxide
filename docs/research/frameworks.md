# Autoxide so với các framework Rust: nên mượn gì

Bản khảo sát làm nền cho D-164 và bước 27 của lộ trình. Ngày khảo sát: 13/11/2026.

**Độ tin của nguồn.** Dữ kiện về phiên bản và tình trạng lấy từ kết quả tìm kiếm (trang chủ dự
án, docs.rs, bài viết của bên thứ ba), chưa đối chiếu từng tính năng với mã nguồn. Chỗ đánh
**(chưa kiểm)** là chưa xác nhận được.

---

## Kết luận

1. **Autoxide là sản phẩm, có cửa cho người khác viết module** — không là framework (thư viện
   để người khác dựng ứng dụng của họ) ở lúc này. Lý do: thứ làm Autoxide mạnh (luật ranh giới,
   hai database, OpenAPI khớp code, i18n) được kiểm bởi gate *bên trong repo này*; ra khỏi repo
   thì mất. Còn ở sân framework, Loco đã 1.0 và có cộng đồng.
2. **Mượn cái hay của họ, nhưng chỉ cái hợp với luật của ta.** Lệnh sinh module, lệnh `doctor`,
   kiểm lúc khởi động do module khai, lệnh CLI do module khai, test harness dựng sẵn — đều làm
   việc viết module dễ hơn mà không phá ranh giới.
3. **Chuẩn bị để sau này nâng thành framework rẻ.** Cái giá chính của framework trong Rust là
   giữ API công khai ổn định (SemVer) và các thư viện lộ ra trong hợp đồng module (axum, SeaORM,
   utoipa). Làm từ bây giờ: liệt kê rõ hợp đồng, đánh dấu kiểu sẽ lớn thêm, có chính sách
   deprecate cho API Rust.

## Bản đồ

| Tên | Bản chất | Tình trạng | Bài học |
|---|---|---|---|
| axum | Thư viện HTTP của nhóm tokio | Mặc định của hệ sinh thái; Autoxide, Loco, Cot, TrailBase dựng trên nó | Ta là tầng trên, không tranh |
| actix-web | Thư viện HTTP | Trưởng thành | Như trên |
| Loco | "Rails của Rust": axum + SeaORM 2, job, mailer, auth, generator | 1.0 khoảng 7/2026; ~8 nghìn sao, ~350 nghìn lượt tải | Generator, `doctor`, task, test helper, mẫu dự án |
| Cot | "Django của Rust": ORM tự viết, admin | 0.7, còn 0.x | Tự viết ORM là đắt; mượn tinh thần "system checks" của Django |
| Pavex | Dependency injection lúc biên dịch, lỗi rõ | Còn beta, ít lượt tải | Kiến trúc lạ khó có người dùng; mượn ý "báo lỗi lắp ráp rõ lúc khởi động" |
| Rocket | Từng phổ biến | Chậm hẳn sau 0.5 | Phụ thuộc một người duy trì là rủi ro lớn nhất của framework |
| Poem / Salvo | Thư viện HTTP có OpenAPI dựng sẵn | Cộng đồng nhỏ | OpenAPI dựng sẵn chưa đủ để thắng |
| TrailBase | Sản phẩm: một binary, SQLite, mở rộng bằng WASM | ~v0.30; Postgres thử nghiệm từ v0.28; tài trợ NLnet | Đối thủ gần nhất về hình dạng sản phẩm |
| PocketBase (Go) | Sản phẩm dùng được như thư viện Go | Phổ biến | "Sản phẩm trước, chế độ thư viện sau" là con đường đã chạy được |

## Ta hơn, ta thua

**Hơn** (so với những gì công bố của Loco và TrailBase; chưa so từng tính năng):
ranh giới module do máy kiểm; hai database có test thật trên cả hai; OpenAPI là hợp đồng có test
giữ khớp; MCP; mã lỗi ổn định + i18n; idempotency key; API key; MFA, login throttling, audit,
xuất dữ liệu cá nhân; deprecation có ngày sunset. Đây là **tính năng sản phẩm**.

**Thua:** cộng đồng và độ tin; công cụ cho người viết code (generator, doctor, task, test
helper); mở rộng không cần biên dịch (TrailBase có WASM — D-005 đã bỏ hướng này).

## Mượn gì (bước 27)

| Cái hay | Từ đâu | Làm thành |
|---|---|---|
| Test helper dựng sẵn app + gọi request | Loco (`testing::request`), Cot, axum-test | `TestApp` trong `autoxide-testkit` |
| Sinh code theo khuôn | Loco / Rails generators | `cargo xtask new-module` từ khuôn `notes` |
| Kiểm môi trường một lệnh | `cargo loco doctor` | `autoxide doctor` |
| Kiểm lúc khởi động do từng phần khai | Django system checks | `Module::checks` |
| Lệnh quản trị do module khai | Loco tasks, Django management commands, Rails rake | `Module::tasks` + `autoxide task` |
| Module ở repo riêng | Django reusable apps, Rails engines | Hướng dẫn phụ thuộc kernel qua git tag |
| Hợp đồng ổn định | Rust API Guidelines, cargo-semver-checks | Danh sách public dependency, `#[non_exhaustive]`, chính sách deprecate |

## Không mượn

- **ORM tự viết** (Cot): SeaORM đủ; tự viết là gánh nặng lớn nhất của Cot.
- **Dependency injection lúc biên dịch** (Pavex): đổi cách viết module, ít lợi so với
  `provide` / `require` đang có.
- **Trang admin dựng sẵn** (Cot, TrailBase, PocketBase): core chỉ có API (D-026); giao diện là
  repo riêng.
- **Plugin WASM / nạp lúc chạy** (TrailBase): D-005 đã cân nhắc và bỏ.
- **Auto-CRUD từ bảng** (PocketBase "collections"): vượt qua service layer, trái luật 5.
- **Realtime (SSE theo bảng)** (TrailBase, PocketBase): là tính năng, không là chuẩn của core;
  để khi có module cần.
- **Re-export axum / SeaORM / utoipa từ kernel**: derive macro của SeaORM và utoipa sinh đường dẫn
  `sea_orm::…`, `utoipa::…` cố định, nên module vẫn phải khai trực tiếp; re-export không gỡ được
  ràng buộc phiên bản. Thay bằng danh sách public dependency và quy tắc nâng phiên bản.

## Nguồn

- Loco: <https://loco.rs/>, <https://docs.rs/crate/loco-rs/1.0.1>
- TrailBase: <https://github.com/trailbaseio/trailbase>, <https://nlnet.nl/project/Trailbase/>
- Pavex: <https://www.github.com/LukeMathWalker/pavex>, <https://lpalmieri.com/posts/pavex-is-in-closed-beta>
- Cot: <https://github.com/cot-rs/cot>, <https://docs.rs/cot>
- Tổng quan 2026: <https://www.youngju.dev/blog/culture/2026-05-16-rust-web-backend-frameworks-2026-axum-actix-web-rocket-poem-loco-pavex-salvo-warp-deep-dive.en>
