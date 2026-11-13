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
  hàm của service rồi trả kết quả. Chúng không bao giờ chạm database.
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
- Thay đổi phá vỡ: thêm endpoint mới và khai endpoint cũ trong `Module::deprecations`.

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

### Dữ liệu cá nhân

Module giữ dữ liệu về người dùng tham gia việc xuất dữ liệu và xoá tài khoản (xem
`autoxide_kernel::privacy`):

- cài `PersonalData` (trả dữ liệu của người dùng dạng JSON, không có secret) và đăng ký
  trong `Module::start` bằng `context.personal_data(…)`; phần đó có mặt trong
  `GET /api/v1/auth/me/export` dưới tên module;
- đăng ký nhận `privacy::USER_DELETED` trong `Module::subscriptions` và xoá các dòng của
  người đó khi nhận (`privacy::deleted_user(event)` cho id). Có thể nhận hai lần.

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
2. Chạy `./gate.sh` tới khi xanh.

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
- [ ] Test trên cả hai database, kể cả ghi đồng thời
- [ ] Đã sinh lại `docs/openapi.json` và `.env.example`; gate xanh
