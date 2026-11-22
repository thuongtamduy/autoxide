# Viết module ở repo riêng của bạn

English: [`writing-a-module.md`](writing-a-module.md).

Autoxide là một lõi để module cắm vào. Module của bạn không cần nằm trong repo Autoxide, và bạn không phải fork gì: bạn giữ một repo riêng gồm **module của bạn** (một crate thư viện) và **máy chủ của bạn** (một binary năm dòng chạy các module chính thức cộng module của bạn). Module của bạn có mọi thứ một module có sẵn có: cấu hình, migration, permission, event, job nền, route REST trong tài liệu OpenAPI, tool MCP cho AI agent, và bản dịch.

Hướng dẫn này đi từ một thư mục trống tới một module chạy được, có test trên SQLite và PostgreSQL, và nằm trong một máy chủ. Mọi thứ ở đây chính là ví dụ ở [`examples/external-module`](../../examples/external-module), được CI build và test ở mỗi thay đổi — khi phân vân, hãy đọc file ở đó. Tài liệu tra cứu cho từng phần của hợp đồng module là [`docs/MODULE_GUIDE.vi.md`](../MODULE_GUIDE.vi.md).

## Bạn cần gì

- Rust, đúng phiên bản của bản phát hành Autoxide bạn dựa vào (file `rust-toolchain.toml` của nó) hoặc mới hơn; `rustup` tự cài.
- Với SQLite thì không cần gì thêm. Để chạy test trên PostgreSQL: một máy chủ PostgreSQL 16 trở lên và một người dùng được phép `CREATE DATABASE` (ví dụ `docker run --rm -e POSTGRES_PASSWORD=postgres -p 5432:5432 postgres:16-alpine`).

## 1. Workspace

Một repo, hai crate: module và máy chủ. Muốn sinh sẵn cả bố cục này — một bản sao của ví dụ với tên của bạn — chạy `cargo xtask new-module --external <dir> <module> <entity>` trong một bản checkout của Autoxide (ví dụ `cargo xtask new-module --external ../acme bookmarks bookmark`), rồi đọc tiếp để biết từng phần làm gì.

```text
acme/
├── Cargo.toml          the workspace
├── rust-toolchain.toml
├── clippy.toml
├── bookmarks/          your module: crate `acme-bookmarks`
│   ├── Cargo.toml
│   ├── locales/vi.json
│   └── src/
└── server/             your server: crate `acme-server`, binary `acme`
    ├── Cargo.toml
    └── src/main.rs
```

`Cargo.toml` của workspace nêu core một lần. Hãy phụ thuộc vào một **tag phát hành**, để bản build của bạn không đổi cho tới khi bạn chọn bản mới hơn:

```toml
[workspace]
resolver = "3"
members = ["bookmarks", "server"]

[workspace.dependencies]
autoxide = { git = "https://github.com/thuongtamduy/autoxide", tag = "vX.Y.Z" }
autoxide-kernel = { git = "https://github.com/thuongtamduy/autoxide", tag = "vX.Y.Z" }
autoxide-testkit = { git = "https://github.com/thuongtamduy/autoxide", tag = "vX.Y.Z" }

async-trait = "0.1"
axum = "0.8"
sea-orm = { version = "2.0", default-features = false, features = ["macros"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
tracing = "0.1"
utoipa = "6.0"
utoipa-axum = "0.3"
uuid = { version = "1", features = ["v7"] }
validator = { version = "0.21", features = ["derive"] }
```

Ba điều cần biết:

- **Cả ba crate của core dùng cùng một tag.** Khi đó cargo build một bản kernel; hai tag khác nhau cho ra hai kernel có kiểu không khớp nhau.
- **Các thư viện bên dưới core là một phần của hợp đồng module** (`axum`, `sea-orm`, `utoipa`…): giữ đúng phiên bản lớn của bản phát hành bạn dựa vào — bảng "Độ ổn định của hợp đồng module" trong [`MODULE_GUIDE.vi.md`](../MODULE_GUIDE.vi.md) liệt kê chúng. Core đã bật các feature nó cần (driver database, runtime async); bạn chỉ thêm thứ code của bạn dùng. Với `sea-orm`, tắt feature mặc định như trên.
- **Phiên bản thư viện.** Phụ thuộc bằng git không mang theo `Cargo.lock` của core: cargo chọn bản tương thích mới nhất của từng thư viện. Để bắt đầu từ đúng các phiên bản mà bản phát hành đã được kiểm, chép `Cargo.lock` của tag đó vào repo của bạn trước lần build đầu (cargo tự bỏ phần bạn không dùng). Hãy commit `Cargo.lock` của bạn.

[`Cargo.toml` của ví dụ](../../examples/external-module/Cargo.toml) còn đặt các lint của core (không `unsafe`, không `unwrap` ngoài test) và một profile build giữ cho chương trình test nhỏ; hãy chép cả hai, cùng [`clippy.toml`](../../examples/external-module/clippy.toml).

## 2. Crate của module

`bookmarks/Cargo.toml` liệt kê thứ code của module dùng; testkit là dev-dependency:

```toml
[package]
name = "acme-bookmarks"
version = "0.1.0"
edition = "2024"

[dependencies]
async-trait.workspace = true
autoxide-kernel.workspace = true
axum.workspace = true
sea-orm.workspace = true
serde.workspace = true
serde_json.workspace = true
tracing.workspace = true
utoipa.workspace = true
utoipa-axum.workspace = true
uuid.workspace = true
validator.workspace = true

[dev-dependencies]
autoxide-testkit.workspace = true
tokio.workspace = true
```

**Tên** của module — ở đây là `bookmarks` — là tiền tố của mọi thứ nó sở hữu: bảng `bookmarks_…`, route dưới `/api/v1/bookmarks`, permission, event, job, mã lỗi và tool MCP tên `bookmarks.…`. Chọn một tên chưa module chính thức nào dùng.

Crate của module mở ra cho crate khác đúng hai thứ: `pub mod api` (trait và kiểu dữ liệu cho module khác dùng) và `pub fn module()`. Mọi thứ khác là riêng tư. Bắt đầu với mỗi vai một file:

| File | Chứa gì |
| --- | --- |
| [`src/lib.rs`](../../examples/external-module/bookmarks/src/lib.rs) | phần cài `Module`: những gì module khai với core |
| [`src/api.rs`](../../examples/external-module/bookmarks/src/api.rs) | hợp đồng cho module khác |
| [`src/config.rs`](../../examples/external-module/bookmarks/src/config.rs) | cấu hình |
| [`src/migration/`](../../examples/external-module/bookmarks/src/migration/mod.rs) | bảng |
| [`src/entity.rs`](../../examples/external-module/bookmarks/src/entity.rs) | entity SeaORM của từng bảng |
| [`src/permissions.rs`](../../examples/external-module/bookmarks/src/permissions.rs) | permission |
| [`src/events.rs`](../../examples/external-module/bookmarks/src/events.rs) | event |
| [`src/service.rs`](../../examples/external-module/bookmarks/src/service.rs) | toàn bộ logic |
| [`src/http.rs`](../../examples/external-module/bookmarks/src/http.rs) | handler REST |
| [`src/tools.rs`](../../examples/external-module/bookmarks/src/tools.rs) | tool MCP |
| [`src/retention.rs`](../../examples/external-module/bookmarks/src/retention.rs) | một job nền chạy theo lịch |
| [`locales/vi.json`](../../examples/external-module/bookmarks/locales/vi.json) | câu chữ tiếng Việt |
| [`src/tests.rs`](../../examples/external-module/bookmarks/src/tests.rs) | test |

`lib.rs` là mục lục. Mọi hàm của `Module` trừ `name` đều có mặc định, nên module chỉ khai thứ nó có:

```rust
pub mod api;
// … private modules …

pub fn module() -> impl Module {
    BookmarksModule::default()
}

impl Module for BookmarksModule {
    fn name(&self) -> &'static str {
        "bookmarks"
    }

    fn configure(&mut self, reader: &mut ConfigReader<'_>) -> Result<(), Invalid> {
        self.config = Some(BookmarksConfig::read(reader)?);
        Ok(())
    }

    fn migrations(&self) -> Vec<Box<dyn MigrationTrait>> {
        migration::all()
    }

    fn permissions(&self) -> Vec<Permission> {
        permissions::all()
    }

    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
        let config = self.config.clone().unwrap_or_default();
        let service = Arc::new(BookmarkService::new(context.outbox()?, config));
        self.service = Some(service.clone());
        context.provide::<dyn api::Bookmarks>(service)
    }

    fn routes(&self) -> OpenApiRouter {
        self.service.clone().map_or_else(OpenApiRouter::new, http::routes)
    }

    // … events, translations, subscriptions, jobs, schedules, tools …
}
```

Những luật quan trọng nhất, mỗi luật có trong file ví dụ nêu tên:

- **Bảng** (`migration/`, `entity.rs`): tên `bookmarks_…`; ID là UUIDv7 sinh trong ứng dụng; bảng giữ dữ liệu của khách hàng có cột `tenant_id`, và mọi khoá, mọi index bắt đầu bằng cột đó; không foreign key sang bảng của module khác — chỉ giữ ID. Migration viết bằng schema builder, không viết SQL, để một migration chạy được trên cả hai database, và cái nào cũng có `down`. Migration đã phát hành thì không sửa: thêm cái mới.
- **Logic ở service** (`service.rs`): mọi hàm nhận `TenantContext` của khách, đọc qua `tenant.database().read()`, ghi trong một transaction `begin_write()` ngắn, và lọc mọi truy vấn bằng `.scoped(tenant)`. Lệnh ghi lưu event của nó trong cùng transaction (`Outbox::record`).
- **Handler và tool chỉ gọi service** (`http.rs`, `tools.rs`): lấy dữ liệu vào (`ValidJson<T>`), kiểm permission (`principal.require(&BOOKMARKS_WRITE)?`), gọi một hàm của service, trả `Data<T>`, `Page<T>` hoặc `Created<T>`. Route đăng ký bằng `routes!` để có mặt trong tài liệu OpenAPI; dòng `///` và `description` của từng response được chép vào đó, nên viết bằng tiếng Anh cho người gọi API.
- **Lỗi** mang một `code` ổn định có tiền tố module (`bookmarks.url_taken`) và câu tiếng Anh; câu tiếng Việt nằm dưới cùng mã đó trong `locales/vi.json`.
- **Cấu hình** (`config.rs`) khai một lần bằng `Setting` và đọc qua `ConfigReader`, không bao giờ `std::env::var`. Dùng tiền tố riêng của bạn (`ACME_…`); `AUTOXIDE_…` là của core.
- **Luôn cả hai database.** Không dùng kiểu dữ liệu, hàm hay cú pháp chỉ một bên có.

## 3. Máy chủ

`server/Cargo.toml`:

```toml
[package]
name = "acme-server"
version = "0.1.0"
edition = "2024"

[[bin]]
name = "acme"
path = "src/main.rs"

[dependencies]
acme-bookmarks = { path = "../bookmarks" }
autoxide.workspace = true
```

`server/src/main.rs` là toàn bộ chương trình:

```rust
use std::process::ExitCode;

fn main() -> ExitCode {
    autoxide::Autoxide::new()
        .name("acme")
        .version(env!("CARGO_PKG_VERSION"))
        .module(acme_bookmarks::module)
        .run()
}
```

`Autoxide::new()` là chương trình chính thức với mọi module chính thức; `.module(…)` thêm module của bạn, mỗi module một dòng. Module của bạn khởi động sau các module chính thức, theo thứ tự thêm, nên chúng `require` được mọi trait module chính thức công bố, và module của bạn dùng một module khác của bạn thì đứng sau module đó. `.run()` cho binary giao diện dòng lệnh chuẩn.

## 4. Chạy

Chạy từ thư mục workspace. Lần build đầu biên dịch cả core nên mất một lúc.

```bash
cargo run -p acme-server -- config                # every setting, yours included; checks them
cargo run -p acme-server -- migrate up            # creates the tables of every module, yours included
AUTOXIDE_BOOTSTRAP_ADMIN_EMAIL=admin@example.com \
AUTOXIDE_BOOTSTRAP_ADMIN_PASSWORD='correct horse battery staple' \
  cargo run -p acme-server -- serve               # http://127.0.0.1:8080; creates the first administrator
```

Module nào chạy là một biến cấu hình, `AUTOXIDE_MODULES` (xem [`docs/MODULES.vi.md`](../MODULES.vi.md)): mặc định là mọi module chính thức và mọi module bạn thêm; `AUTOXIDE_MODULES=official,-bookmarks` tắt module của bạn, và module được bật kéo theo các module nó nêu trong `Module::depends_on`. `acme modules` liệt kê mọi module kèm hạng, đang bật hay tắt, và phụ thuộc (`--json` cho chương trình). `acme openapi` và `acme mcp-manifest` luôn in hợp đồng đầy đủ.

Không cấu hình gì thì máy chủ giữ dữ liệu trong SQLite dưới `./data`. Dùng PostgreSQL thì đặt `DATABASE_URL=postgres://…`; không gì khác thay đổi. `acme config --env-example > .env.example` ghi ra danh sách mọi biến cấu hình kèm mô tả, và `acme doctor` kiểm một bản cài từng bước.

Ở một terminal khác, đăng nhập rồi gọi module của bạn. Quản trị viên tạo lúc khởi động lần đầu có mọi permission, kể cả những cái module của bạn khai:

```bash
export API=http://127.0.0.1:8080
TOKEN=$(curl -sS "$API/api/v1/auth/login" \
  --json '{"email":"admin@example.com","password":"correct horse battery staple"}' \
  | sed -n 's/.*"access_token":"\([^"]*\)".*/\1/p')
curl -sS "$API/api/v1/bookmarks" -H "Authorization: Bearer $TOKEN" \
  --json '{"url":"https://www.rust-lang.org/","title":"Rust"}'
curl -sS "$API/api/v1/bookmarks" -H "Authorization: Bearer $TOKEN"
```

Route của bạn nằm trong tài liệu API ở `$API/api/v1/openapi.json` (cũng in được bằng `acme openapi`), tool của bạn nằm trong `acme mcp-manifest` và ở MCP endpoint, và mọi thứ module khai được liệt kê ở `GET /api/v1/kernel/modules`.

## 5. Test trên cả hai database

Mỗi test nhận một database trống từ `TestDatabase::run`; `TestApp` dựng module của bạn như máy chủ dựng, rồi gọi nó qua REST và bằng một client MCP thật:

```rust
#[tokio::test]
async fn a_bookmark_can_be_created() {
    TestDatabase::run(async |test| {
        let app = TestApp::builder(test)
            .user("ada", 0xada, &["bookmarks.read", "bookmarks.write"])
            .module(crate::module())
            .build()
            .await
            .unwrap();
        let answer = app
            .post("/api/v1/bookmarks")
            .token("ada")
            .json(json!({"url": "https://www.rust-lang.org/", "title": "Rust"}))
            .send()
            .await
            .unwrap();
        assert_eq!(answer.status(), StatusCode::CREATED);
    })
    .await
    .unwrap();
}
```

Database được chọn bằng `TEST_DATABASE_URL`: không đặt là SQLite, URL `postgres://` là PostgreSQL. Chạy mọi test trên cả hai:

```bash
cargo test --workspace
TEST_DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/postgres \
  cargo test --workspace -- --test-threads=4
```

[`src/tests.rs`](../../examples/external-module/bookmarks/src/tests.rs) của ví dụ cho thấy những test một module nên có: REST và MCP cho cùng kết quả, permission được kiểm, ghi đồng thời (hai request lưu cùng một liên kết) chỉ một bên qua, hai khách không bao giờ thấy nhau (`leak_check`), bảng theo đúng luật (`schema_ownership_violations`, `schema_tenancy_violations`), job và cấu hình chạy đúng. [`server/tests/cli.rs`](../../examples/external-module/server/tests/cli.rs) chạy binary thật: `migrate up`, `openapi` và `mcp-manifest` có module. [`check.sh`](../../examples/external-module/check.sh) chạy định dạng, lint và cả hai lượt test; hãy giữ một script như vậy và chạy nó trong CI.

## 6. Luật của core, kiểm trên crate của bạn

Trong repo Autoxide, `cargo xtask check` giữ các luật của module. Với một crate ở nơi khác, chạy đúng các luật đọc mã nguồn đó từ một bản checkout của Autoxide ở tag bạn dựa vào:

```bash
cargo run --manifest-path /path/to/autoxide/xtask/Cargo.toml -- check-module /path/to/acme/bookmarks
```

Nó kiểm: crate chỉ mở `pub mod api` và `pub fn module`; `api` không với vào phần bên trong của crate; route đi qua `routes!`; handler và tool không chạm database; các lint an toàn không bị tắt trong `src/`; cấu hình không đọc bằng `std::env::var`; câu SQL viết tay dựng bằng `autoxide_kernel::database::statement`; mọi mã lỗi và mô tả có câu tiếng Việt; entity có `tenant_id`; truy vấn lọc theo khách; tool MCP truyền người gọi vào service ở chỗ REST giới hạn theo phạm vi.

Thứ nó không thấy được nằm trong database và do test của bạn kiểm (bước 5): tiền tố bảng, không foreign key sang module khác, `tenant_id` đứng đầu mọi khoá và index, dữ liệu không lọt giữa các khách.

## 7. Nâng lên bản Autoxide mới hơn

1. Đọc `CHANGELOG.md` của bản mới: thay đổi phá vỡ cho module nằm ở **Changed** và **Removed**, phần Rust ở `### Breaking (Rust)`.
2. Đổi tag trong `Cargo.toml` của workspace (cả ba crate của core) rồi build: cargo cập nhật `Cargo.lock` cho các crate của core. Chạy test trên cả hai database.
3. Trên từng máy chủ: `acme migrate up`, rồi khởi động binary mới — như với máy chủ chính thức ([`docs/DEPLOY.vi.md`](../DEPLOY.vi.md)).

Điều core cam kết: khi phiên bản còn `0.x`, thay đổi phá vỡ hợp đồng module chỉ đến ở bản minor (`0.17` → `0.18`), không bao giờ ở bản patch; thứ sắp bỏ được đánh dấu `#[deprecated]` ít nhất một bản minor trước; trait `Module` chỉ lớn thêm bằng hàm có thân mặc định. Từ `1.0` hợp đồng theo Semantic Versioning. Bản đầy đủ ở [`MODULE_GUIDE.vi.md`](../MODULE_GUIDE.vi.md), mục "Độ ổn định của hợp đồng module".

Các crate của core chưa có trên crates.io; cho tới khi có, git tag là cách phụ thuộc vào chúng.

## Danh sách kiểm

- [ ] Crate của module chỉ mở `pub mod api` và `pub fn module`
- [ ] Bảng tên `<module>_…`, ID UUIDv7, `tenant_id` đứng đầu mọi khoá và index, migration có `down`
- [ ] Permission đã khai; mọi handler và tool đòi một permission
- [ ] Logic chỉ ở service; mỗi lệnh ghi lưu event trong cùng transaction
- [ ] Mọi route có tài liệu: tóm tắt, permission, từng response
- [ ] Tool MCP cho những việc agent nên làm được
- [ ] Mã lỗi có tiền tố tên module và có bản dịch
- [ ] Dữ liệu về một người dùng bị xoá khi có `USER_DELETED`
- [ ] Cấu hình mang tiền tố riêng của bạn, khai bằng `Setting`
- [ ] Test trên SQLite và PostgreSQL, gồm ghi đồng thời, cách ly giữa các khách và luật của bảng
- [ ] `cargo xtask check-module` không báo vi phạm
