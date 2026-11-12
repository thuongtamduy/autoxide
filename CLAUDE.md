# CLAUDE.md

Luật cho AI agent làm việc trong repo này. `AGENTS.md` là liên kết tới file này.
File được nạp vào mọi lượt — chỉ giữ LUẬT và CON TRỎ.

## Dự án

Autoxide: lõi backend Rust, core trước, module cắm vào sau.

- **Dùng gì, vì sao** → [`STACK.md`](STACK.md)
- **Làm gì tiếp** → [`docs/ROADMAP.md`](docs/ROADMAP.md)
- **Đã quyết gì** → [`docs/DECISIONS.md`](docs/DECISIONS.md)

## Mục tiêu — không được quên

Dựng **Autoxide**: một lõi backend bằng Rust **tiêu chuẩn, đầy đủ, nhanh, tối ưu,
dễ phát triển, hiện đại, phù hợp với AI và AI agent**; dễ nâng cấp, dễ bảo trì;
chạy được trên **cả SQLite và Postgres**; có REST + OpenAPI và MCP endpoint. **Core
dựng trước, module nghiệp vụ cắm vào sau.** AI agent quyết và thực hiện xuyên suốt.

Core xong khi mọi bước trong `docs/ROADMAP.md` xanh trên cả hai database.

Bảy tiêu chí chủ dự án đặt ra, và nghĩa cụ thể của từng cái ở đây:

| Tiêu chí          | Nghĩa là                                                                 |
| ----------------- | ------------------------------------------------------------------------ |
| Tiêu chuẩn        | Theo chuẩn ngành thay vì tự chế: REST, OpenAPI, JWT, TOTP, Argon2id, UUIDv7, OTLP |
| Đầy đủ            | Không bỏ dở: mỗi tính năng có test, tài liệu, xử lý lỗi; chạy trên cả hai database |
| Nhanh             | Đo được: đường nóng không query thừa, list có index và pagination, không chặn thread async |
| Tối ưu            | Đo trước rồi mới tối ưu; không đổi tính đúng lấy tốc độ; không thêm thư viện thừa |
| Dễ phát triển     | Thêm module không phải sửa core; ranh giới rõ; một cách làm cho mỗi việc |
| Hiện đại          | Phiên bản ổn định mới nhất của Rust và thư viện; không dùng API đã deprecated |
| Phù hợp AI / agent | Xem mục "Thân thiện với AI" bên dưới                                    |

Mọi việc — kể cả việc chủ dự án giao thêm giữa chừng — phải đối chiếu với mục tiêu
này. Trước khi làm và trước khi báo xong, tự hỏi:

1. Việc này có đưa core tới gần "xong" không, hay đang lạc sang hướng khác?
2. Nó có làm core khó nâng cấp, khó bảo trì hoặc khó cắm module hơn không?
3. Nó có chạy được trên cả SQLite và Postgres không?

Thấy đang lạc thì dừng việc đó, quay lại lộ trình và nói rõ với chủ dự án.

**Autoxide dùng để làm gì:** làm nền cho một **sản phẩm mới** (không phải bản viết
lại của hệ thống nào có sẵn). **Ai dùng:** mọi người đều dùng được — không gắn với
một công ty hay một ngành. **Mã nguồn mở, công khai toàn bộ.** **Module đầu
tiên:** `cms` — chỉ nội dung (D-078). Giao diện là repo riêng, công khai.

Hệ quả cho agent:

- Core phải **trung lập với nghiệp vụ**: không đưa khái niệm của một ngành cụ thể
  (đơn hàng, kho, bệnh nhân…) vào core. Thứ đó thuộc về module.
- Người lạ phải tự cài và dùng được: cấu hình có giá trị mặc định hợp lý, thông báo
  lỗi rõ, tài liệu cài đặt đủ để làm theo mà không cần hỏi ai.
- Không dựng module nghiệp vụ nào ngoài `cms` và module mẫu của lộ trình khi chủ dự
  án chưa quyết. Khái niệm của `cms` (bài viết, trang, slug…) không vào core.
- Repo là công khai: mọi file, commit message, comment và tài liệu đều viết với
  giả định người lạ sẽ đọc. Không ghi tên máy chủ, IP, đường dẫn riêng hay thông
  tin cá nhân của chủ dự án. **Công khai không áp cho secret** — secret không bao
  giờ vào git.

## Luật số một: chạy được trên CẢ SQLite và Postgres

Chủ dự án yêu cầu: **làm gì cũng phải tương thích với cả SQLite và Postgres.** Áp
cho mọi thứ chạm tới database: migration, entity, query, transaction, index, job
queue, test, lệnh CLI, tài liệu hướng dẫn.

- Một tính năng chỉ chạy trên một database là **chưa xong**, kể cả khi gate của
  database kia xanh.
- Không dùng kiểu dữ liệu, hàm hay cú pháp chỉ một bên có. Buộc phải rẽ nhánh thì
  giấu sau một trait và viết test cho cả hai nhánh (chi tiết: mục "Luật dữ liệu").
- Test mới phải chạy trên cả hai. Không đánh dấu bỏ qua test cho một database.
- Không thấy cách làm chạy được trên cả hai thì đổi thiết kế, không hạ yêu cầu.
  Muốn bỏ yêu cầu này cho một trường hợp cụ thể thì phải dừng lại hỏi chủ dự án.

## Agent tự quyết

Chủ dự án yêu cầu: **AI agent quyết định xuyên suốt, không cần con người can
thiệp**, và kết quả phải chuẩn chỉnh, đầy đủ.

- **Tự quyết, không hỏi.** Việc có nhiều cách thì chọn cách đơn giản nhất mà giải
  quyết trọn vấn đề, làm, rồi sang bước kế trong lộ trình.
- **Quyết định có ảnh hưởng lâu dài phải ghi lại** vào `docs/DECISIONS.md` (thêm
  mục mới ở cuối, không sửa mục cũ): đổi thư viện, đổi quy ước, thêm ngoại lệ cho
  một luật ở file này.
- **Quyết định đã chốt trong `docs/DECISIONS.md` không tự đảo.** Thấy cần đổi thì
  ghi mục mới nêu lý do và mục bị thay; riêng các mục ghi "chủ dự án quyết" thì
  dừng lại hỏi.
- **Chỉ dừng lại hỏi khi:** cần secret thật (mật khẩu database production, access key
  S3, token), cần trỏ tên miền, sắp làm việc không hoàn tác được ở bên ngoài máy
  này (deploy production, xoá dữ liệu thật, đẩy lên remote lần đầu), hoặc muốn
  đảo một quyết định của chủ dự án.
- **Chủ dự án mới học Rust.** Báo cáo bằng tiếng Việt, lời thường; nêu kết quả và
  việc cần họ làm, không giả định họ đọc được diff. Trong code, comment giải thích
  VÌ SAO ở những chỗ Rust khác thói quen của TypeScript (ownership, lifetime,
  trait, `async`).

## Thân thiện với AI

Hai nghĩa, cả hai đều là yêu cầu:

**AI agent phát triển được repo này.** Luật ở một chỗ (file này), việc tiếp theo ở
lộ trình, quyết định ở nhật ký, quy trình ở skill, đúng sai do gate phán. Code đặt
tên rõ, file không quá dài, mỗi việc chỉ có một cách làm — để agent phiên sau đọc
là hiểu mà không cần lịch sử trò chuyện.

**AI agent dùng được sản phẩm.** API của Autoxide phải dùng được bởi máy, không chỉ
bởi người:

- OpenAPI là hợp đồng đầy đủ và luôn khớp code; mỗi endpoint có mô tả đủ để gọi
  đúng mà không cần đọc mã nguồn. Tài liệu nằm ở `docs/openapi.json` (sinh bằng
  `autoxide openapi`) và được phục vụ ở `/api/v1/openapi.json`. Mỗi handler cần:
  một dòng `///` (thành summary) và `description` cho từng response.
- Lỗi trả về có `code` ổn định cho máy và `message` nói rõ phải sửa gì.
- Hành vi nhất quán giữa các endpoint: cùng envelope, cùng pagination, cùng cách
  lọc — học một endpoint là biết các endpoint còn lại.
- Lệnh ghi an toàn khi gọi lại (idempotency key), vì agent hay retry.
- Có cách xác thực cho máy (API key), không buộc qua bước đăng nhập tương tác.
- Có MCP endpoint: module khai báo tool, mỗi tool gắn permission và gọi vào service
  layer như REST.

## Chia agent và subagent

Dự án làm bằng Claude Code. Chủ dự án **cho phép chia agent và subagent, bao nhiêu
cũng được**, tuỳ tài nguyên máy và độ khó của việc. Trước khi chia, chạy skill
`split-work`. Ba luật cứng:

- Số agent **co giãn theo máy của người đang dev**, không có con số cố định: đo CPU
  và RAM còn rảnh trước mỗi lần chia, rồi tăng hay giảm theo việc máy có bị quá
  tải không. Không ghi số đo của một máy cụ thể vào repo.
- Mỗi agent biên dịch một git worktree riêng; chia theo ranh giới crate; file dùng
  chung (`Cargo.toml` gốc, lộ trình, nhật ký quyết định) do agent chính giữ.
- Agent chính chịu trách nhiệm kết quả: tự kiểm lại báo cáo của subagent và chạy
  gate trên bản đã gộp trước khi commit.

## Không đoán API thư viện

Phiên bản thư viện ở đây **mới hơn kiến thức có sẵn của model** (SeaORM 2, utoipa
6, sqlx 0.9, hmac 0.13, argon2 0.6…). Trước khi dùng một API:

- đọc tài liệu đúng phiên bản đang ghim: `cargo doc -p <crate> --no-deps` hoặc mã
  nguồn trong `~/.cargo/registry/src/`;
- để trình biên dịch xác nhận (`cargo check`) trước khi viết tiếp phần dựa vào nó;
- không chép cú pháp từ trí nhớ về phiên bản cũ rồi coi là đúng.

## Gate — định nghĩa "xong"

Một bước chỉ xong khi `./gate.sh` xanh. Gate gồm, theo thứ tự:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo xtask check                                    # ranh giới workspace, license, lint
cargo deny check                                     # license, lỗ hổng, nguồn thư viện
cargo test --workspace                               # SQLite
TEST_DATABASE_URL=postgres://… cargo test --workspace  # Postgres (gate tự dựng bằng Docker)
```

Trong các test đó có ba bài giữ tài liệu khớp code: `.env.example` (cấu hình),
`docs/openapi.json` (hợp đồng API, kèm kiểm mỗi endpoint đủ mô tả) và luật sở hữu
bảng. Đổi cấu hình hay API thì sinh lại file tương ứng — thông báo của test ghi
sẵn lệnh.

- Gate đỏ thì sửa code, **không** nới gate, không tắt lint, không đánh dấu test
  bỏ qua. Ngoại lệ thật sự cần thiết phải ghi vào `docs/DECISIONS.md`.
- Code mới phải có test. Thứ liên quan ghi đồng thời (job, idempotency, refresh
  token) phải có test đồng thời chạy trên cả hai database.
- Test chạm database lấy database qua `TestDatabase::run(async |database| { … })`
  của crate `autoxide-testkit`: mỗi test một database trống, tự chọn SQLite hay
  Postgres theo `TEST_DATABASE_URL`. Không tự mở kết nối tới một database cố định.
- `unwrap` / `expect` trong test: viết trong `#[cfg(test)] mod tests` (clippy tự
  miễn) hoặc trong `tests/` với dòng `#![allow(…)]` đầu file như các file sẵn có.
  Trong `src/` không được tắt các lint này — `cargo xtask check` chặn.
- Xong một bước: đánh dấu trong `docs/ROADMAP.md`, rồi `git commit` cục bộ. Không
  push khi chủ dự án chưa yêu cầu.

## Bố cục workspace

| Thư mục          | Crate              | Vai                                                     |
| ---------------- | ------------------ | ------------------------------------------------------- |
| `crates/kernel`  | `autoxide-kernel`  | Nền mọi module dựa vào; không phụ thuộc module nào      |
| `crates/testkit` | `autoxide-testkit` | Đồ dùng cho test; chỉ được khai ở `[dev-dependencies]`  |
| `crates/app`     | `autoxide`         | Binary; nơi duy nhất đăng ký module                     |
| `modules/<tên>`  | `autoxide-<tên>`   | Mỗi module một crate                                    |
| `xtask`          | `xtask`            | Công cụ cho người phát triển (`cargo xtask check`)      |

- Phiên bản và feature flag của thư viện khai ở `[workspace.dependencies]` của
  `Cargo.toml` gốc; crate con chỉ viết `<tên>.workspace = true`.
- Mỗi crate phải có `license.workspace = true` và `[lints] workspace = true`.
- Crate của module chỉ mở ra ngoài đúng hai thứ: `pub mod api;` (hợp đồng: trait và
  kiểu dữ liệu cho module khác dùng) và `pub fn module(…)` (cho binary đăng ký).
  Mọi thứ khác là `mod` riêng tư hoặc `pub(crate)`. Module khác chỉ được viết
  `autoxide_<tên>::api::…`; `api` không tham chiếu phần bên trong (`crate::…`).
- Các luật trên do `cargo xtask check` kiểm trong gate; lý do ở D-031.
- Module cài trait `Module` của `autoxide_kernel::module` (tên, migration, cấu
  hình…) và được đăng ký ở đúng một chỗ: `crates/app/src/modules.rs`. Phần mới của
  core cần module khai thêm gì thì thêm một hàm vào `Module` **kèm cài đặt mặc
  định** — không bắt module cũ phải sửa (D-038).

## Luật kiến trúc

1. **Module sở hữu bảng của mình.** Không đọc, ghi hay join bảng của module khác.
2. **Gọi module khác qua trait nó công bố**, không import phần bên trong. Module
   công bố trait bằng `context.provide::<dyn Trait>(…)` trong `Module::start`, module
   khác lấy bằng `context.require::<dyn Trait>()`; bên cần đăng ký sau bên cung cấp.
3. **Việc phụ đi qua event** (audit, thông báo); event ghi vào database cùng
   transaction với thay đổi: module khai event qua `Module::events`, thêm
   `event::outbox_migration` vào migration, ghi bằng `Outbox::record(&transaction, …)`
   (lấy `Outbox` từ `StartContext::outbox`); module nhận event qua
   `Module::subscriptions`. Bên nhận phải chịu được việc nhận lại một event (D-054).
4. **Không foreign key xuyên module** — chỉ giữ ID. Không có ngoại lệ, kể cả với
   bảng người dùng.
5. **Logic ở service layer.** Handler chỉ: lấy dữ liệu vào → gọi service → trả
   envelope. Không truy vấn database trong handler. Thân JSON nhận bằng
   `ValidJson<T>` (đã kiểm bằng `validator`), không dùng `Json<T>` trần.
6. **Mọi route có tài liệu OpenAPI**, đăng ký qua `utoipa-axum` cùng lúc với route
   (`OpenApiRouter::routes(routes!(…))`, không dùng `.route(…)` trần). Route của
   module `x` nằm dưới `/api/v1/x`; module chỉ khai phần đuôi (`/`, `/{id}`). Thay
   đổi phá vỡ thì làm endpoint mới, khai endpoint cũ qua `Module::deprecations` (kèm
   ngày sunset và endpoint thay thế), không sửa hành vi endpoint cũ (D-059).
7. **Kiểm quyền bằng permission, không bằng tên role.** Module khai permission
   của mình qua `Module::permissions` (`<module>.<việc>`, vd `notes.write`). Handler
   cần đăng nhập nhận `principal: Principal` (`autoxide_kernel::access`) rồi gọi
   `principal.require(&NOTES_WRITE)?`; tài liệu của handler nêu permission cần có.
   Module nghiệp vụ không phụ thuộc module `auth` (D-046).
8. **REST và MCP dùng chung service.** Tool MCP không có logic riêng và không được
   phép làm điều mà REST với cùng permission không làm được.
9. **Core chỉ có API.** Không có giao diện web hay trang quản trị trong repo này;
   giao diện là dự án riêng dùng API. (Trang xem tài liệu API sinh tự động thì được.)

## Luật dữ liệu

- **ID:** UUIDv7 sinh trong ứng dụng. Không dùng ID tự tăng, không dùng hàm sinh
  ID của database.
- **Tiền:** số nguyên đơn vị nhỏ nhất + mã tiền tệ. Không `f32`/`f64`, không kiểu
  thập phân của database.
- **Thời gian:** lưu UTC.
- **Hai database:** không dùng tính năng chỉ SQLite hoặc chỉ Postgres có. Chỗ buộc
  phải rẽ nhánh theo database (vd claim job) thì giấu sau một trait và test cả
  hai nhánh.
- **Đọc và ghi tách kết nối.** Đọc qua `Database::read()`, ghi qua
  `Database::write()`; nhiều lệnh ghi đi cùng nhau thì `Database::begin_write()`
  (không tự gọi `begin()` trên kết nối). Kết nối đọc bị cấm ghi trên cả hai
  database. Transaction ghi phải ngắn: không gọi mạng, không hash mật khẩu bên trong.
- **Migration đã commit thì không sửa** — thêm migration mới. Module khai migration
  qua `ModuleMigrations` (`autoxide_kernel::migration`); mọi bảng của module `x` tên
  là `x_…`, kể cả bảng lịch sử `x_migrations`. Migration phải có `down`.
- **SeaORM là mặc định.** Chỉ viết SQL trực tiếp ở chỗ đã **đo được** là chậm, câu
  SQL đó phải chạy trên cả hai database, và ghi lý do bằng skill `record-decision`.

## Luật code

- `unsafe` bị cấm. Không `unwrap()` / `expect()` / `panic!` ngoài test — trả lỗi.
- Một kiểu lỗi chung cho mọi layer: `autoxide_kernel::error::Error`. Mã lỗi
  (`code`) là hợp đồng với client, đổi mã là thay đổi phá vỡ. Mã viết `snake_case`;
  lỗi của module mang tiền tố `<module>.` (vd `notes.title_taken`).
- Thông báo cho người dùng đi qua i18n key, không viết câu cứng: **`code` của lỗi
  chính là i18n key**; câu tiếng Anh ở chỗ tạo lỗi là bản mặc định, bản dịch tiếng
  Việt nằm ở `locales/vi.json` của crate (cùng chỗ điền `{tên}`), khai qua
  `Module::translations` (D-039, D-057). Thêm `code` mới thì thêm bản dịch —
  `cargo xtask check` chặn `code` thiếu bản dịch. Client không nêu ngôn ngữ thì trả
  **tiếng Anh**.
- Response chỉ có ba hình dạng: `Data<T>` (`{"data": …}`), `Page<T>`
  (`{"data": […], "pagination": {…}}`) và lỗi `Problem` (RFC 9457). Danh sách lấy
  bằng `fetch_page` với tham số `limit` / `cursor` / `order`; không tự viết
  `OFFSET`.
- Doc comment `///` trên kiểu có `ToSchema` / `IntoParams` và trên handler có
  `#[utoipa::path]` được chép vào tài liệu OpenAPI: viết **tiếng Anh**, cho người
  gọi API. Giải thích cho người đọc code thì dùng `//`.
- Log bằng macro của `tracing` với trường có tên (`tracing::info!(user_id, "…")`),
  không `println!`. Không ghi secret, token, mật khẩu hay thân request vào log. Số
  đo ghi bằng macro của `metrics`, tên mang tiền tố `<module>_`, nhãn chỉ nhận giá
  trị thuộc tập nhỏ cố định (không dùng ID hay đường dẫn thô làm nhãn).
- Tên trong code và OpenAPI bằng tiếng Anh. Comment, tài liệu, tên test viết tiếng
  Việt được.
- Secret không vào git: chỉ có `.env.example`, không có `.env`.
- Cấu hình: mỗi biến khai một lần bằng `Setting` và đọc qua `ConfigReader` của
  `autoxide_kernel::config`; không gọi `std::env::var` trực tiếp. Tên biến: tên
  chuẩn ngành nếu có (`DATABASE_URL`), còn lại tiền tố `AUTOXIDE_`. Thêm biến thì
  sinh lại `.env.example` (`cargo run -p autoxide -- config --env-example`).
- Không thêm thư viện khi vài chục dòng tự viết là đủ; thêm thì ghi vào `STACK.md`.

## Luật ngôn ngữ của tài liệu

Hai nhóm tài liệu, hai ngôn ngữ:

- **Cho người ngoài** — `README`, hướng dẫn cài đặt và sử dụng, mô tả trong
  OpenAPI và MCP: viết **tiếng Anh**, kèm bản **tiếng Việt song song** cho README
  và các hướng dẫn (`README.md` + `README.vi.md`). Hai bản phải cùng nội dung; sửa
  một bản thì sửa bản kia trong cùng commit.
- **Nội bộ, cho chủ dự án và agent** — file này, `STACK.md`, `docs/ROADMAP.md`,
  `docs/DECISIONS.md`, skill, comment trong code, báo cáo: viết **tiếng Việt**.

Trong tài liệu tiếng Việt, **thuật ngữ kỹ thuật giữ nguyên tiếng Anh, không dịch**. Từ dịch dễ trùng nghĩa (cổng = gate hay port, khoá = key
hay lock, luồng = thread hay stream) và làm agent hiểu sai.

Luôn để nguyên: gate, port, key, lock, thread, transaction, migration, index,
foreign key, event, job queue, dead-letter queue, retry, backoff, role, permission,
token, rate limiting, login throttling, pagination, i18n, trait, service layer,
handler, endpoint, feature flag, secret, backup, restore, read replica,
single-writer, graceful shutdown, deprecated — và mọi tên thư viện, lệnh, kiểu dữ
liệu. Lần đầu một thuật ngữ khó xuất hiện trong tài liệu dành cho chủ dự án thì
giải thích ngắn bằng tiếng Việt trong ngoặc.

## Phạm vi

Làm theo thứ tự trong `docs/ROADMAP.md`. Không dựng thứ nằm ở bảng "Chưa làm, đã
cân nhắc" của `STACK.md` (GraphQL, microservice, MySQL, MongoDB, multi-tenant,
passkey) khi chủ dự án chưa mở.
