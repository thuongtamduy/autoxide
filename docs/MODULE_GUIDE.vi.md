# Viết một module

[English](MODULE_GUIDE.md)

Tính năng nghiệp vụ nằm trong module, không nằm trong core. Một module là một crate
ở `modules/<tên>`, sở hữu các bảng của mình, khai với core những gì nó cần qua trait
`Module`, và được đăng ký ở đúng một chỗ. Module mẫu
[`modules/notes`](../modules/notes) dùng mọi phần của hợp đồng;
`cargo xtask new-module` tạo module mới từ nó.

## Những luật gate kiểm

- **Một crate, hai thứ công khai.** Module chỉ mở ra `pub mod api;` (kiểu và trait cho
  module khác dùng) và `pub fn module()` (cho binary). Mọi thứ khác là riêng tư. Module
  khác chỉ viết `autoxide_<tên>::api::…`.
- **Chỉ bảng của mình.** Mọi bảng của module `notes` tên `notes_…`, kể cả bảng lịch sử
  migration `notes_migrations`. Không đọc, ghi hay join bảng của module khác, và không
  thêm foreign key sang chúng: chỉ giữ ID.
- **Cả hai database.** Mọi thứ chạy trên SQLite và PostgreSQL. Không dùng kiểu dữ
  liệu, hàm hay cú pháp chỉ một bên có. Test chạy trên cả hai.
- **Logic ở service.** Handler REST và tool MCP nhận đầu vào, kiểm permission, gọi một
  hàm của service rồi trả kết quả. Chúng không bao giờ chạm database: `cargo xtask check`
  chặn handler hay tool mở kết nối.
- **Lỗi có `code` ổn định** mang tiền tố module (`notes.title_taken`) và có bản dịch
  tiếng Việt trong `locales/vi.json`.

`./gate.sh` kiểm tất cả; module xong khi gate xanh.

## Từng bước

### 1. Tạo crate

Sinh nó từ module mẫu:

```bash
cargo xtask new-module bookmarks bookmark   # <module: số nhiều> <entity: một bản ghi>
```

Lệnh này chép `modules/notes` sang `modules/bookmarks` và đổi mọi `notes` / `note` (mọi
kiểu viết hoa: tên crate, `Module::name`, bảng, permission, event, mã lỗi, kiểu, tên
file). Kết quả qua được gate ngay; sau đó viết lại bảng, service, mô tả và
`locales/vi.json` cho đúng tính năng. Để đăng ký:

1. thêm `autoxide-<tên> = { path = "modules/<tên>" }` vào `[workspace.dependencies]`
   của `Cargo.toml` gốc;
2. thêm `autoxide-<tên>.workspace = true` vào `crates/app/Cargo.toml`;
3. thêm một dòng vào `all()` trong `crates/app/src/modules.rs`:
   `Box::new(autoxide_<tên>::module())`. Module dùng trait của module khác đăng ký sau
   module đó.

`Cargo.toml` của crate khai thư viện dạng `<thư viện>.workspace = true`, kèm
`license.workspace = true` và `[lints] workspace = true`.

**Bố cục file.** Bắt đầu với mỗi vai một file, như `notes`: `lib.rs` (cài đặt `Module`),
`api.rs` (hợp đồng cho module khác), `config.rs`, `permissions.rs`, `events.rs`, `entity.rs`,
`migration/`, `service.rs`, `http.rs`, `tools.rs` và `tests.rs`. Khi module vượt khoảng 5.000
dòng, chia phần service theo tính năng, mỗi tính năng một thư mục (`session/`, `user/` của
`auth`; `entries/`, `publishing/` của `content`), còn `lib.rs`, `api.rs`, `config.rs`,
`permissions.rs`, `events.rs`, `entity/`, `migration/`, `http/` và `tools.rs` giữ ở trên cùng,
để module nào cũng đọc theo một cách. Test dài quá khoảng 1.000 dòng thì vào thư mục `tests/`,
mỗi tính năng một file.

### 2. Bảng: migration và entity

Mỗi migration là một file trong `src/migration/` và một dòng ở cuối `all()`. Migration
đã commit thì không sửa — thêm migration mới. Mọi migration có `down`.

- ID là UUIDv7 do ứng dụng sinh (`Uuid::now_v7()`), không bao giờ tự tăng.
- Thời gian lưu UTC; tiền là số nguyên đơn vị nhỏ nhất kèm mã tiền tệ.
- Thêm index cho mọi danh sách và mọi chỗ tra cứu, và index unique cho mọi luật không
  trùng — database, chứ không phải một câu `SELECT` trước đó, quyết khi hai lệnh ghi
  cùng lúc.
- Module ghi event thêm `autoxide_kernel::event::outbox_migration(...)`; module có job
  nền thêm `autoxide_kernel::job::jobs_migration(...)`.

Entity dùng SeaORM (`src/entity.rs`). Đọc bằng `Database::read()`, ghi bằng
`Database::write()`, nhiều lệnh ghi đi cùng nhau thì `Database::begin_write()`. Giữ
transaction ghi ngắn: không gọi mạng, không hash mật khẩu bên trong.

### 3. Permission

Khai trong `src/permissions.rs` dạng `<module>.<việc>` và trả từ `Module::permissions`.
Kiểm permission, không bao giờ kiểm tên role. Role do quản trị viên dựng từ permission
mà mọi module khai.

### 4. Service

`src/service.rs` giữ mọi luật. Nó nhận một `Actor` (ai làm; lấy bằng
`Actor::from(&principal)`) để event ghi lại người làm. Mỗi lệnh ghi ghi event của nó
trong cùng transaction bằng `Outbox::record(&transaction, actor, subject, &event)` —
thay đổi và dòng audit log cùng có, hoặc cùng không.

### 5. REST API và OpenAPI

Route nằm trong `src/http.rs`, đăng ký bằng `OpenApiRouter::routes(routes!(…))`, và
được gắn dưới `/api/v1/<tên>`; module chỉ khai phần đuôi (`/`, `/{id}`).

- Đọc thân JSON bằng `ValidJson<T>` (`T: Deserialize + Validate + ToSchema`). Trường
  nào service kiểm thì chỉ kiểm ở service (không gắn `#[validate]`), và service gom mọi
  lỗi của đầu vào vào một `Error::validation(…)`: người gọi thấy hết lỗi trong một lần.
- Trả `Data<T>`, `Created::new("/<tên>/<id>", …)` (201 kèm header `Location`) cho bản ghi
  mới, `Page<T>` (danh sách, qua `fetch_page` với `limit`/`cursor`/`order`)
  hoặc một `Error` (gửi đi dạng RFC 9457 Problem Details).
- Bản ghi sửa được thì có `updated_at`. `GET` và lệnh sửa của nó trả
  `Tagged { version: row.updated_at, data }` (thêm `ETag`); handler sửa nhận
  `if_match: IfMatch` và đưa `if_match.expected()?` cho service, service thêm
  `updated_at = <phiên bản mong đợi>` vào câu `UPDATE` và trả `version::changed()` (412)
  khi không dòng nào khớp (xem `autoxide_kernel::version`).
- Handler cần người gọi đã đăng nhập nhận `principal: Principal` và gọi
  `principal.require(&PERMISSION)?`. Endpoint công khai (ai cũng gọi được, người đã đăng
  nhập thấy nhiều hơn) nhận `principal: Option<Principal>` và khai
  `security((), ("bearer" = []))`.
- Muốn bắt buộc `If-Match` khi sửa thì gọi `if_match.required()?` thay cho
  `expected()?` (thiếu header: 428).
- Danh sách sắp theo thứ khác thời gian tạo dùng `fetch_page_by` (keyset pagination trên
  `(cột, id)`, có index trên cả hai); tìm theo chữ dùng `autoxide_kernel::search` trên một
  cột viết thường. `GET` trả `ETag` thì tự trả `304` khi `If-None-Match` khớp, không phải
  viết thêm gì.
- Nội dung đã lưu đọc theo khoảng byte bằng `Storage::get_range`, và module chỉ liệt kê
  khoá của mình bằng `Storage::keys_with_prefix("<tên>/")`. Module `media` là ví dụ đầy
  đủ cho các phần này (`modules/media`).
- Viết tài liệu cho mọi handler: một dòng `///` (thành summary), permission cần có, và
  `description` cho mọi response kèm `code` lỗi. Doc comment trên handler và kiểu schema
  viết tiếng Anh, cho người gọi API.
- Khai ai được gọi mỗi endpoint trong `#[utoipa::path]`, ngay sau `path`:
  `extensions(("x-permissions" = json!(["notes.write"])))` (có một trong các permission là đủ;
  `[]` nghĩa là mọi người đã đăng nhập) hoặc `extensions(("x-public" = json!(true)))`. Mô tả
  hay response 403 phải nêu đúng các permission đó. Gate kiểm mọi endpoint: không token → 401,
  người không có permission → 403 `permission_denied`. Kiểm permission trước tiên trong
  handler, trước khi đọc đầu vào, để người không có quyền không bao giờ nhận 422. Tool MCP
  hoặc `.requires(…)` một permission, hoặc khai `.for_any_signed_in()`; tool chỉ đọc phải gọi
  được bằng một permission mức `read`.
- Mức rủi ro của permission (`read`, `write`, `destructive`, `sensitive_data`, `money`) suy từ
  tên (`….read` là `read`, còn lại `write`); khác thì đặt bằng
  `Permission::new(…).with_risk(Risk::Destructive)`.
- Thay đổi phá vỡ: thêm endpoint mới và khai endpoint cũ trong `Module::deprecations`.
  Gate (`cargo xtask api-diff`) so `docs/openapi.json` với bản phát hành gần nhất và từ chối
  thay đổi phá vỡ, nêu từng chỗ và cách làm đúng.
- Tool MCP cũng là hợp đồng, nằm ở `docs/mcp-tools.json`
  (`cargo run -p autoxide -- mcp-manifest > docs/mcp-tools.json`). Thay đổi phá vỡ thì thêm
  tool mới (vd `create_note_v2`) và khai tool cũ bằng
  `Deprecation::tool("create_note", "YYYY-MM-DD").sunset(…).successor("create_note_v2")`
  trong `Module::deprecations`: agent thấy điều đó trong `tools/list` và trong mọi kết quả.

### 6. Event và audit log

Khai event trong `src/events.rs` và trả từ `Module::events`. Dữ liệu dạng
`{"before": …, "after": …}` (bỏ phần không có) và không bao giờ chứa secret. Module
`audit` ghi mọi event; module khác đăng ký nhận qua `Module::subscriptions` và phải chịu
được việc nhận lại cùng một event.

### 7. Tool MCP

`Module::tools` trả các `autoxide_kernel::mcp::Tool`. Mỗi tool nhận một kiểu đầu vào
(`Deserialize + Validate + ToSchema`, trường phẳng), đòi cùng permission với endpoint
REST tương ứng, và gọi cùng hàm service:

```rust
Tool::new("create_note", "Create a note.", move |principal: Principal, input: CreateNoteRequest| {
    let service = service.clone();
    async move {
        let note = service.create(Actor::from(&principal), input.into()).await?;
        Ok::<_, Error>(NoteResponse::from(note))
    }
})
.requires(NOTES_WRITE)
```

Tool chỉ đọc thì đánh dấu `.read_only()`, tool xoá dữ liệu thì `.destructive()`. Tool
liệt kê dùng `PageInput`.

Mọi thứ module khai — permission, event, event nhận, job, lịch, tool, endpoint deprecated,
migration — tự hiện trong sổ module đã cài ở `GET /api/v1/kernel/modules` (permission
`kernel.modules.read`). Không phải viết thêm gì.

### 8. Làm việc với module khác

Công bố một trait trong `src/api.rs` và cung cấp nó trong `Module::start` bằng
`context.provide::<dyn Trait>(…)`. Module khác lấy bằng
`context.require::<dyn Trait>()` và đăng ký sau bạn. Dịch vụ dùng chung của core cũng
theo cách đó: `dyn Mailer` (gửi mail; module `mail`) và `dyn Storage` (lưu file; module
`files`).

### Bản ghi mà module khác gắn dữ liệu vào

Module như `comments` gắn dữ liệu vào bản ghi của module khác mà không phụ thuộc module đó
(`autoxide_kernel::resource`):

- `context.resource_type("shop.item", resolver)` trong `Module::start` cho module khác hỏi bản ghi
  có không, có công khai không, lấy tiêu đề, đường dẫn và ảnh (`Resource::image`). Cài `resolve_many`
  (nhiều ID một câu query, tiêu đề theo ngôn ngữ) khi module khác tra bản ghi của bạn theo lô, vd menu
  của website; loại có cây (danh mục) trả `true` ở `has_children` và liệt kê mục con ở `children`. Cả
  ba có mặc định nên bộ tra cũ vẫn chạy.
- `Module::resource_events` khai event nào của bạn (trong `Module::events`, chủ thể `<loại>:<id>`)
  nghĩa là `ResourceChange::Closed` (không nhận thêm gì) hay `ResourceChange::Removed` (đã gỡ: đóng và
  ẩn mọi thứ gắn vào). Kernel kiểm lúc khởi động.
- `context.badge_source(kind, &["seller"], resolver)` tính nhãn của tác giả trên bản ghi của loại bất
  kỳ, vd `seller` cho người quản lý sản phẩm. Mỗi nhãn của một loại do một module tính; nhãn là mã ổn
  định, không phải câu chữ.
- `context.posting_verifier(…)` (một module trong cả chương trình) trả lời tài khoản đã xác thực để
  đăng nội dung chưa (số điện thoại hay định danh); `context.posting_verification()` để hỏi. Chưa ai
  đăng ký thì mọi tài khoản đăng nhập tính là đã xác thực.

### Dữ liệu cá nhân

Module giữ dữ liệu về người dùng tham gia việc xuất dữ liệu và xoá tài khoản (xem
`autoxide_kernel::privacy`):

- cài `PersonalData` (trả dữ liệu của người dùng dạng JSON, không có secret) và đăng ký
  trong `Module::start` bằng `context.personal_data(…)`; phần đó có mặt trong
  `GET /api/v1/auth/me/export` dưới tên module;
- đăng ký nhận `privacy::USER_DELETED` trong `Module::subscriptions` và xoá các dòng của
  người đó khi nhận (`privacy::deleted_user(event)` cho id). Có thể nhận hai lần.

### Tiền và số dư

Module giữ số tiền, số dư hay giao dịch dùng các công cụ của kernel;
`crates/kernel/tests/money.rs` dùng đủ cả trên một bảng ví mẫu, trên cả hai database.

- **Số tiền là `autoxide_kernel::money::Money`**: số `i64` đơn vị nhỏ nhất của loại tiền
  (cent của `USD`, đồng của `VND`) cùng một `Currency` theo ISO 4217. Không bao giờ
  `f32`/`f64` hay kiểu thập phân của database. `checked_add`, `checked_sub`, `checked_mul` và
  `checked_neg` trả lỗi thay vì tràn số; trộn hai loại tiền là lỗi (`money.currency_mismatch`).
  Chia một số tiền bằng `allocate(&[tỉ lệ])` hay `split(n)`: tổng các phần luôn bằng số ban
  đầu (100 chia 3 thành 34, 33, 33).
- **Lưu** một `Money` thành hai cột `<tên>_minor` (`BIGINT`) và `<tên>_currency` (3 ký tự);
  đọc lại bằng `Money::from_columns`. **Trong JSON** nó là
  `{"amount": "1999", "currency": "USD"}` — `amount` là chuỗi, vì JavaScript làm tròn số lớn
  hơn 2^53. Dùng thẳng `Money` trong kiểu request và response; schema OpenAPI của nó nói rõ
  điều này.
- **Đổi số dư bằng một câu cập nhật có điều kiện**, không bao giờ "đọc, kiểm, ghi":
  `rows::adjust::<Entity, _>(&transaction, Column::BalanceMinor, Column::Id.eq(id), -amount,
  Bounds::NON_NEGATIVE)` trả `0` khi không đủ số dư (hay dòng không còn) và không bao giờ để
  cột tràn số. Các request đồng thời xếp hàng ở dòng đó; số dư không bao giờ xuống dưới mức
  sàn.
- **Khi buộc phải đọc trước khi ghi** (kiểm trạng thái, chuyển giữa hai dòng), khoá dòng
  trước bằng `rows::lock_one` / `rows::lock` trong `Database::begin_write()`. Khoá nhiều dòng
  **theo cùng một thứ tự** — `rows::lock(…, Column::Id)` khoá theo id — để hai transaction
  không chờ nhau thành vòng (deadlock).
- **Chạy lại khi xung đột**: bọc cả thao tác ghi trong `Database::write_with_retry(move ||
  async move { … })`. Nó chạy lại closure khi PostgreSQL báo deadlock hay serialization
  failure, hoặc SQLite bận lâu; lỗi khác trả ngay. Closure tự mở transaction và phải chịu được
  chạy lại: không gửi mail, không gọi mạng, không đổi gì ngoài database bên trong nó.
- **Mã tham chiếu của client chặn trùng lệnh mãi mãi.** Header `Idempotency-Key` chỉ sống 24
  giờ. Lệnh có hậu quả lâu dài (thanh toán, chuyển tiền) nhận một `reference` từ người gọi,
  kiểm bằng `reference::Reference::checked`, và lưu cùng bản ghi bên cạnh
  `reference::fingerprint(&nội_dung)` dưới một unique index trên (người sở hữu,
  `reference`). Chạy lệnh bằng `reference::submit_once`, gọi `reference::replayed` trên bản ghi
  đã lưu trước: cùng mã, cùng nội dung thì trả bản ghi cũ (`Submitted::Replayed`, trả `200`),
  khác nội dung thì lỗi `reference_reused` (422), và các bản gửi trùng đồng thời chỉ tạo một
  bản ghi.
- **Ghi event trong cùng transaction** với thay đổi (`Outbox::record`), để audit log và module
  khác thấy mọi biến động. Audit log là chuỗi băm: `autoxide doctor` báo dòng đầu tiên bị sửa
  hay bị xoá.
- **Không bao giờ sửa hay xoá một giao dịch.** Sửa sai bằng một giao dịch đảo mới
  (`amount.checked_neg()`) trỏ tới giao dịch nó đảo; nhờ vậy số dư luôn giải thích được bằng
  lịch sử.

### 9. Cấu hình, job, giới hạn

- Cấu hình: khai mỗi biến bằng `Setting` và đọc trong `Module::configure` qua
  `ConfigReader` (không bao giờ `std::env::var`); tên bắt đầu bằng `AUTOXIDE_<TÊN>_`.
  Rồi sinh lại `.env.example`:
  `cargo run -p autoxide -- config --env-example > .env.example`.
- Việc nền: `Module::jobs` (bộ chạy), `Module::schedules` (job định kỳ), và
  `context.jobs()` để đưa job vào hàng đợi.
- Route cần thân request lớn hơn mặc định thì khai trong `Module::body_limits`.
- Thứ người cài có thể làm sai (thư mục không ghi được, dịch vụ ngoài không trả lời,
  thiếu dữ liệu ban đầu) thì có một bước kiểm trong `Module::checks`: một `Check` có
  `Probe` trả `Finding` (ok, warning hay failed, kèm gợi ý sửa). Nó chạy trong
  `autoxide doctor`; không được thay đổi dữ liệu. Xem `modules/files/src/doctor.rs`.
- Việc người vận hành chạy tay thỉnh thoảng (làm lại dữ liệu dẫn xuất sau khi đổi cấu
  hình, dọn một lần) là một lệnh trong `Module::tasks`: một `Task` kèm các `TaskArgument`
  và một `TaskRun`, chạy bằng `autoxide task <module>.<lệnh> tên=giá_trị`. Nó gọi service,
  như REST. Xem `modules/media/src/refresh.rs`.

### 10. Bản dịch

Mọi `code` lỗi và khoá mẫu mail có câu tiếng Việt trong `locales/vi.json`, trả từ
`Module::translations`. Câu tiếng Anh nằm trong code, ở chỗ tạo lỗi.

### 11. Test

Test chạm database dùng `TestDatabase::run(async |test| { … })` của
`autoxide-testkit`: mỗi test một database trống, SQLite hay PostgreSQL tuỳ
`TEST_DATABASE_URL`, và gate chạy cả hai.

`TestApp` dựng module như binary dựng — cấu hình, migration, `start`, route, tool MCP và
mọi lớp HTTP của core — rồi gọi nó:

```rust
let app = TestApp::builder(test)
    .setting("AUTOXIDE_NOTES_SOMETHING", "42")         // như một biến môi trường
    .provide::<dyn Storage>(Arc::new(MemoryStorage::default())) // thay cho module khác
    .user("ada", 0xada, &["notes.read", "notes.write"]) // token "ada" → các permission này
    .module(crate::module())
    .build()
    .await
    .unwrap();

let answer = app.post("/api/v1/notes").token("ada").json(json!({"title": "Hi"})).send().await.unwrap();
assert_eq!(answer.status(), StatusCode::CREATED);

let client = app.mcp("ada").await.unwrap();             // một client MCP thật
let result = client.call_tool(tool("notes.list_notes", json!({}))).await.unwrap();
```

`app.database()`, `app.modules()` và `app.api::<dyn Trait>()` cho truy cập phần còn lại.
`modules/notes/src/tests.rs` là ví dụ đầy đủ. Thứ gì ghi đồng thời (tên không trùng, token
dùng một lần, bộ đếm) cần một test đồng thời.

### 12. Hoàn tất

1. Nếu API đổi, sinh lại hợp đồng:
   `cargo run -p autoxide -- openapi > docs/openapi.json`.
2. Trong lúc làm, `./gate.sh --quick` chạy test của module (và của các crate phụ thuộc vào
   nó) trên SQLite trong chưa tới một phút.
3. Chạy `./gate.sh` đầy đủ tới khi xanh: mọi test trên SQLite và PostgreSQL.

## Module ở repo riêng

Module không nhất thiết nằm trong repo này. Để nó trong crate riêng và phụ thuộc vào
core theo tag phát hành:

```toml
[dependencies]
autoxide-kernel = { git = "https://github.com/<owner>/autoxide", tag = "vX.Y.Z" }

[dev-dependencies]
autoxide-testkit = { git = "https://github.com/<owner>/autoxide", tag = "vX.Y.Z" }
```

Dùng phiên bản Rust của bản phát hành đó (`rust-toolchain.toml`) hoặc mới hơn, và cùng
phiên bản lớn của các thư viện công khai (bảng bên dưới), nếu không trình biên dịch báo
hai bản của cùng một kiểu. Để chạy, thêm crate vào một bản
checkout của Autoxide (`crates/app/Cargo.toml` và `crates/app/src/modules.rs`, bước 1).

`cargo xtask check` chỉ chạy trong repo này, nên module để ở nơi khác tự giữ luật của
nó: hai thứ công khai, tiền tố bảng, không foreign key sang module khác, mọi route qua
`routes!`, mã lỗi có bản dịch. Giữ test sở hữu bảng
(`autoxide_testkit::schema_ownership_violations`) và test `TestApp` trên cả hai database.

## Độ ổn định của hợp đồng module

Thứ module dựa vào là **hợp đồng module**: các thứ công khai của `autoxide-kernel` và
`autoxide-testkit`, cùng các thư viện có kiểu xuất hiện trong đó.

| Thư viện | Phiên bản | Gặp ở |
| --- | --- | --- |
| `axum` | 0.8 | handler, extractor |
| `sea-orm`, `sea-orm-migration` | 2 | entity, `Database`, `Module::migrations` |
| `utoipa` | 6 | `ToSchema`, `#[utoipa::path]` |
| `utoipa-axum` | 0.3 | `Module::routes` (`OpenApiRouter`) |
| `tokio` | 1 | runtime async |
| `serde`, `serde_json` | 1 | thân request, event, tham số tool |
| `uuid` | 1 (v7) | ID |
| `chrono` | 0.4 | thời gian |
| `async-trait` | 0.1 | trait gọi qua `dyn` |
| `validator` | 0.21 | `ValidJson<T>` |

Luật core tự giữ:

- Khi phiên bản còn `0.x`, thay đổi phá vỡ hợp đồng chỉ đến ở bản minor (`0.7` → `0.8`)
  và được ghi ở mục **Changed** hoặc **Removed** của `CHANGELOG.md`. Một thư viện trong
  bảng lên phiên bản lớn mới là một thay đổi như vậy.
- `Module` chỉ lớn thêm bằng hàm có thân mặc định: module viết cho bản cũ vẫn biên dịch.
- Kiểu công khai sẽ còn lớn thêm được đánh dấu `#[non_exhaustive]`: dựng chúng bằng hàm
  tạo, không bằng struct literal, và `match` enum của chúng có nhánh `_`.
- Thứ công khai sắp bỏ được đánh dấu `#[deprecated]` (nói dùng gì thay) ít nhất một bản
  phát hành trước khi xoá.

## Danh sách kiểm

- [ ] Crate đã thêm vào workspace và đăng ký trong `crates/app/src/modules.rs`
- [ ] Bảng tên `<module>_…`, ID UUIDv7, index cho mọi danh sách và tra cứu, migration
      có `down`
- [ ] Permission đã khai; mọi handler và tool đòi một permission
- [ ] Logic chỉ ở service; mỗi lệnh ghi ghi event trong cùng transaction
- [ ] Mọi route có tài liệu (summary, permission, mọi response)
- [ ] Tool MCP cho những gì agent cần làm được
- [ ] Mã lỗi có tiền tố và có bản dịch
- [ ] Dữ liệu về người dùng được xuất (`PersonalData`) và xoá khi nhận `USER_DELETED`
- [ ] Tiền dùng `Money`; số dư đổi bằng `rows::adjust`, khoá dòng theo thứ tự id, lệnh có
      hậu quả lâu dài chống trùng bằng `reference`
- [ ] Test trên cả hai database, kể cả ghi đồng thời
- [ ] Đã sinh lại `docs/openapi.json` và `.env.example`; gate xanh
