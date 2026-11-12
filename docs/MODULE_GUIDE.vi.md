# Viết một module

[English](MODULE_GUIDE.md)

Tính năng nghiệp vụ nằm trong module, không nằm trong core. Một module là một crate
ở `modules/<tên>`, sở hữu các bảng của mình, khai với core những gì nó cần qua trait
`Module`, và được đăng ký ở đúng một chỗ. Module mẫu
[`modules/notes`](../modules/notes) dùng mọi phần của hợp đồng; hãy bắt đầu từ một bản
sao của nó.

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

Chép `modules/notes` sang `modules/<tên>` rồi thay `notes` ở mọi chỗ: tên crate
(`autoxide-<tên>`) trong `Cargo.toml`, `Module::name`, tên bảng, tên permission, tên
event, mã lỗi và khoá trong `locales/vi.json`. Rồi:

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
  `principal.require(&PERMISSION)?`.
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

### 10. Bản dịch

Mọi `code` lỗi và khoá mẫu mail có câu tiếng Việt trong `locales/vi.json`, trả từ
`Module::translations`. Câu tiếng Anh nằm trong code, ở chỗ tạo lỗi.

### 11. Test

Test chạm database dùng `TestDatabase::run(async |test| { … })` của
`autoxide-testkit`: mỗi test một database trống, SQLite hay PostgreSQL tuỳ
`TEST_DATABASE_URL`, và gate chạy cả hai. `modules/notes/src/tests.rs` cho thấy cách
dựng module như binary dựng và gọi nó qua REST và qua một client MCP thật. Thứ gì ghi
đồng thời (tên không trùng, token dùng một lần, bộ đếm) cần một test đồng thời.

### 12. Hoàn tất

1. Nếu API đổi, sinh lại hợp đồng:
   `cargo run -p autoxide -- openapi > docs/openapi.json`.
2. Chạy `./gate.sh` tới khi xanh.

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
