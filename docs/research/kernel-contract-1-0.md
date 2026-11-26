# Hợp đồng kernel cho 1.0: hai hạng, lớp thay thư viện, một cách khai module, tách crate, cách ly khách

Thiết kế cho bước 71 nhóm G1 và G2 (D-20261126-road-to-1-0, D-20261126-library-isolation). Tài liệu nội bộ: các quyết định
ngắn gọn nằm ở `docs/decisions/D-20261126-*.md` (liệt kê ở cuối), còn đây là lý lẽ, API mẫu và kế hoạch chuyển. Viết
ngày 26/11/2026, trước khi có dòng code nào; số liệu đo trên `main` cùng ngày (commit `af74d2fb5`).

## 0. Hiện trạng và mục tiêu

| Đo được (26/11/2026)                                   | Con số                                             |
| ------------------------------------------------------ | -------------------------------------------------- |
| Mô-đun công khai của `autoxide-kernel` (`lib.rs`)      | 70                                                 |
| Kernel                                                 | ~86.000 dòng; đa khách riêng ~21.700 dòng          |
| Module (39) khai thẳng `axum` / `sea-orm` / `utoipa` / `validator` / `jiff` / `async-trait` | 36 / 34 / 33 / 31 / 32 / 39 |
| Chỗ module gọi SeaORM: `find()` / `ActiveModel {…}` / `Expr::` / `update_many` / `.join(` / `.lock(` / `OnConflict` / `group_by` | 3.389 / 824 / 1.062 / 419 / 172 / 266 / 198 / 135 |
| `#[utoipa::path]` / `ToSchema` / `#[schema(…)]`        | 1.211 / ~1.500 / ~1.800                             |
| Luật `#[validate(…)]` của module                       | `length` 107, `nested` 4, `email` 3, `range` 2, `url` 1 — không gì khác |
| `jiff::SignedDuration` trong module                    | 239 (còn lại: `TimeZone` 19, `civil::Time` 14, `Weekday` 2) |
| Trait `Module` / hàm đăng ký của `StartContext`         | 49 / ~50                                           |
| Chỗ miễn lint `tenant-scope`                            | 64                                                 |

Mục tiêu đo được khi xong G1 + G2:

1. Module trong repo và module ở repo riêng **không khai** `sea-orm`, `sea-query`, `axum`, `utoipa`, `utoipa-axum`,
   `validator`, `jiff`, `async-trait` (gate chặn). Nâng bản lớn của một thư viện trong danh sách đó chỉ sửa kernel.
2. Chữ ký công khai hạng ổn định của kernel và testkit không có kiểu của các thư viện đó (gate kiểm bằng rustdoc JSON).
3. Trait `Module` còn 4 hàm; mỗi khái niệm một đường đăng ký.
4. Quên lọc khách là lỗi biên dịch hoặc lỗi lúc chạy trên **cả hai** database, không phải lời nhắc của lint đọc text.
5. Đổi một file của phần đa khách không biên dịch lại phần giá trị, database, HTTP.

## 1. Nguyên tắc chung

### 1.1 Ba nhóm thư viện

- **Thư viện từ vựng** (vocabulary): được xuất hiện trong chữ ký ổn định và được module khai trực tiếp. Điều kiện: đã
  `1.x`, có lời hứa ổn định dài, là "ngôn ngữ chung" của hệ Rust. Danh sách đóng: `std`, `serde` 1, `serde_json` 1
  (`Value`), `uuid` 1, `tokio` 1 (chỉ đồng bộ và spawn, không I/O mạng), `tracing` 0.1 (chỉ macro ghi log), `futures-core`
  (trait `Stream`). Thêm vào danh sách là một quyết định. Bản lớn mới của một thư viện từ vựng (rất hiếm: `serde` 2) mới
  là thay đổi lớn của Autoxide.
- **Thư viện có lớp của kernel**: `sea-orm`, `sea-query`, `sqlx`, `axum`, `axum-core`, `http`, `tower`, `utoipa`,
  `utoipa-axum`, `validator`, `jiff`, `async-trait`. Module không khai; kernel và testkit không lộ kiểu của chúng ở hạng ổn
  định. Danh sách nằm ở **một** chỗ (`xtask/wrapped-libraries.txt`) cho cả `xtask check`, `check-module` và kiểm rustdoc.
- **Thư viện riêng của module** (`reqwest`, `lettre`, `image`, `argon2`…): module tự khai, tự nâng; không lọt ra `api` của
  module. Không níu ai, nên không bọc.

### 1.2 Bọc bằng macro của mình, sinh mã vào thư viện qua đường riêng

Kiểu thì bọc bằng kiểu của kernel. Phần khó là **macro** (`#[derive(ToSchema)]`, `#[utoipa::path]`, `DeriveEntityModel`,
`#[derive(Validate)]`): macro của thư viện sinh mã gọi `utoipa::…` theo tên nên crate gọi phải khai thư viện. Cách giải:

- Crate mới `autoxide-macros` (proc-macro, dùng `syn` 2, `quote`, `proc-macro2`) giữ **ngữ pháp** mà module viết. Kernel
  xuất lại các macro đó (`autoxide_kernel::db::Record`, `autoxide_kernel::http::{Schema, endpoint, routes}`…); module
  không khai `autoxide-macros`.
- Mã sinh ra gọi thư viện qua `::autoxide_kernel::__private::{utoipa, axum, sea_query, serde}` (`#[doc(hidden)]`, ngoài
  hợp đồng, gate cấm module viết tay đường này).
- Với `utoipa`: macro của mình **gọi lại** macro của utoipa trong một khối có `use ::autoxide_kernel::__private::utoipa;`
  (bản sao "bóng" của struct cho `ToSchema`, hàm lồng cho `#[utoipa::path]`). Đã thử trên crate nháp với đúng `utoipa`
  6.0.0 / `utoipa-gen` 6.0.1 / `utoipa-axum` 0.3.0: crate dùng chỉ khai `kern` + `serde`, ra đủ path và schema. Chi tiết ở §4.
- Ngữ pháp thuộc tính lúc đầu **trùng tập con** của thư viện (`#[schema(example = …)]`, `#[validate(length(max = 200))]`)
  để chuyển module gần như chỉ đổi `use`. Khi thư viện đổi ngữ pháp, `autoxide-macros` dịch ngữ pháp cũ sang ngữ pháp
  mới — một chỗ sửa. Thuộc tính ngoài tập con được hỗ trợ: macro báo lỗi biên dịch nêu tên thuộc tính và chỗ đọc tài liệu.

Vì sao không tự viết toàn bộ (ORM riêng, bộ sinh OpenAPI riêng): lớn, dễ lệch chuẩn, và không cần — thứ níu chân là **bề
mặt** module chạm vào, không phải phần cài đặt bên trong. Bên trong kernel vẫn dùng SeaORM/sea-query, utoipa, axum; thay
cái bên trong về sau (vd bỏ tầng ORM của SeaORM, chỉ giữ sea-query + sqlx) không chạm module.

Vì sao không chỉ xuất lại thư viện (`pub use sea_orm`): module vẫn viết theo API của thư viện, bản lớn mới vẫn bắt module
sửa — D-20261126-library-isolation đã loại.

## 2. (a) Ba hạng của phần công khai

| Hạng          | Cam kết                                    | Cơ chế                                                                     |
| ------------- | ------------------------------------------ | -------------------------------------------------------------------------- |
| **Ổn định**   | SemVer từ 1.0; deprecate ≥ 1 bản minor trước khi gỡ | Đường thường `autoxide_kernel::<mô-đun>`; `cargo semver-checks` kiểm  |
| **Thử nghiệm** | Đổi được ở bản minor, ghi ở changelog     | Feature `unstable` (tắt mặc định) + đường `autoxide_kernel::unstable::<mô-đun>`; semver-checks chạy với feature mặc định nên không tính |
| **Nội bộ**    | Không cam kết gì; chỉ binary `autoxide` dùng | Crate `autoxide-host` (§6); không phải phụ thuộc của module               |

Chọn feature + mô-đun riêng (như `tokio_unstable`, `unstable` của nhiều crate) thay vì `#[doc(hidden)]` (ẩn cả tài liệu,
người dùng không biết có) hay crate riêng cho mỗi phần thử nghiệm (tăng số crate, đường dẫn đổi khi lên hạng ổn định cũng
là đổi phá vỡ như mô-đun). Module muốn dùng phần thử nghiệm phải bật `features = ["unstable"]` — tự thấy mình đang dựa
vào gì. Workspace bật `unstable` cho binary vì nhiều module chính thức dùng. Lên hạng ổn định = thêm đường ổn định, giữ
đường `unstable::` là bí danh `#[deprecated]` một bản minor.

Rust không cảnh báo `#[deprecated]` trên `pub use` (bí danh), nên lúc chuyển đường dẫn trước 1.0 ta **đổi thẳng** và ghi ở
`### Breaking (Rust)` của changelog, sửa luôn ví dụ `examples/external-module` trong cùng PR (D-20261125-api-break-before-1-0
cho phép trước 1.0, quyết định này nêu rõ).

### 2.1 Phân loại 70 mô-đun hiện có

Tiêu chí: **ổn định** = mọi module cần, hình dạng đã qua ít nhất hai module dùng, không gắn một nhà cung cấp hay một tính
năng còn non. **Thử nghiệm** = hợp đồng của bộ nối nhà cung cấp, tính năng bước 52+ chưa chín, hay hình dạng mới có một
người dùng. **Nội bộ** = module không dùng (cột "module dùng" = 0) hoặc chỉ binary chạy.

| Hạng | Mô-đun |
| --- | --- |
| Ổn định | `error`, `i18n`, `config`, `response`, `http` (phần §4), `module` (phần §5), `event`, `job`, `mcp`, `access`, `db` (mới, §3), `migration` (bản mới, §3.6), `pagination`, `tenancy` (tập con: `TenantContext`, `TenantId`, `OrgId`, `OrgTree`, `TenantSeed`, `Relocation`, `settings`), `time`, `money`, `decimal`, `email`, `slug`, `public_url`, `csv`, `template`, `secret`, `idempotency`, `entitlement`, `privacy`, `version`, `reference`, `rows`, `numbering`, `upload`, `search`, `task`, `doctor`, `throttle`, `antispam`, `ip_hash`, `net`, `tokens`, `storage`, `mail`, `ai`, `identity`, `accounts`, `resource`, `provider` |
| Thử nghiệm | `einvoice`, `zns`, `voice`, `sms`, `push`, `payment`, `blocks`, `analytics`, `report`, `automation`, `custom_fields`, `sync`, `moderation`, `store`, `find`, `suggest`, `summary`, `work`, `usages`, `texts`, `reasons`, `tags`, `bulk`, `list_exports`, `connections`, `resolve`, `tenancy::{sandbox, distribution, moves, domains, site, link_token}` |
| Nội bộ (sang `autoxide-host`) | `backup`, `transfer`, `logging`, `metrics`, `telemetry`, `tls`, `setup`, `builtin`, `module_catalog`, `user_flags`, phần chạy của `migration` (`Migrator`, khoá), `Database::connect`, các route của kernel |
| Sang testkit | `bench_seed` |

`payment`, `sms`, `push` vào thử nghiệm vì đó là hợp đồng của bộ nối nhà cung cấp (D-20261126-road-to-1-0 mục 4); `mail`,
`storage`, `ai` ổn định vì đã có ≥ 2 cài đặt và được 7–8 module dùng. `custom_fields`, `automation` thử nghiệm tới khi G5
chốt hình dạng chính thức. Danh sách này là **điểm xuất phát**: lên hạng là một quyết định mới.

### 2.2 `#[non_exhaustive]` và `#[deprecated]`

- Mọi `pub enum` và `pub struct` có trường công khai của hạng ổn định mà sẽ còn lớn thêm: `#[non_exhaustive]`, kèm hàm dựng
  (D-170). Ngoại lệ phải liệt kê trong `xtask/exhaustive-allow.txt` với lý do một dòng (vd `Order { Asc, Desc }` — đóng
  theo bản chất). Gate kiểm: enum/struct công khai mới thiếu cả hai thì đỏ.
- `#[deprecated(since = "0.x.0", note = "use `…` instead; removed in …")]` viết tiếng Anh, luôn nêu cái thay thế. Gate kiểm
  `note` có chữ "use".
- Gỡ `TenantContext::legacy_default` (0 người gọi) trong PR đầu tiên của nhóm này.

## 3. (b) Lớp dữ liệu `autoxide_kernel::db`

### 3.1 Record thay entity SeaORM

```rust
use autoxide_kernel::db::Record;
use autoxide_kernel::time::Timestamp;
use uuid::Uuid;

/// Bảng `notes_note`.
#[derive(Record, Clone, Debug, PartialEq, Eq)]
#[record(table = "notes_note", tenant)] // `tenant`: bảng cấp khách, có cột `tenant_id`
pub(crate) struct Note {
    #[record(primary_key)]
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub org_id: Option<Uuid>,
    pub owner_id: Uuid,
    pub title: String,
    pub body: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}
```

`#[derive(Record)]` sinh: `impl Record for Note` (tên bảng, danh sách cột, đọc/ghi một dòng), và **cột có kiểu** dạng hằng
liên kết `Note::ID: Col<Note, Uuid>`, `Note::TITLE: Col<Note, String>`… So với SeaORM (cột không mang kiểu giá trị),
`Note::TITLE.eq(42)` là lỗi biên dịch. Không có `ActiveModel`: ghi cả dòng bằng struct, ghi một phần bằng `.set(cột, giá
trị)`. Kiểu trường được nhận: `bool`, `i16`/`i32`/`i64`, `String`, `Uuid`, `Timestamp`, `Date`, `serde_json::Value`,
`Vec<u8>`, `Option<_>` của các kiểu đó, enum lưu dạng text qua `#[derive(DbEnum)]`, và kiểu của module qua trait
`DbValue` (của kernel, chuyển sang/từ `db::Value` của kernel). Tiền vẫn là hai cột (số nguyên + mã tiền tệ), như luật dữ
liệu.

### 3.2 Truy vấn: handle quyết định khách

```rust
let db = tenant.db();                                   // TenantDb — mọi record `tenant` tự lọc theo khách
let note: Option<Note> = db.read().find::<Note>().filter(Note::ID.eq(id)).one().await?;
let page = db.read().find::<Note>().filter(scope).page(Note::ID, &request).await?;

let tx = db.begin_write().await?;                       // TenantTx
let before = tx.find::<Note>().filter(Note::ID.eq(id)).for_update().one().await?;
tx.insert(&note).await?;                                // kiểm note.tenant_id == khách; sai → lỗi, không ghi
let changed: u64 = tx
    .update::<Note>()
    .set(Note::TITLE, title)
    .set(Note::UPDATED_AT, now)
    .filter(Note::ID.eq(id))
    .filter(Note::UPDATED_AT.eq(expected))
    .exec()
    .await?;
tx.delete::<Note>().filter(Note::ID.eq(id)).exec().await?;
tx.commit().await?;
```

- `Database::read()/write()/begin_write()` hiện trả `&DatabaseConnection` / `DatabaseTransaction` của SeaORM. Đổi kiểu
  trả về là phá vỡ cả 29 module cùng lúc, nên lớp mới dùng **tên mới** (`tenant.db()`, `database.db()`), đường cũ được
  `#[deprecated]` và gỡ trước 1.0.
- Tập thao tác (đủ cho mọi mẫu trong bảng §0): `find` / `filter` / `order_by` / `limit` / `one` / `all` / `exists` /
  `count`; `page` / `page_by` / `page_sorted` (thay `fetch_page*`, cùng luật keyset); `select((cột, cột))` +
  `group_by` + `count()`/`sum()`/`min()`/`max()`/`count_distinct()` → `all_as::<(A, B)>()`; `join::<B>(điều kiện)` /
  `left_join`; `for_update()`; `insert` / `insert_many` / `.on_conflict([cột]).do_nothing()` / `.do_update([cột])`;
  `update().set(cột, Expr)` (gồm `Note::COUNT.add(1)`); `delete`; biểu thức `Expr`: cột, giá trị, `+ - *`, `coalesce`,
  `case_when`, `exists(subquery)`, `is_in`, `like` (thoát ký tự đặc biệt sẵn), `is_null`, `and`/`or`/`not`, `cast` sang
  tập kiểu nhỏ.
- `for_update()`: Postgres `FOR UPDATE`; SQLite bỏ qua vì transaction ghi của SQLite đã giữ khoá cả database (một bên ghi).
  Viết ở một chỗ, có test đồng thời trên cả hai.
- `join` chỉ giữa record **cùng module** (cùng tiền tố bảng): kernel kiểm lúc dựng câu, sai thì trả lỗi `internal` — luật
  kiến trúc 1 có cái kiểm thật.
- Lỗi: module không thấy `DbErr`. `insert(...).await` trả `Result<_, Error>` của kernel; vi phạm unique đọc bằng
  `error.conflict()` → `Option<Conflict>` (tên ràng buộc nếu database cho biết) để module đổi sang mã của mình
  (`notes.title_taken`). Lỗi tạm thời (khoá bận, mất kết nối) vẫn đi qua `write_with_retry`.

### 3.3 Khách được kiểm bằng kiểu, không bằng lint

`Record` có kiểu liên kết `type Scope = Tenant | Global` (do thuộc tính `tenant` của derive). Handle:

| Handle                     | Lấy từ                         | Record `tenant`                         | Record `Global`  |
| -------------------------- | ------------------------------ | --------------------------------------- | ---------------- |
| `TenantDb` / `TenantTx`    | `tenant.db()`                  | tự thêm `tenant_id = …`, `insert` kiểm  | chạy thường      |
| `Db` / `Tx`                | `database.db()` (bảng control-plane) | **lỗi biên dịch** (`Scope = Tenant` không thoả bound) | chạy thường |
| `AllTenants`               | `database.db().all_tenants("lý do")` | không lọc; mỗi lần dùng ghi metric `kernel_all_tenant_queries{module}` | chạy thường |

`all_tenants("…")` thay 64 chỗ `// tenant-scope:` — lý do nằm trong code, tìm được bằng `grep`, có số đo lúc chạy. Lint
`tenant-scope` gỡ khi module cuối cùng đã chuyển.

### 3.4 SQL viết tay (phần ngoài lề còn lại)

`db.sql("SELECT … WHERE tenant_id = :tenant AND …").bind("since", since).rows::<Row>()` với `#[derive(FromRow)]` của kernel.
Câu chạy trên `TenantDb` **phải** có tham số `:tenant` (kernel gắn giá trị); thiếu thì trả lỗi trước khi gửi câu. Câu viết
tay vẫn phải chạy trên cả hai database và vẫn cần lý do đo được (luật dữ liệu). Thay `autoxide_kernel::database::statement`
(D-20261121-raw-statement) — giữ đường cũ `#[deprecated]`.

### 3.5 Bên trong

Giai đoạn đầu dựng trên sea-query (dựng câu) + kết nối của SeaORM (chạy câu, transaction, pool) — ít rủi ro nhất vì kernel
đang chạy đúng trên đó. `Col<R, T>` giữ `sea_query::Iden`, `Expr` bọc `sea_query::SimpleExpr`, `db::Value` đổi qua
`sea_query::Value`. Những kiểu này **không** có trong chữ ký công khai. Về sau bỏ tầng ORM của SeaORM (chạy thẳng sqlx) là
việc nội bộ. Mọi API của sea-query/SeaORM dùng lần đầu tra bằng skill `check-library-api` ở PR cài đặt.

### 3.6 Migration: kế hoạch khai báo thay DSL sea-query

```rust
use autoxide_kernel::migration::{Index, Migration, Plan, Table, col};

pub(crate) struct CreateNote;

impl Migration for CreateNote {
    fn name(&self) -> &'static str {
        "m0001_create_note"
    }

    fn up(&self, plan: &mut Plan) {
        plan.create_table(
            Table::new("notes_note")
                .column(col::uuid("id").primary_key())
                .column(col::uuid("owner_id"))
                .column(col::string_len("title", 200))
                .column(col::text("body"))
                .column(col::timestamp("created_at"))
                .column(col::timestamp("updated_at")),
        );
        plan.create_index(Index::unique("notes_note_owner_title_unique").on("notes_note", ["owner_id", "title"]));
        plan.create_index(Index::new("notes_note_owner_idx").on("notes_note", ["owner_id", "id"]));
    }

    fn down(&self, plan: &mut Plan) {
        plan.drop_table("notes_note");
    }
}
```

- `up`/`down` **đồng bộ**, chỉ ghi lại các bước vào `Plan`; kernel dựng SQL cho từng database và chạy. Bước dữ liệu:
  `plan.sql(câu chạy được trên cả hai)` hoặc `plan.data(DataStep)` (trait async của kernel, nhận `Db` không kiểu — migration
  không dùng `Record` vì record đổi theo thời gian còn migration thì đóng băng).
- Vì kế hoạch là dữ liệu, G3 được lợi ngay: "luật an toàn migration" đọc `Plan` (xoá cột, đổi tên, NOT NULL không mặc định)
  thay vì đọc text; `autoxide rollback` biết trước bước `down` làm gì.
- `MigrationTrait` + `SchemaManager` + `migration::prelude` (xuất lại sea-query, SeaORM, async-trait) giữ `#[deprecated]` tới
  khi mọi module đã chuyển; kernel chạy được cả hai loại trong cùng một bảng lịch sử (tên migration không đổi).
- **Viết lại migration đã commit** (để module bỏ được `sea-query`) chỉ được phép khi câu SQL sinh ra **giống từng byte**
  trên cả SQLite và Postgres: thêm test chụp SQL của mọi migration (`xtask/migration-sql/*.sql`, sinh bằng `cargo xtask regen`)
  trước khi chuyển; PR chuyển không được làm đổi file chụp. Đây là ngoại lệ có điều kiện cho luật "migration đã commit thì
  không sửa" (D-20261126-migration-rewrite-proof).

## 4. (b) Lớp HTTP và API

### 4.1 Endpoint và route

```rust
use autoxide_kernel::http::{Path, Query, Routes, State, endpoint, routes};

#[endpoint(
    get,
    path = "/{id}",
    permission = "notes.read",
    params(("id" = Uuid, Path, description = "Id of the note.")),
    responses(
        (status = 200, description = "The note.", body = Data<NoteResponse>, etag),
        (status = 404, description = "No note with this id in your scope; code `notes.note_not_found`.", body = Problem),
    )
)]
/// Get a note.
async fn get_note(…) -> Result<Tagged<NoteResponse>, Error> { … }

pub(crate) fn routes(service: Arc<NoteService>) -> Routes {
    routes![list_notes, create_note, list_note_changes, get_note, update_note, delete_note].with_state(service)
}
```

- `#[endpoint]` (của `autoxide-macros`) sinh, trong thân một hàm ẩn `__endpoint_<tên>()`, lời gọi `#[utoipa::path(…)]`
  với `use ::autoxide_kernel::__private::utoipa;` ở phạm vi khối, rồi trả `Endpoint` của kernel (route axum + mô tả
  OpenAPI). Đã thử chạy (§1.2).
- Ngữ pháp: tập con của `utoipa::path` đang dùng, cộng ba đường tắt cho thứ lặp lại hàng trăm lần: `permission = "…"` (sinh
  `x-permissions`, `security(("bearer" = []))`, response 401/403 chuẩn), `operation_id` mặc định = tên hàm, `etag` trên
  response (header `ETag` chuẩn). Bỏ lặp giúp OpenAPI nhất quán hơn và tài liệu ngắn hơn cho agent đọc.
- `routes![…]` của kernel trả `Routes<S>`; `Module` không còn thấy `OpenApiRouter`.
- Extractor của kernel: `Path<T>`, `Query<T>`, `State<T>`, `Headers`, `Body` (byte thô có giới hạn), cùng `Principal`,
  `TenantContext`, `IfMatch`, `ClientInfo` sẵn có. Extractor của kernel trả lỗi `Problem` có `code` cho mọi lỗi đọc đầu vào
  (axum hiện trả text thô ở vài trường hợp) — lợi thêm cho agent.
- Response của kernel: `Data`, `Page`, `Created`, `Tagged`, `Problem` (sẵn có) + `NoContent` (thay `StatusCode::NO_CONTENT`,
  184 chỗ), `Download` (byte + content type + `Content-Disposition`), `Redirect`. `axum::response::Response` thô chỉ còn trong
  `unstable::http::raw` cho trường hợp hiếm (WebSocket, stream).

### 4.2 Schema và tham số

```rust
use autoxide_kernel::http::Schema;

/// A note.
#[derive(Debug, Serialize, Schema)]
#[schema(as = notes::Note)]
pub(crate) struct NoteResponse { … #[schema(example = "Shopping list")] title: String, … }
```

`#[derive(Schema)]` sinh một bản sao "bóng" của struct (cùng trường, cùng `#[serde(…)]`, cùng `#[schema(…)]`) trong
`const _: () = { use ::autoxide_kernel::__private::utoipa; … }`, cho bản sao `derive(utoipa::ToSchema)`, rồi cài
`ToSchema`/`PartialSchema` của kiểu thật bằng cách trả schema của bản sao với tên của kiểu thật. Bản sao không sinh mã chạy
(chỉ mô tả), nên không tốn gì lúc chạy; tốn thêm thời gian biên dịch của macro — đo ở PR đầu tiên, ngưỡng chấp nhận: thời gian
`cargo check` của workspace tăng không quá 5%. `#[derive(Params)]` (thay `IntoParams`) cùng cách.

Lối thoát: nếu bản lớn sau của utoipa bỏ được cách này, `autoxide-macros` sinh thẳng `ToSchema` bằng builder của utoipa
(hay của bộ sinh OpenAPI khác) — module không đổi.

### 4.3 Kiểm đầu vào

`#[derive(Validate)]` của kernel, ngữ pháp `#[validate(length(min = 1, max = 200))]`, `range(min, max)`, `email`, `url`,
`nested` — đúng năm luật module đang dùng; thêm luật là thêm vào macro. Lỗi trả đúng `FieldError` và mã i18n hiện có
(`validation.length`…) nên OpenAPI và client không đổi. `ValidJson<T>` đổi bound từ `validator::Validate` sang
`autoxide_kernel::http::Validate`; trong thời gian chuyển kernel có cả hai bound qua hai extractor (`ValidJson` cũ
`#[deprecated]`, `Input<T>` mới) — không ép module đổi một lần. Cuối cùng `validator` chỉ còn (nếu còn) bên trong kernel.

### 4.4 Thời gian

`autoxide_kernel::time` thêm `Duration` (thay `jiff::SignedDuration`, 239 chỗ), `TimeZone`, `TimeOfDay`, `Weekday`,
`DateTime` (lịch dân sự) — tiếp tục D-20261121-time-library. `jiff` ra khỏi `Cargo.toml` của module.

### 4.5 `async-trait`

Kernel xuất `autoxide_kernel::async_trait` (macro thuộc tính). Mã nó sinh chỉ dùng `core`/`alloc` nên không cần module khai
crate. Khi Rust ổn định `async fn` gọi qua `dyn` thì kernel bỏ macro đó ở bản lớn kế (có `#[deprecated]` trước).

## 5. (c) Hợp đồng `Module`: manifest + đăng ký lúc chạy

```rust
pub trait Module: Send + Sync {
    /// Everything known without starting: name, tables, permissions, events, settings, texts…
    fn manifest(&self) -> Manifest;
    fn configure(&mut self, config: &mut ConfigReader<'_>) -> Result<(), Invalid> { Ok(()) }
    /// Builds services and registers what needs them: routes, tools, subscriptions, traits for other modules.
    fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error>;
    /// The module is in the binary but turned off: keep personal-data duties (D-20261125-dormant-modules).
    fn dormant(&self, context: &mut DormantContext<'_>) -> Result<(), Error> { Ok(()) }
}
```

- **Luật phân chia, một câu**: thứ cần biết khi chưa khởi động (migration, catalog, sinh tài liệu, `config --env-example`,
  chọn module) đi vào `Manifest`; thứ cần service đi qua `StartContext` trong `start`. Không còn khái niệm nào có hai đường.
- `Manifest` là builder `#[non_exhaustive]`, mỗi khái niệm một hàm: `Manifest::new("notes", "…").stability(…).depends_on([…])
  .migrations(…).permissions(…).events(…).translations(…).limit(…).tenant_setting(…).sync_resource(…)…`. Thêm khái niệm mới
  = thêm hàm builder (không phá vỡ). Manifest là dữ liệu: in ra được thành JSON cho G8 (manifest module của hệ sinh thái) và
  cho `autoxide modules --json`.
- Khái niệm thử nghiệm (`blocks`, `analytics_events`, `funnels`, `automation_templates`, `facts`, `sandbox_copy`,
  `promotable`…) khai qua `manifest.unstable()` có `#[cfg(feature = "unstable")]` — trait `Module` không chứa kiểu thử nghiệm.
- `StartContext` còn: `kernel::<H>()` (handle của kernel: `Outbox`, `SyncLog`, `JobQueue`, `Entitlements`…; thay ~25 getter
  riêng), `provide` / `require` / `later` (trait giữa module, giữ nguyên), `contribute::<dyn Trait>(…)` (thay ~20 hàm đăng ký
  riêng: `work_source`, `finder_source`, `suggest_source`, `summary_metric`, `export_source`, `bulk_action`, `code_resolver`,
  `setup_step`, `badge_source`, `posting_verifier`, `personal_data`…), `routes(…)`, `tools(…)`, `subscribe(event, handler)`.
  Bảng "khái niệm → trait đóng góp" nằm trong `MODULE_GUIDE`.
- Vì sao manifest mà không phải trait con theo nhóm: Rust không tự phát hiện module có cài trait con nào nếu không có
  specialization — phải thêm hàm `as_x(&self) -> Option<&dyn X>` cho mỗi nhóm, vẫn là nhiều đường. Manifest là dữ liệu thuần,
  dễ kiểm, dễ cho agent đọc.

### 5.1 Đường chuyển

1. Thêm `manifest()` có **cài đặt mặc định** dựng `Manifest` từ 45 hàm cũ (gọi chúng với `#[allow(deprecated)]`) — module
   cũ chạy y nguyên. Kernel chỉ đọc `manifest()`.
2. 45 hàm cũ và các hàm đăng ký riêng của `StartContext`: `#[deprecated(note = "declare it in `Module::manifest`…")]`. Rust
   không cảnh báo khi *cài* một hàm trait bị deprecated, nên `xtask check` đếm module còn cài hàm cũ, so với
   `xtask/module-contract-legacy.txt` (chỉ được giảm).
3. Chuyển module theo cụm (§8). Khi danh sách rỗng: gỡ hàm cũ, `manifest()` thành bắt buộc — trước 1.0, là thay đổi phá
   vỡ được ghi (D-20261125-api-break-before-1-0).

## 6. (d) Tách kernel thành crate

Đích (đồ thị một chiều, mũi tên = phụ thuộc):

```
autoxide-macros (proc-macro)
autoxide-values   : error, i18n, config, time, money, decimal, email, slug, public_url, csv, template, secret, ip_hash
     ↑
autoxide-data     : db, migration (Plan), pagination, rows, reference, numbering, version, search (index)
     ↑
autoxide-web      : http (extractor, endpoint, routes, response), mcp (tool, máy chủ)
     ↑
autoxide-platform : access, tenancy, event, job, entitlement, privacy, idempotency, throttle, antispam, storage, mail, ai,
                    identity, accounts, resource, provider, upload, task, doctor, tokens, net, unstable::*
     ↑
autoxide-kernel   : mặt tiền — `pub use` lại mọi đường hiện có + trait `Module`, `Manifest`, `StartContext`
autoxide-host     : chạy server, `Migrator`, backup, transfer, logging/metrics/telemetry, TLS, route của kernel — chỉ `autoxide` dùng
```

- Module và testkit **chỉ** khai `autoxide-kernel` (mặt tiền); các crate con là chi tiết cài đặt (`publish` được nhưng mô tả
  "không dùng trực tiếp"). Đường `autoxide_kernel::x` không đổi, `tests/public_paths.rs` giữ.
- Thứ tự làm: (1) đo đồ thị `use crate::…` giữa các nhóm bằng `cargo xtask kernel-deps` (mới, chỉ đọc) để thấy vòng phụ
  thuộc thật; (2) cắt vòng (thường là kiểu nhỏ dùng chung, chuyển xuống `autoxide-values`); (3) tách từ dưới lên, mỗi crate một
  PR, chỉ di chuyển code; (4) đo `cargo build --timings` trước/sau: sửa một file trong `tenancy` không biên dịch lại
  `values`/`data`/`web`.
- Ranh giới cuối cùng (vd `tenancy` thành crate riêng) quyết sau bước (1) dựa trên số đo, ghi bằng quyết định mới.
- File trên 880 dòng tách theo trách nhiệm trong cùng đợt (chỉ di chuyển code, D-20261118-file-length).

## 7. (e) Cách ly khách lúc chạy

Ba lớp, lớp chính chạy trên cả hai database:

1. **Lớp kết nối gắn khách (chính, cả hai database)** — §3.3: record `tenant` chỉ truy vấn được qua `TenantDb`; quên khách là
   lỗi biên dịch; quét mọi khách phải gọi `all_tenants("lý do")`; SQL viết tay trên `TenantDb` bắt buộc có `:tenant`;
   `insert` kiểm `tenant_id` khớp. Test: mỗi thao tác của `db` có test "khách A không thấy/không sửa được dòng của khách B"
   trên cả hai database; `testkit::leak_check` sẵn có chạy lại trên mọi module đã chuyển.
2. **Row-level security tuỳ chọn trên Postgres (phòng thủ thêm)** — `AUTOXIDE_POSTGRES_ROW_SECURITY=off|on` (mặc định `off`
   tới khi đo xong chi phí, rồi quyết bật mặc định cho database dùng chung). Khi `on`: migration tạo policy cho mọi bảng đã khai
   `migration::TenantTable` (`USING (tenant_id = current_setting('autoxide.tenant', true)::uuid OR
   current_setting('autoxide.all_tenants', true) = 'on')`, `FORCE ROW LEVEL SECURITY`); `TenantDb` mở mỗi đơn vị việc trong
   một transaction và đặt `set_config('autoxide.tenant', …, true)` (chỉ sống trong transaction, không rò sang kết nối khác
   của pool); `all_tenants` và migration đặt `autoxide.all_tenants`. SQLite không có RLS: nhánh đó là "không làm gì", giấu sau
   trait `RowSecurity` của kernel, test cả hai nhánh (luật số một: cam kết cách ly đến từ lớp 1, chạy trên cả hai; lớp 2 chỉ
   chặn thêm câu SQL viết tay sai). Chi phí đo: một `BEGIN … COMMIT` thêm cho mỗi lượt đọc gom.
3. **Lint `tenant-scope`** giữ tới khi module cuối cùng chuyển sang `db`, rồi gỡ.

Kèm theo (mục 71-G1): hạn mức theo khách của request **chưa xác thực** chỉ lấy khách từ thứ đã kiểm (tên miền đã xác minh,
`Site` đã phân giải), không từ gợi ý do client gửi (header, tham số); request không xác định được khách tính vào hạn mức
chung theo IP. Test: gửi gợi ý khách giả không làm tiêu hạn mức của khách đó.

## 8. Hợp nhất cài đặt theo khách

Hai hệ hiện có: `TenantSetting` (kernel, giá trị ở `kernel_tenant_settings`, có cache, có secret) và `OrgSetting` (catalog ở
kernel, giá trị ở bảng của `organizations`, kế thừa theo cây). Đích: **một** hệ ở kernel.

- `Setting` một kiểu, có phạm vi `SettingScope::Tenant` hoặc `SettingScope::Inherited` (đặt được ở mọi nút, nút dưới kế
  thừa). Giá trị ở `kernel_tenant_settings` thêm cột `org_id` (NULL = mức khách = nút gốc). Đọc: nút gần nhất trên chuỗi tổ
  tiên → mức khách → mặc định của cài đặt → cấu hình chung (biến môi trường). Chuỗi tổ tiên lấy qua `OrgTree` (trait sẵn có,
  `organizations` cung cấp), cache theo khách như hiện nay.
- Kiểm chéo lúc lưu: checker của cài đặt + phạm vi (cài đặt `Tenant` không đặt ở nút con) + quyền (`kernel.settings.manage`
  hoặc `organizations.settings.write` tại nút).
- Chuyển dữ liệu: `organizations` có `DataUpgrade` đọc bảng của chính nó và ghi qua API cài đặt của kernel (không module nào
  đọc bảng của module khác); endpoint cũ của `organizations` thành lớp mỏng gọi kernel, rồi `#[deprecated]` theo D-059.
- `Module::tenant_settings` + `Module::org_settings` thành một: `manifest.setting(Setting::…)`.

## 9. Luật gate mới

| Luật                                 | Kiểm gì                                                                                    | Nơi chạy                         |
| ------------------------------------ | ------------------------------------------------------------------------------------------ | -------------------------------- |
| `wrapped-libraries`                  | Module khai thư viện trong `xtask/wrapped-libraries.txt` → đỏ, trừ module trong baseline đang giảm | `xtask check`, `check-module`    |
| `private-paths`                      | Mã viết tay chứa `__private::`                                                             | `xtask check`, `check-module`    |
| `stable-signatures`                  | rustdoc JSON của kernel/testkit (feature mặc định): item công khai có kiểu từ thư viện bọc | `xtask check` (job riêng ở CI nếu chậm) |
| `exhaustive`                         | enum/struct công khai mới thiếu `#[non_exhaustive]` hay không có trong danh sách cho phép  | `xtask check`                    |
| `deprecated-note`                    | `#[deprecated]` thiếu `note` nêu cái thay thế                                              | `xtask check`                    |
| `module-contract-legacy`             | Module còn cài hàm cũ của `Module` (baseline chỉ giảm)                                     | `xtask check`                    |
| `migration-sql-snapshot`             | SQL sinh từ migration khớp file chụp                                                       | test của `xtask` / regen --check |

Mỗi vi phạm in một dòng "cách sửa" (mục 71-G6).

## 10. Thứ tự PR

Mỗi dòng một PR, mỗi PR có test trên cả hai database cho phần chạm database và qua kiểm độc lập (D-20261126-independent-review).

1. Tài liệu: `COMPATIBILITY.md` + `MODULE_GUIDE.md` (EN + VI) theo D-20261126-library-isolation và ba hạng; gỡ
   `legacy_default`.
2. `autoxide-macros` + `db` (Record, find/insert/update/delete, `TenantDb`/`Db`/`AllTenants`, lỗi `conflict()`), test cách ly
   hai database; chuyển `notes` làm mẫu; luật `wrapped-libraries` với baseline 38 module.
3. Migration `Plan` + file chụp SQL + `notes` chuyển migration.
4. HTTP: `endpoint`, `routes!`, `Schema`, `Params`, `Validate`, extractor, `NoContent`/`Download`; `time::Duration…`;
   `async_trait` xuất lại; `notes` chuyển hết → `notes` không còn khai thư viện bọc nào. Đo thời gian biên dịch (§4.2).
5. `Manifest` + `StartContext` gọn; `notes` chuyển; luật `module-contract-legacy`.
5b. Đánh phiên bản event / webhook / `x-*` (§13): `EventInfo::payload`, `docs/events.json`, danh mục phần mở rộng,
   `xtask contract-diff`.
6. Ba hạng: feature `unstable`, đường `unstable::`, `bench_seed` sang testkit; semver-checks chạy feature mặc định; luật
   `exhaustive`, `deprecated-note`, `stable-signatures` (baseline).
7–12. Chuyển 38 module còn lại theo cụm phụ thuộc (đáy trước): nền (`auth`, `organizations`, `plans`, `files`, `audit`,
   `webhooks`, `idempotency`, `mail`, `ai`) → liên lạc (`notifications`, `media`, `content`, `forms`, `newsletter`, `comments`,
   `menus`, `address`) → tiền (`ledger`, `tax`, `invoices`, `compliance`, `reporting`) → thương mại 1 (`catalog`, `inventory`,
   `promotions`, `shipping`, `customers`) → thương mại 2 (`orders`, `support`, `bookings`, `conversations`, `analytics`,
   `automations`) → bộ nối (`sepay`, `momo`, `ghn`, `esms`, `einvoice_sepay`). Mỗi cụm một PR (lớn thì chia đôi); file chụp SQL
   không đổi.
13. Cách ly: gỡ lint `tenant-scope`; RLS tuỳ chọn trên Postgres; hạn mức của request chưa xác thực.
14. Hợp nhất cài đặt theo khách.
15. Tách crate (đo trước, §6) + tách file lớn.
16. Gỡ mọi đường cũ `#[deprecated]` (thư viện bọc ra khỏi chữ ký, baseline rỗng), cập nhật `examples/external-module`,
   `new-module` template, `MODULE_GUIDE`.

PR 2–5 nối tiếp nhau (cùng chạm kernel); 7–12 song song được sau PR 5 (mỗi cụm một worktree); 13–15 song song với 7–12 khi
không chạm cùng file.

## 11. Rủi ro và cách giữ

- **Khối lượng**: ~6.000 chỗ gọi đổi. Giữ: API mới trùng từ vựng của SeaORM/utoipa khi được (`find`, `filter`, `one`,
  `#[schema(…)]`), nên phần lớn là đổi `use` và tên; file chụp SQL + test sẵn có của module bắt sai.
- **Lớp `db` thiếu một mẫu truy vấn**: thêm vào `db` (ổn định) hay tạm qua `db.sql` có lý do; không mở lối gọi thẳng SeaORM.
- **Thời gian biên dịch** của bản sao bóng: đo ở PR 4, vượt ngưỡng thì chuyển sang sinh thẳng bằng builder (§4.2).
- **Đụng phiên "Nâng cấp một nút"** (lệnh migrate, CI nâng cấp): PR 3 chạm bộ chạy migration — báo phiên điều phối trước khi
  sửa `migration.rs`/`manager.rs`.
- **Kiểu SeaORM rò qua `api` của module** (trait giữa module có tham số `&DatabaseTransaction` để ghi chung transaction):
  thay bằng `&TenantTx`/`&Tx` của kernel ở cụm chuyển tương ứng.

## 12. Câu hỏi cho người kiểm độc lập

1. Danh sách thư viện từ vựng (§1.1) có nên gồm `tokio` không, hay kernel bọc luôn phần đồng bộ (`Mutex`, `spawn`)?
2. Ngưỡng 5% thời gian `cargo check` cho bản sao bóng có hợp lý?
3. `payment`/`sms`/`push` ở hạng thử nghiệm có làm module thương mại "ổn định" bị kéo xuống không — hay module thương mại
   cũng vào hạng thử nghiệm lúc 1.0 (D-20261126-road-to-1-0 mục 4 gợi ý vậy)?
4. RLS mặc định `off` lúc đầu có đủ "hướng dài hạn"?

## 13. Đánh phiên bản hợp đồng ngoài REST (71-G13)

Hiện trạng 26/11/2026: event chỉ khai tên + mô tả (`EventInfo`), payload là `serde_json::Value` không có schema; webhook gửi
thân theo dạng Standard Webhooks (`type`, `timestamp`, `data{id, module, subject, actor, payload}`) không có số phiên bản;
OpenAPI có 9 phần mở rộng (`x-permissions` 1.150 chỗ, `x-autoxide-list`/`x-autoxide-filters` 296, `x-stability` 210,
`x-public` 175, `x-sunset`, `x-buyers`, `x-approval`, `x-idempotency`) không có danh mục; `api-diff` chỉ so path và schema
REST. Tên và schema tool MCP do phiên AI agent làm.

Quy tắc chung cho cả ba (giống REST, D-059): **thêm** trường tuỳ chọn / giá trị enum mới (enum đã khai là mở) / event mới /
phần mở rộng mới là không phá vỡ; **bỏ, đổi kiểu, đổi nghĩa, thêm trường bắt buộc** là phá vỡ → làm bản mới chạy song song,
bản cũ khai sắp bỏ kèm ngày sunset và cái thay thế, gỡ chỉ ở bản lớn (trước 1.0: theo D-20261125-api-break-before-1-0).

### 13.1 Event giữa module

- `EventInfo` thêm schema payload có kiểu: `EventInfo::new("notes.note_created", "…").payload::<NoteChange>()` với
  `NoteChange: Schema` (§4.2) — cùng derive với REST, nên một nguồn cho cả tài liệu lẫn kiểm. `Outbox::record` nhận
  `&impl EventPayload` (trait gắn kiểu payload với `EventInfo`), không nhận `Value` thô; bên nhận đọc bằng
  `delivered.payload_as::<NoteChange>()`.
- Bản mới của event có tên mới hậu tố `.v2` (`notes.note_created.v2`); module phát **cả hai** trong cùng transaction tới
  sunset; `EventInfo::deprecated(since, sunset, replaced_by)`. Bên nhận chọn bản bằng tên khi đăng ký, nên không ai bị đổi
  hình dạng dưới chân.
- Catalog sinh ra `docs/events.json` (tên, mô tả, JSON Schema payload, hạng, sunset) bằng `cargo xtask regen`;
  `xtask contract-diff` (mở rộng `api-diff`) so với bản phát hành gần nhất theo quy tắc trên và đỏ khi phá vỡ không qua đường
  `.v2`. Event của module hạng thử nghiệm được báo, không chặn.

### 13.2 Webhook gửi đi

- Thân giữ dạng Standard Webhooks, thêm header `webhook-version: 1` và trường `data.schema` = tên event có hậu tố bản
  (`notes.note_created` hoặc `….v2`). Đăng ký webhook chọn tên event cụ thể (gồm bản), nên webhook nhận bản nào là do người
  đăng ký quyết; khi bản cũ tới sunset, đăng ký đó nhận thông báo trước 30 ngày (event `webhooks.subscription_deprecated` vào
  hộp việc) rồi tự chuyển tắt.
- Khung thân (`type`, `timestamp`, `data.{id, module, subject, actor}`) là một hợp đồng riêng, bản `1`; đổi phá vỡ khung =
  `webhook-version: 2`, đăng ký chọn bản khung, bản cũ chạy song song tới bản lớn.
- OpenAPI khai payload gửi đi ở mục `webhooks` (71-G7) sinh từ catalog event, nên SDK có kiểu cho bên nhận.

### 13.3 Phần mở rộng `x-*` của OpenAPI

- Danh mục ở kernel (`http::extensions`): tên, JSON Schema của giá trị, hạng, mô tả; `#[endpoint]` chỉ sinh phần mở rộng có
  trong danh mục (lỗi biên dịch nếu không). Xuất ra `docs/openapi-extensions.json` và mô tả trong `info.x-autoxide-extensions`
  của OpenAPI để client tự khám phá.
- Phần mở rộng mới đặt tên `x-autoxide-<tên>`; 7 tên ngắn cũ (`x-permissions`, `x-public`, `x-stability`, `x-sunset`,
  `x-buyers`, `x-approval`, `x-idempotency`) giữ nguyên vì client đã đọc — đăng ký vào danh mục, không đổi tên.
- `contract-diff` so cả giá trị phần mở rộng: đổi `x-permissions` của một thao tác sang quyền khác là phá vỡ (client sinh
  giao diện theo nó); thêm quyền thay thế (danh sách "một trong") là không.

## 14. Đủ cho bước 54 và 52 (làm sau G1–G2, D-20261126-steps-52-54-before-1-0)

Bước 54/52 viết thẳng trên hợp đồng mới, nên hợp đồng phải có sẵn chỗ cắm. Đối chiếu từng mục còn mở:

| Mục còn mở                                                     | Chỗ cắm trong thiết kế này                                                    |
| -------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| 54 Quyền theo trường dữ liệu (cá nhân / nhạy cảm / tài chính, che khi thiếu `<module>.read_personal`, đọc có audit) | `#[derive(Schema)]` nhận `#[schema(data_class = personal | sensitive | financial)]` trên trường: sinh `x-autoxide-data-class` trong OpenAPI **và** cài trait `Redact` cho kiểu. Kernel bọc response (`Data`, `Page`, `Tagged`, kết quả tool MCP) bằng bước che theo `principal` trước khi trả: thiếu quyền đọc lớp đó thì trường thành `null` (kiểu đã là `Option` hoặc có giá trị che khai sẵn), có quyền thì ghi event audit "đọc dữ liệu nhạy cảm" (gom theo request). Vì che nằm ở lớp response của kernel, REST và MCP giống nhau, module không viết tay |
| 54 Chia sẻ bản ghi có hạn (`viewer`/`editor`, không trao quyền mình không có), API key chọn bản ghi | `manifest.relation(Relation::new("notes.note", ["viewer", "editor"]))`; `principal.can::<Note>(&NOTES_READ)` trả `Cond<Note>` của lớp `db` gồm phạm vi nút **hoặc** `EXISTS` trên `kernel_shares` (bảng kernel, cùng database) theo người / nhóm / nút / key — thay `scope_condition` trả `Condition` của sea-query |
| 54 Nâng quyền đúng lúc, tài khoản khẩn cấp, lệnh hai người, cặp quyền xung đột (ngưỡng là dữ liệu) | Khai trong manifest: `manifest.conflicting(PERM_A, PERM_B)`, `manifest.two_person(&PERMISSION)`; `#[endpoint(approval = "…")]` sinh `x-approval` và bọc handler bằng bước duyệt của kernel (REST và MCP như nhau) |
| 54 Giải thích, mô phỏng, "ai có quyền trên bản ghi này", quyền không dùng 90 ngày | Manifest là dữ liệu: kernel có đủ permission, quan hệ, xung đột của mọi module mà không cần khởi động; số lần dùng permission đo ở bước kiểm `require` |
| 54 AuthZEN Authorization API 1.0, `on_behalf_of` cho agent | Loại tài nguyên = `manifest.relation`/`resource_type`; `on_behalf_of` là trường của `Principal` và `Actor`, đi vào khung event (§13.1) và webhook (§13.2) |
| 54 Nhân sự nền tảng xin vào dữ liệu khách, "xem như" chỉ đọc | `TenantDb` mở ở chế độ chỉ đọc khi `principal` là phiên "xem như": `begin_write` trả lỗi trước khi chạm database, áp cho mọi module mà không sửa module; `all_tenants` cấm trong phiên đó |
| 52 Thông báo theo loại event, lý do nhận, cấu hình kênh | Catalog event có schema (§13.1) cho giao diện liệt kê và lọc loại event |
| 52 Trợ lý AI chạy tool MCP với quyền người gọi, ghi là nháp / yêu cầu duyệt | Tool đăng ký trong `start` cùng service với REST (luật 8); `approval` ở `#[endpoint]` và tool dùng chung một bước duyệt |
| 52 Sức khoẻ người bán / tuân thủ, ngưỡng có ngày hiệu lực | Cài đặt hợp nhất (§8) cho ngưỡng theo khách/nút; ngưỡng theo luật là dữ liệu có ngày hiệu lực ở module, không ở hợp đồng |
| 52 Event theo bản ghi qua tool MCP, điều hướng `mobile`, ghim | Đã có hình dạng; chuyển sang `manifest.navigation(…)`, không đổi nghĩa |

Hệ quả cho thứ tự PR: phần `data_class` của `#[derive(Schema)]` và `Cond<R>` có chia sẻ là **API ổn định từ PR 2 và 4** (để
54 không phải đổi chữ ký), còn hành vi (che, audit, bảng `kernel_shares`) do bước 54 làm. §13 thành PR 5b (sau manifest, vì
`EventInfo` khai trong manifest).

## Quyết định đi kèm

- D-20261126-contract-tiers — ba hạng, cơ chế, danh sách xuất phát
- D-20261126-kernel-data-layer — `db`, `Record`, handle theo khách, SQL viết tay, migration `Plan`
- D-20261126-migration-rewrite-proof — ngoại lệ có điều kiện cho luật "migration đã commit thì không sửa"
- D-20261126-kernel-web-layer — `endpoint`, `routes!`, `Schema`, `Validate`, extractor, `autoxide-macros`
- D-20261126-module-manifest — `Module` bốn hàm, `Manifest`, `StartContext` gọn
- D-20261126-kernel-crates — tách crate theo lớp, `autoxide-host`
- D-20261126-tenant-isolation — ba lớp cách ly, RLS tuỳ chọn, hạn mức request chưa xác thực
- D-20261126-one-settings-system — hợp nhất cài đặt theo khách
- D-20261126-contract-versioning — đánh phiên bản event, webhook, phần mở rộng `x-*` (§13)
