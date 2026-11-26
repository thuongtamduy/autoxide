# Hợp đồng kernel cho 1.0: hai hạng, lớp thay thư viện, một cách khai module, tách crate, cách ly khách

Thiết kế cho bước 71 nhóm G1 và G2 (D-20261126-road-to-1-0, D-20261126-library-isolation). Tài liệu nội bộ: các quyết định
ngắn gọn nằm ở `docs/decisions/D-20261126-*.md` (liệt kê ở cuối), còn đây là lý lẽ, API mẫu và kế hoạch chuyển. Viết
ngày 26/11/2026, trước khi có dòng code nào; số liệu đo trên `main` cùng ngày (commit `af74d2fb5`). Bản này đã sửa theo
kết quả kiểm độc lập lần 1 (duyệt kèm sửa, 13 điểm) và kết quả ba spike ở §15.

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
  (`Value`), `uuid` 1, `tokio` 1, `tracing` 0.1 (chỉ macro ghi log), `futures-core` (trait `Stream`), `bytes` 1. Thêm vào
  danh sách là một quyết định. Bản lớn mới của một thư viện từ vựng (rất hiếm: `serde` 2) mới là thay đổi lớn của Autoxide.
  Riêng `tokio`: module được khai và dùng (`Mutex`, `sleep`, `select!`), nhưng **chữ ký ổn định của kernel không có kiểu
  của tokio**; việc nền của module chạy qua `autoxide_kernel::task::spawn` (mang ngữ cảnh khách và span của tracing) chứ
  không qua `tokio::spawn`.
- `futures-util` (6 module): chỉ dùng tiện ích luồng (`StreamExt`, `join_all`) → thư viện riêng của module (nhóm 3), không
  lọt ra `api`. `tower` (dùng trong test của module) → chỉ ở `[dev-dependencies]`, testkit có hàm gửi request thay nó.
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
| Ổn định | `error`, `i18n`, `config`, `response`, `http` (phần §4), `module` (phần §5), `event`, `job`, `mcp`, `access`, `db` (mới, §3; đường ổn định ngay, cam kết từ 1.0 khi PR 16 xong — xem dưới), `migration` (bản mới, §3.6), `pagination`, `tenancy` (tập con: `TenantContext`, `TenantId`, `OrgId`, `OrgTree`, `TenantSeed`, `Relocation`, `settings`), `time`, `money`, `decimal`, `email`, `slug`, `public_url`, `csv`, `template`, `secret`, `idempotency`, `entitlement`, `privacy`, `version`, `reference`, `rows`, `numbering`, `upload`, `search`, `task`, `doctor`, `throttle`, `antispam`, `ip_hash`, `net`, `tokens`, `storage`, `mail`, `ai`, `identity`, `accounts`, `resource`, `provider` |
| Thử nghiệm | `einvoice`, `zns`, `voice`, `sms`, `push`, `payment`, `blocks`, `analytics`, `report`, `automation`, `custom_fields`, `sync`, `moderation`, `store`, `find`, `suggest`, `summary`, `work`, `usages`, `texts`, `reasons`, `tags`, `bulk`, `list_exports`, `connections`, `resolve`, `tenancy::{sandbox, distribution, moves, domains, site, link_token}` |
| Nội bộ (sang `autoxide-host`) | `backup`, `transfer`, `logging`, `metrics`, `telemetry`, `tls`, `setup`, `builtin`, `module_catalog`, `user_flags`, phần chạy của `migration` (`Migrator`, khoá), `Database::connect`, các route của kernel |
| Sang testkit | `bench_seed` |

`payment`, `sms`, `push` vào thử nghiệm vì đó là hợp đồng của bộ nối nhà cung cấp (D-20261126-road-to-1-0 mục 4); `mail`,
`storage`, `ai` ổn định vì đã có ≥ 2 cài đặt và được 7–8 module dùng. `custom_fields`, `automation` thử nghiệm tới khi G5
chốt hình dạng chính thức. Danh sách này là **điểm xuất phát**: lên hạng là một quyết định mới.

**`db` và `migration` (bản mới)**: nằm ngay ở đường ổn định `autoxide_kernel::db` / `autoxide_kernel::migration`, không qua
`unstable::` (nâng hạng sau này sẽ là dời đường dẫn, tức phá vỡ). Trước 1.0 chúng còn đổi được như mọi thứ ở bản `0.x`; cam
kết SemVer bắt đầu từ 1.0, và 1.0 chỉ phát hành khi PR 16 xong (mọi module đã chuyển, mẫu truy vấn thiếu đã lộ ra hết).

Hạng của **module** (`Stability` của manifest) tách khỏi hạng của **kernel**: module ổn định được dùng phần thử nghiệm
của kernel bên trong, nhưng `api` của nó không được lộ kiểu thử nghiệm (gate `stable-signatures` kiểm cả `api` của module
ổn định). Lúc ra 1.0 các module thương mại (bước 41–49, 59–65) và bộ nối nhà cung cấp mang nhãn thử nghiệm.

Đề xuất hạng module lúc 1.0 (phiên điều phối quyết thay chủ dự án ngày 26/11/2026 cho nhóm thương mại; phần còn lại là đề
xuất, chốt bằng quyết định riêng ở PR 6). Tiêu chí "Becoming official" của `docs/MODULES.md`: không còn mục mở trong lộ trình,
có người dùng thật, API không phải đổi qua vài bản liên tiếp, bộ nối đã chạy thử với tài khoản thật.

| Hạng lúc 1.0 | Module | Lý do |
| --- | --- | --- |
| core (giữ) | `mail`, `auth`, `idempotency` | nền của mọi cài đặt |
| official (giữ) | `ai`, `files`, `address`, `organizations`, `audit`, `webhooks`, `notifications`, `plans`, `media`, `content`, `forms`, `menus` | hạ tầng chung hay nội dung, API đã ổn qua nhiều bản, không phụ thuộc luật chuyên ngành hay đổi |
| official → **experimental** | `catalog`, `inventory`, `promotions`, `shipping`, `orders`, `customers` | đã quyết (phiên điều phối, 26/11/2026): chưa có người dùng thật |
| official → **experimental** (đã quyết, kiểm lần 2) | `tax`, `ledger`, `invoices` | chưa có người dùng thật; còn chờ luật sư rà (71-G12) — quy định thuế, hoá đơn, kế toán đổi theo văn bản; lên hạng khi rà xong và có người dùng |
| official (giữ, đã quyết) | `compliance` | giữ quy trình yêu cầu dữ liệu cá nhân và báo sự cố trong 72 giờ — nghĩa vụ theo luật số hai với mọi bản cài; `plans`, `content`, `notifications` dùng nó. Thao tác nào còn chưa chắc về pháp lý gắn `x-stability: experimental` theo từng operation |
| experimental (giữ) | `sepay`, `momo`, `einvoice_sepay`, `esms`, `ghn`, `bookings`, `newsletter`, `comments`, `support`, `conversations`, `analytics`, `reporting`, `automations` | như hiện nay |

Hai hệ quả phải làm cùng PR chuyển hạng (PR 6):

- `plans` (official) đang dùng API của `invoices`/`ledger` (experimental): `plans` **giảm tính năng rõ ràng** khi hai module đó
  tắt — thao tác cần hoá đơn/sổ trả `plans.invoicing_unavailable` có hướng dẫn bật, OpenAPI ghi rõ phụ thuộc — kèm ghi chú
  Breaking; cấu hình mẫu (`compose.yml`, `.env.example`, `autoxide demo`) bật sẵn `tax,ledger,invoices` để người mới không mất
  tính năng.
- **Không lặng lẽ tắt module đang có dữ liệu**: lúc khởi động và trong `doctor`, module có migration đã chạy (bảng lịch sử
  có dòng) mà không còn được chọn (vd vì nâng lên 1.0 đổi hạng sang experimental, không bật mặc định) thì báo **lỗi** kèm đúng
  dòng `AUTOXIDE_MODULES=…` cần thêm; muốn tắt thật thì khai rõ (`-<module>` hay chế độ "nghỉ" D-20261125-dormant-modules).

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
#[record(table = "notes_note", scope = tenant)] // bắt buộc: `scope = tenant` hoặc `scope = global`
pub(crate) struct Note {
    #[record(primary_key)]
    pub id: Uuid,
    pub tenant_id: TenantKey,  // kiểu riêng: đọc và so được, không `set` được
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
trị)`. Thuộc tính `scope` **bắt buộc**: thiếu là lỗi biên dịch, và `scope = global` trên struct có trường `tenant_id` cũng
là lỗi biên dịch (không có đường "quên khai thành global"). Lúc khởi động kernel đối chiếu mọi `Record` với danh sách
`TenantTable` của migration: bảng có trong danh sách mà record khai `global` (hay ngược lại) thì không khởi động. Cột
`tenant_id` có kiểu `TenantKey` không cài `Settable`, nên `update().set(Note::TENANT_ID, …)` không biên dịch. Kiểu trường được nhận: `bool`, `i16`/`i32`/`i64`, `String`, `Uuid`, `Timestamp`, `Date`, `serde_json::Value`,
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
- Các mẫu mà bản đầu thiếu (kiểm lần 1, điểm 4), nay có trong `db` và đã chạy trong spike (a): `for_share()` (thay
  `rows::lock_one_shared`), `update().adjust(cột, delta, Bounds)` (thay `rows::adjust`, chặn tràn `i64` trên cả hai
  database), `tx.lock_key("…")` (thay `rows::lock_key`, tên tự gắn khách), `sub(cột).filter(…)` + `cột.in_sub(…)` (thay
  `Query::select()` tự dựng — 51 file module — mà nay tự lọc khách), `Cond::any()/all()` dựng động (bộ lọc thuộc tính của
  catalog), `sum`/`count`/`max` luôn ép `BIGINT` qua `COALESCE` (Postgres trả `numeric` cho `SUM(bigint)`).
- Ghi chung transaction với kernel: `Outbox::record(&TenantTx, …)`, `SyncLog::changed(&TenantTx, …)`,
  `principal.require_write_at(&TenantTx, …)` — ~125 chữ ký kernel đang generic theo `ConnectionTrait` đổi sang nhận
  `&impl Handle` (trait niêm phong của `db`), giữ bản cũ `#[deprecated]` trong lúc chuyển.
- Đọc và ghi tách kết nối như luật dữ liệu: `read()` đi pool đọc (read replica nếu có), `begin_write()` đi pool ghi; handle
  đọc không có hàm ghi nào (lỗi biên dịch, không phải lỗi lúc chạy).
- `for_update()`: Postgres `FOR UPDATE`; SQLite bỏ qua vì transaction ghi của SQLite đã giữ khoá cả database (một bên ghi).
  Viết ở một chỗ, có test đồng thời trên cả hai.
- **Luật kiến trúc 1 kiểm lúc biên dịch** (kiểm lần 2, F2): `#[derive(Record)]` đọc tên module từ `CARGO_PKG_NAME` (bỏ tiền
  tố `autoxide-`, `-` thành `_`: `autoxide-einvoice-sepay` → `einvoice_sepay`) và đòi `table` bắt đầu bằng `<module>_` —
  khai `Record` cho bảng của module khác là lỗi biên dịch, nên `find`/`sub`/`in_sub`/`join` trên bảng lạ không thể viết.
  Tiền tố so **trọn tên module + `_`**, không tách ở dấu `_` đầu tiên (spike tách sai `einvoice_sepay_*` thành `einvoice`).
  Kernel còn kiểm lúc khởi động: mọi `Record` một module khai có bảng nằm trong danh sách bảng của migration của chính module
  đó. Crate ngoài repo (tên crate tuỳ ý, không theo mẫu `autoxide-<tên>`) khai tên module ở `Cargo.toml`:
  `[package.metadata.autoxide] module = "…"`; derive đọc file đó qua `CARGO_MANIFEST_DIR`, thiếu thì lỗi biên dịch nêu cách khai.
  Không có thuộc tính `module = …` trên từng `Record` (một chỗ khai, không thể khai lệch giữa các record).
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

Đóng các lối vượt (kiểm lần 1, điểm 3):

- `Db`, `AllTenants` và `Db::sql` **không** lấy được từ `TenantContext` hay `Database` công khai: chỉ có qua
  `context.kernel::<ControlPlane>()` trong `start` (module control-plane như `auth`, `plans`) — handle đó giữ tên module để
  ghi metric. `TenantContext::database()` (trả `&Database` thô) thành `#[deprecated]` rồi gỡ; gate `raw-database` chặn module
  gọi nó trong lúc chuyển.
- `on_conflict().do_update(…)` trên `TenantDb`: kernel thêm `WHERE <bảng>.tenant_id = <khách>` vào nhánh `DO UPDATE` (Postgres
  và SQLite đều hỗ trợ `ON CONFLICT … DO UPDATE … WHERE`); đụng dòng của khách khác thì 0 dòng đổi và kernel trả lỗi
  `db.unique_conflict` — không ghi đè. Đã chạy trong spike (a) với khoá unique không có `tenant_id`.
- `join` và `sub`/`exists` lọc khách trên **mọi** record cấp khách tham gia (bảng được nối thêm điều kiện ở `ON`).
- `insert` từ chối dòng có `tenant_id` khác khách của handle trước khi gửi câu.

### 3.4 SQL viết tay (phần ngoài lề còn lại)

`db.sql("SELECT … WHERE tenant_id = :tenant AND …").bind("since", since).all::<(A, B)>()` (tuple hay `#[derive(FromRow)]`).
Câu chạy trên `TenantDb` **phải** có tham số `:tenant` (kernel gắn giá trị); thiếu thì trả lỗi trước khi gửi câu. Đây chỉ là
**rào phụ**, không phải cam kết cách ly của lớp 1: kiểm dò chuỗi không thấy `OR 1=1` hay `:tenant` nằm trong comment. Vì vậy
số chỗ gọi `db.sql` của mỗi module đếm theo baseline chỉ được giảm (`xtask/raw-sql-baseline.txt`), mỗi chỗ có lý do đo được
và test cách ly khách riêng. Câu viết tay vẫn phải chạy trên cả hai database (luật dữ liệu). Thay `autoxide_kernel::database::statement`
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
- **Viết lại migration đã commit** (để module bỏ được `sea-query`) — D-20261126-migration-rewrite-proof, sửa theo kiểm lần 1
  (điểm 2). Rà 26/11: ~64/236 file migration có bước dữ liệu hay code riêng từng database (vd `auth` `m0019_move_tokens` đọc
  từng dòng và ghi `Timestamp::now()`), helper dùng chung (`event::outbox_migration`, `tokens_migration`…) và `down` gọi `up`
  của migration cũ hơn. Vì vậy:
  - **Cái kiểm lâu dài là schema, không phải câu SQL**: test `migration-schema` chạy, trên cả hai database, `up` của từng
    migration rồi `up → down → up`, và sau mỗi bước chụp **catalog schema đã chuẩn hoá** (bảng, cột, kiểu, NULL, mặc định,
    index, ràng buộc — đọc từ `information_schema`/`pg_catalog` và `sqlite_schema`/`pragma`, sắp xếp, bỏ tên tự sinh). File
    chụp `xtask/migration-schema/<module>.<database>.txt`. Viết lại một migration không được đổi file chụp. Renderer của `Plan`
    vì vậy không bị trói vào từng byte đầu ra của sea-query.
  - So từng câu lệnh (byte hay dạng câu) chỉ là công cụ **lúc viết lại** để soát nhanh, không là test lâu dài.
  - Migration chỉ có DDL: cách rẻ là sinh máy móc `plan.sql_for(Backend::Sqlite, "…")` / `plan.sql_for(Backend::Postgres, "…")`
    từ câu đã chụp — đóng băng đúng câu cũ, không cần dịch sang builder.
  - Bước dữ liệu giữ nguyên code Rust thành `DataStep` (trait async của kernel, nhận handle `Db` không kiểu); có test dữ liệu
    mẫu: nạp dữ liệu trước migration, chạy, so dữ liệu sau — trên cả hai database.
  - Helper dùng chung của kernel (`outbox_migration`, `tokens_migration`…) chuyển một lần ở kernel, giữ tên migration họ sinh.
  - `down` gọi `up` của migration cũ hơn: chuyển cả cặp trong cùng PR.
  - Tên migration không bao giờ đổi (khoá của bảng lịch sử và của manifest migration theo bản phát hành của G3).
- Thứ tự với G3: PR `Plan` đi sau các PR của G3 chạm bộ chạy migration; "luật an toàn migration" của G3 viết trên `Plan`
  khi `Plan` đã có (trước đó G3 dùng bản đọc text tạm, rồi chuyển).

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

Bản sao bóng chép **mọi** thuộc tính utoipa đọc: doc comment (`#[doc]`), `#[serde(…)]` (43 `flatten`), `#[schema(…)]` (321
`value_type`, 53 `inline`…), `#[deprecated]`, `#[cfg]`; bỏ mọi thuộc tính khác; trường thành riêng tư. Không có `as` thì gắn
`#[schema(as = <tên thật>)]` để tên schema không đổi. `ToSchema::name()` **và** `schemas()` (schema lồng) chuyển tiếp sang bản
sao; kiểu generic giữ nguyên tham số và thêm ràng buộc `Bóng<T>: ToSchema`. Spike (b) đã chạy trên toàn bộ DTO của module
khó nhất (§15).

Ba chỗ module đang chạm utoipa/axum ngoài derive (kiểm lần 1, điểm 5):

- `forms` dựng schema bằng builder lúc chạy (`inbox.rs`: schema của câu trả lời phụ thuộc biểu mẫu) → kernel thêm
  `http::schema::JsonSchema` (giá trị JSON Schema của kernel, dựng bằng `serde_json::json!`) và `Endpoint` nhận schema động
  dạng đó; kernel tự đổi sang kiểu của utoipa.
- `auth` dùng `axum::middleware::map_response` (thêm header vào response) → kernel thêm `Module`-level `response_hook` trong
  `start` (`context.response_hook(fn(&RequestInfo, &mut ResponseHeaders))`), không lộ kiểu axum.
- `bytes` 1 → thư viện từ vựng (§1.1); `futures-util`, `tower` → như §1.1.

Luật khi nâng utoipa (hay thay bộ sinh): `docs/openapi.json` **không đổi từng byte**; đổi là thay đổi hợp đồng, đi qua
`contract-diff` như mọi thay đổi REST.

Impl trait của thư viện bọc **trên kiểu ổn định** (vd `impl utoipa::ToSchema for Page<T>`, `impl IntoResponse for Error`)
không giấu được trong Rust: khai rõ trong `COMPATIBILITY.md` là **ngoài hợp đồng** (có thể biến mất khi kernel đổi thư viện),
và gate `stable-signatures` bỏ qua impl trait của thư viện trong danh sách bọc, chỉ kiểm chữ ký hàm, trường, kiểu trả về.

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

### 5.1 Ánh xạ đủ 49 hàm của trait cũ

49 hàm = 45 hàm khai báo + `name`/`configure`/`start`/`dormant` (đã nằm trong bảng). Tiêu chí: giá trị là **dữ liệu** (tên, danh sách, hằng, con trỏ hàm thuần) → `Manifest`; giá trị **giữ `Arc<dyn …>`** cần
service hay cấu hình đã đọc → `start` (kiểm lần 1, điểm 6).

| Hàm cũ của `Module` | Đường mới |
| --- | --- |
| `name`, `description` | `Manifest::new(name, description)` |
| `stability`, `essential`, `always_enabled`, `depends_on` | `manifest.stability(…)`, `.essential()`, `.always_enabled()`, `.depends_on([…])` |
| `migrations`, `upgrades` | `manifest.migrations(…)`, `.upgrades(…)` (bước dữ liệu là đối tượng thuần, nhận ngữ cảnh lúc chạy) |
| `tenant_settings`, `org_settings` | `manifest.setting(Setting::…)` (một hệ, §8) |
| `retained_tables`, `retention`, `control_plane_tables` | `manifest.retained_tables(…)`, `.retention(…)`, `.control_plane_tables(…)` |
| `features`, `limits` | `manifest.feature(…)`, `.limit(…)` |
| `permissions`, `events`, `resource_events` | `manifest.permissions(…)`, `.events(…)`, `.resource_events(…)` |
| `schedules`, `deprecations`, `body_limits` | `manifest.schedule(…)`, `.deprecation(…)`, `.body_limit(…)` |
| `translations`, `navigation`, `reason_lists`, `overridable_texts` | `manifest.translations(…)`, `.navigation(…)`, `.reason_list(…)`, `.overridable_text(…)` |
| `relocation` | bảng và cột niêm phong: `manifest.relocation(…)`; `switch_hooks` (`Arc<dyn SwitchHook>`): `context.contribute` trong `start` |
| `custom_field_targets`, `facts`, `analytics_events`, `funnels`, `automation_templates`, `promotable`, `sync_resources`, `blocks` | `manifest.unstable().…` (feature `unstable`) |
| `subscriptions`, `routes`, `tools` | `start`: `context.subscribe(…)`, `.routes(…)`, `.tools(…)` |
| `jobs`, `tasks`, `checks` | `start`: `context.job(kind, handler)`, `.task(…)`, `.check(…)` |
| `prompts`, `mcp_resources` | `start`: `context.prompt(…)`, `.mcp_resource(…)` |
| `client_config` | `start`: `context.client_config(section)` (đọc cấu hình đã `configure`) |
| `seed_tenant`, `sample_data` | `start`: `context.contribute::<dyn TenantSeed>(…)`, `::<dyn SampleData>(…)` |
| `sandbox_copy` | `start`: `context.contribute::<dyn SandboxCopy>(…)` (thử nghiệm) |
| `configure`, `start`, `dormant` | giữ (3 trong 4 hàm của trait mới) |

`schedules` ở manifest vì chỉ là (tên việc, lịch); handler của việc đăng ký ở `start` cùng tên — kernel kiểm khớp lúc khởi
động.

### 5.2 Đường chuyển

1. Thêm `manifest()` có **cài đặt mặc định** dựng `Manifest` từ các hàm cũ (gọi với `#[allow(deprecated)]`); `name()` đổi từ
   bắt buộc thành **có mặc định** trả `""` (`#[deprecated]`). Kernel chỉ đọc `manifest()`; tên rỗng → lỗi đăng ký "implement
   `Module::manifest`". Nhờ vậy module kiểu mới chỉ cài `manifest()` + `start()` — **không** phải cài hàm deprecated nào, và
   không có vòng gọi vô hạn giữa hai mặc định.
2. Các hàm cũ và các hàm đăng ký riêng của `StartContext`: `#[deprecated(note = "use `Module::manifest` …")]`. Rust không cảnh
   báo khi *cài* một hàm trait bị deprecated, nên `xtask check` đếm module còn cài hàm cũ theo
   `xtask/module-contract-legacy.txt` (chỉ được giảm).
3. Chuyển module theo cụm. Khi danh sách rỗng: gỡ hàm cũ; `manifest()` và `start()` thành bắt buộc (không mặc định) — trước
   1.0, thay đổi phá vỡ có ghi (D-20261125-api-break-before-1-0).
4. `examples/external-module` biên dịch và chạy test ở **mọi** PR của G1–G2 (CI chạy `./gate.sh --only example` khi PR chạm
   kernel, testkit hay `crates/app`), không chỉ ở PR cuối: nó là người dùng ngoài đầu tiên của hợp đồng.

## 6. (d) Tách kernel thành crate

**Đã quyết (chủ dự án giao phiên điều phối quyết, 26/11/2026, theo số đo §15.3):** trước 1.0 **chỉ tách `autoxide-host`**;
tách phần còn lại theo lớp dời **sau 1.0** vì chỉ rút 6–9% thời gian build lại (ngưỡng 20%). Lộ trình G1 đã sửa. Hai vướng
mắc đã thấy: orphan rule (`impl IntoResponse for Error`, `impl From<DbErr> for Error`, `impl ToSchema for Page/Data` phải nằm ở
crate có `Error`/`Page` hoặc crate của thư viện) và phụ thuộc ngược (`http`, `mcp` dùng `access`/`tenancy`; `ops/*/http.rs` dùng
`http`). Lợi ích cũng nhỏ hơn dự tính: module nào cũng phụ thuộc mặt tiền nên sửa một file kernel vẫn biên dịch lại mọi module.

Đích ban đầu (giữ để tham khảo; chỉ làm nếu §15.3 cho thấy đáng):

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

**Sửa sau kiểm lần 1 (điểm 8):** lớp 1 (chạy trên cả hai database) là cam kết của 1.0 và đủ cho mục lộ trình ("lớp kết nối
gắn khách **hay** RLS tuỳ chọn"); **RLS dời sau 1.0**. Khi làm RLS: `current_setting` rỗng coi như chưa đặt (chặn, không cho
qua); `doctor` báo khi role kết nối là superuser hay có `BYPASSRLS`; mọi worker của kernel quét nhiều khách tự đặt cờ bỏ qua;
policy dựng bằng bước đối chiếu idempotent lúc khởi động (theo danh sách `TenantTable`), không bằng migration phụ thuộc cấu
hình.

Các lớp (lớp 2 sau 1.0), lớp chính chạy trên cả hai database:

1. **Lớp kết nối gắn khách (chính, cả hai database)** — §3.3: record `tenant` chỉ truy vấn được qua `TenantDb`; quên khách là
   lỗi biên dịch; quét mọi khách phải gọi `all_tenants("lý do")`; SQL viết tay trên `TenantDb` bắt buộc có `:tenant` (rào phụ, §3.4);
   `insert` kiểm `tenant_id` khớp. Test: mỗi thao tác của `db` có test "khách A không thấy/không sửa được dòng của khách B"
   trên cả hai database; `testkit::leak_check` sẵn có chạy lại trên mọi module đã chuyển.
2. **Row-level security trên Postgres (phòng thủ thêm) — sau 1.0**; thiết kế chi tiết làm lúc đó, theo bốn điều kiện ở đầu
   mục này. SQLite không có RLS nên cam kết cách ly luôn đến từ lớp 1.
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
- Sửa sau kiểm lần 1 (điểm 7):
  - `organizations` tắt (không có `OrgTree`) mà khách có giá trị kế thừa ở nút: đọc **không** lặng lẽ rơi về mức khách —
    `doctor` báo, API đọc cài đặt trả kèm `inherited_unavailable: true`, và đặt giá trị ở nút con bị từ chối với mã
    `kernel.settings_tree_unavailable`.
  - `DataUpgrade` chuyển dữ liệu có `down` chạy được (chép ngược về bảng của `organizations`), để `autoxide rollback` của G3
    lùi qua bản này.
  - Secret kế thừa (khai `.secret()` trên cài đặt `Inherited`) niêm phong như secret của khách (`Sealer`, tên cài đặt + nút
    làm dữ liệu gắn kèm), không bao giờ trả ra API.

## 9. Luật gate mới

| Luật                                 | Kiểm gì                                                                                    | Nơi chạy                         |
| ------------------------------------ | ------------------------------------------------------------------------------------------ | -------------------------------- |
| `wrapped-libraries`                  | Module khai thư viện trong `xtask/wrapped-libraries.txt` → đỏ, trừ module trong baseline đang giảm | `xtask check`, `check-module`    |
| `private-paths`                      | Mã viết tay chứa `__private::`                                                             | `xtask check`, `check-module`    |
| `stable-signatures`                  | rustdoc JSON của kernel/testkit (feature mặc định): item công khai có kiểu từ thư viện bọc | `xtask check` (job riêng ở CI nếu chậm) |
| `exhaustive`                         | enum/struct công khai mới thiếu `#[non_exhaustive]` hay không có trong danh sách cho phép  | `xtask check`                    |
| `deprecated-note`                    | `#[deprecated]` thiếu `note` nêu cái thay thế                                              | `xtask check`                    |
| `module-contract-legacy`             | Module còn cài hàm cũ của `Module` (baseline chỉ giảm)                                     | `xtask check`                    |
| `migration-schema`                   | Catalog schema chuẩn hoá sau `up` và `up→down→up` của từng migration khớp file chụp, cả hai database | test, chạy trong CI         |
| `raw-database`                       | Module gọi `TenantContext::database()` / `Database::read()` thô (trong lúc chuyển)          | `xtask check`                    |
| `module-api-tier`                    | `api` của module ổn định lộ kiểu thử nghiệm của kernel                                     | `xtask check`                    |

Mỗi vi phạm in một dòng "cách sửa" (mục 71-G6).

## 10. Thứ tự PR

Mỗi dòng một PR, mỗi PR có test trên cả hai database cho phần chạm database và qua kiểm độc lập (D-20261126-independent-review).

Luật của đợt (kiểm lần 1, điểm 9): trong PR 2–5 **đóng băng** việc khác chạm kernel (phiên điều phối giữ); bước 54/52 đi sau
G1–G2. Mỗi PR chuyển một cụm module bỏ **hết** phụ thuộc thư viện bọc của mọi module trong cụm — không module nào nửa cũ nửa
mới; baseline của `wrapped-libraries` giảm đúng số module của cụm. Ba spike (§15) xong trước PR 2.

1. Tài liệu: `COMPATIBILITY.md` + `MODULE_GUIDE.md` (EN + VI) theo D-20261126-library-isolation và ba hạng; gỡ
   `legacy_default`.
2. `autoxide-macros` + `db` (Record, find/insert/update/delete, `TenantDb`/`Db`/`AllTenants`, lỗi `conflict()`), test cách ly
   hai database; chuyển `notes` làm mẫu; luật `wrapped-libraries` với baseline 38 module. Kèm (kiểm lần 2, F3): các ca của
   spike (a) thành test tích hợp chạy trong CI trên cả hai database; test lỗi biên dịch bằng `trybuild` ghim thông báo lỗi (thêm
   vào `STACK.md`) cho: record cấp khách qua handle control-plane, `set(TENANT_ID, …)`, thiếu `scope`, `scope = global` mà có
   `tenant_id`, `scope = tenant` mà thiếu `tenant_id: TenantKey`, `TENANT_ID` trong danh sách cập nhật của
   `on_conflict_update`, `Record` cho bảng của module khác (F2); bước đối chiếu lúc khởi động `Record` ↔ `TenantTable` ↔ bảng
   của migration của module; số chỗ `db.sql` theo baseline.
3. Migration `Plan` + test `migration-schema` (chụp catalog schema, §3.6) + `notes` chuyển migration. **Đi sau các PR của G3 (nâng cấp một nút) chạm
   `migration.rs`/`manager.rs`**: G3 dựa vào manifest tên migration theo từng bản phát hành (rollback) và lock của placement
   trong `kernel_migration_runs`, nên PR này giữ nguyên **tên** mọi migration, bảng lịch sử và đường chạy của G3; chỉ thêm
   cách khai mới bên cạnh.
4. HTTP: `endpoint`, `routes!`, `Schema`, `Params`, `Validate`, extractor, `NoContent`/`Download`; `time::Duration…`;
   `async_trait` xuất lại; `notes` chuyển hết → `notes` không còn khai thư viện bọc nào. Đo thời gian biên dịch (§4.2) ít nhất 3
   lần, lấy trung vị. Có test khẳng định rõ `openapi.json` sinh ra bằng đúng bản đã commit sau khi đổi derive (không chỉ dựa
   vào việc file sinh lại không đổi). **Trước PR 4** (kiểm lần 2, F9): spike `#[endpoint]` + `Params` trên một module trọn vẹn
   không còn khai `utoipa`/`utoipa-axum`, `openapi.json` giống từng byte — **đã làm, đạt** (§15.4).
5. `Manifest` + `StartContext` gọn; `notes` chuyển; luật `module-contract-legacy`.
5b. Đánh phiên bản event / webhook / `x-*` (§13): `EventInfo::payload`, `docs/events.json`, danh mục phần mở rộng,
   `xtask contract-diff`.
5c. Hợp đồng CLI và định dạng file (§16): `docs/cli.json`, `format_version`, file mẫu mỗi bản, mở rộng `contract-diff`.
6. Ba hạng: feature `unstable`, đường `unstable::`, `bench_seed` sang testkit; semver-checks chạy feature mặc định; luật
   `exhaustive`, `deprecated-note`, `stable-signatures` (baseline); chuyển hạng module (§2.1, quyết định riêng; sửa
   `docs/MODULES.md` + `.vi.md`).
6b. Công cụ `migrate-module` + hướng dẫn chuyển 0.x → 1.0 (§17); chạy thử trên `notes` và bản 0.x của `examples/external-module`.
7–12. Chuyển 38 module còn lại theo cụm phụ thuộc (đáy trước): nền (`auth`, `organizations`, `plans`, `files`, `audit`,
   `webhooks`, `idempotency`, `mail`, `ai`) → liên lạc (`notifications`, `media`, `content`, `forms`, `newsletter`, `comments`,
   `menus`, `address`) → tiền (`ledger`, `tax`, `invoices`, `compliance`, `reporting`) → thương mại 1 (`catalog`, `inventory`,
   `promotions`, `shipping`, `customers`) → thương mại 2 (`orders`, `support`, `bookings`, `conversations`, `analytics`,
   `automations`) → bộ nối (`sepay`, `momo`, `ghn`, `esms`, `einvoice_sepay`). Mỗi cụm một PR (lớn thì chia đôi); file chụp
   schema không đổi.
13. Cách ly: gỡ lint `tenant-scope` và `raw-database`; hạn mức của request chưa xác thực. (RLS: sau 1.0, §7.)
14. Hợp nhất cài đặt theo khách.
15. Tách `autoxide-host` + tách file lớn. Tách phần còn lại của kernel theo lớp: sau 1.0 (D-20261126-kernel-crates).
16. Gỡ mọi đường cũ `#[deprecated]` (thư viện bọc ra khỏi chữ ký, baseline rỗng), cập nhật `examples/external-module`,
   `new-module` template, `MODULE_GUIDE`.

PR 2–5 nối tiếp nhau (cùng chạm kernel); 7–12 song song được sau PR 5 (mỗi cụm một worktree); 13–15 song song với 7–12 khi
không chạm cùng file.

## 11. Rủi ro và cách giữ

- **Khối lượng**: ~6.000 chỗ gọi đổi. Giữ: API mới trùng từ vựng của SeaORM/utoipa khi được (`find`, `filter`, `one`,
  `#[schema(…)]`), nên phần lớn là đổi `use` và tên; file chụp SQL + test sẵn có của module bắt sai.
- **Lớp `db` thiếu một mẫu truy vấn**: thêm vào `db` (ổn định) hay tạm qua `db.sql` có lý do; không mở lối gọi thẳng SeaORM.
  `db` ở đường ổn định ngay từ đầu (§2.1); trước 1.0 còn đổi được như mọi thứ của `0.x`, cam kết SemVer từ 1.0 — và 1.0
  đòi PR 16 xong, nên mẫu thiếu lộ ra trong lúc chuyển 39 module vẫn sửa được trước khi khoá.
- **`OR EXISTS` (chia sẻ bản ghi) phá luật index**: đo query plan trên cả hai database ở bước 54 trước khi chốt; phương án dự
  phòng là `page()` trộn hai luồng keyset (theo phạm vi nút và theo chia sẻ), mỗi luồng dùng index riêng.
- **Thời gian biên dịch** của bản sao bóng: đo ở PR 4, vượt ngưỡng thì chuyển sang sinh thẳng bằng builder (§4.2).
- **Đụng G3 "Nâng cấp một nút"**: PR 3 chạm bộ chạy migration nên làm sau các PR của G3 ở `migration.rs`/`manager.rs`;
  tên migration là khoá của manifest rollback theo bản phát hành, không bao giờ đổi khi viết lại sang `Plan`.
- **Kiểu SeaORM rò qua `api` của module** (trait giữa module có tham số `&DatabaseTransaction` để ghi chung transaction):
  thay bằng `&TenantTx`/`&Tx` của kernel ở cụm chuyển tương ứng.

## 12. Câu hỏi đã có trả lời (kiểm lần 1)

1. `tokio`: giữ trong danh sách từ vựng; không để kiểu tokio trong chữ ký ổn định; kernel có `task::spawn` mang ngữ cảnh khách
   và tracing; không bọc `Mutex` (§1.1).
2. Ngưỡng 5% `cargo check`: giữ; đo thêm `check` gia tăng của một module lớn (`auth`, `catalog`); đường dự phòng builder sẵn.
3. Hạng module tách khỏi hạng kernel; module thương mại mang nhãn thử nghiệm lúc 1.0 (§2.1).
4. RLS: dời sau 1.0, kèm bốn điều kiện khi làm (§7).

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

- Thân giữ dạng Standard Webhooks, thêm trường `data.schema` = tên event có hậu tố bản (`notes.note_created` hoặc `….v2`) và
  `data.envelope_version` = `1`. **Nguồn đúng là thân đã ký** (chữ ký Standard Webhooks phủ thân, không phủ header); header
  `webhook-version: 1` chỉ là gợi ý tiện cho bộ định tuyến, bên nhận phải tin trường trong thân (kiểm lần 1, điểm 13). Đăng ký webhook chọn tên event cụ thể (gồm bản), nên webhook nhận bản nào là do người
  đăng ký quyết; khi bản cũ tới sunset, đăng ký đó nhận thông báo trước 30 ngày (event `webhooks.subscription_deprecated` vào
  hộp việc) rồi tự chuyển tắt.
- Khung thân (`type`, `timestamp`, `data.{id, module, subject, actor}`) là một hợp đồng riêng, bản `1`; đổi phá vỡ khung =
  `webhook-version: 2`, đăng ký chọn bản khung, bản cũ chạy song song tới bản lớn.
- OpenAPI khai payload gửi đi ở mục `webhooks` (71-G7) sinh từ catalog event, nên SDK có kiểu cho bên nhận.

### 13.3 Phần mở rộng `x-*` của OpenAPI

- Danh mục ở kernel (`http::extensions`): tên, JSON Schema của giá trị, hạng, mô tả. Kiểm bằng file sinh ra +
  `contract-diff` (phần mở rộng có trong OpenAPI mà không có trong danh mục thì đỏ) — không cần kiểm lúc biên dịch (kiểm lần 1,
  điểm 8). Xuất ra `docs/openapi-extensions.json` và mô tả trong `info.x-autoxide-extensions`
  của OpenAPI để client tự khám phá.
- Phần mở rộng mới đặt tên `x-autoxide-<tên>`; 7 tên ngắn cũ (`x-permissions`, `x-public`, `x-stability`, `x-sunset`,
  `x-buyers`, `x-approval`, `x-idempotency`) giữ nguyên vì client đã đọc — đăng ký vào danh mục, không đổi tên.
- `contract-diff` so cả giá trị phần mở rộng: đổi `x-permissions` của một thao tác sang quyền khác là phá vỡ (client sinh
  giao diện theo nó); thêm quyền thay thế (danh sách "một trong") là không.

## 14. Đủ cho bước 54 và 52 (làm sau G1–G2, D-20261126-steps-52-54-before-1-0)

Bước 54/52 viết thẳng trên hợp đồng mới, nên hợp đồng phải có sẵn chỗ cắm. Đối chiếu từng mục còn mở:

| Mục còn mở                                                     | Chỗ cắm trong thiết kế này                                                    |
| -------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| 54 Quyền theo trường dữ liệu (cá nhân / nhạy cảm / tài chính, che khi thiếu `<module>.read_personal`, đọc có audit) | `#[derive(Schema)]` nhận `#[schema(data_class = personal \| sensitive \| financial)]` trên trường: sinh `x-autoxide-data-class` và cài trait `Redact`. Che ở **mọi lối ra**, không chỉ response — xem §14.1 |
| 54 Chia sẻ bản ghi có hạn (`viewer`/`editor`, không trao quyền mình không có), API key chọn bản ghi | `manifest.relation(Relation::new("notes.note", ["viewer", "editor"]))`; `principal.can::<Note>(&handle, &NOTES_READ)` trả `Cond<Note>` — **chỉ là predicate**, dựng từ cùng handle nên `EXISTS` vào `kernel_shares` lọc cùng khách. Điều kiện: `kernel_shares` có ở **mọi** placement (bảng cấp khách của kernel, đi theo khách khi dời); `Record` khai cột nút/người sở hữu bằng `#[record(org)]`, `#[record(owner)]`; `count`/`exists`/hàm gộp nhận `Cond` như `find`. Rủi ro index của `OR EXISTS`: §11 |
| 54 Nâng quyền đúng lúc, tài khoản khẩn cấp, lệnh hai người, cặp quyền xung đột (ngưỡng là dữ liệu) | Khai trong manifest: `manifest.conflicting(PERM_A, PERM_B)`, `manifest.two_person(&PERMISSION)`; `#[endpoint(approval = "…")]` sinh `x-approval` và bọc handler bằng bước duyệt của kernel (REST và MCP như nhau) |
| 54 Giải thích, mô phỏng, "ai có quyền trên bản ghi này", quyền không dùng 90 ngày | Manifest là dữ liệu: kernel có đủ permission, quan hệ, xung đột của mọi module mà không cần khởi động; số lần dùng permission đo ở bước kiểm `require` |
| 54 AuthZEN Authorization API 1.0, `on_behalf_of` cho agent | Loại tài nguyên = `manifest.relation`/`resource_type`; `on_behalf_of` là trường của `Principal` và `Actor`, đi vào khung event (§13.1) và webhook (§13.2) |
| 54 Nhân sự nền tảng xin vào dữ liệu khách, "xem như" chỉ đọc | Ba lớp (kiểm lần 1, điểm 12): (1) lớp route của kernel chỉ cho method an toàn (`GET`/`HEAD`) và tool MCP khai `readOnlyHint`; (2) handle: `begin_write`/`write()`, `JobQueue::enqueue`, gửi mail/SMS, gọi nhà cung cấp qua kernel đều trả `kernel.view_as_read_only` khi `principal` là phiên "xem như"; (3) audit của kernel (cả "đọc dữ liệu nhạy cảm") ghi qua đường riêng của kernel, không qua handle của khách nên không bị chặn; `all_tenants` cấm trong phiên đó |
| 52 Thông báo theo loại event, lý do nhận, cấu hình kênh | Catalog event có schema (§13.1) cho giao diện liệt kê và lọc loại event |
| 52 Trợ lý AI chạy tool MCP với quyền người gọi, ghi là nháp / yêu cầu duyệt | Tool đăng ký trong `start` cùng service với REST (luật 8); `approval` ở `#[endpoint]` và tool dùng chung một bước duyệt |
| 52 Sức khoẻ người bán / tuân thủ, ngưỡng có ngày hiệu lực | Cài đặt hợp nhất (§8) cho ngưỡng theo khách/nút; ngưỡng theo luật là dữ liệu có ngày hiệu lực ở module, không ở hợp đồng |
| 52 Event theo bản ghi qua tool MCP, điều hướng `mobile`, ghim | Đã có hình dạng; chuyển sang `manifest.navigation(…)`, không đổi nghĩa |

Hệ quả cho thứ tự PR: phần `data_class` của `#[derive(Schema)]` và `Cond<R>` có chia sẻ là **API ổn định từ PR 2 và 4** (để
54 không phải đổi chữ ký), còn hành vi (che, audit, bảng `kernel_shares`) do bước 54 làm. §13 thành PR 5b (sau manifest, vì
`EventInfo` khai trong manifest).

### 14.1 Che dữ liệu đã phân loại ở mọi lối ra (kiểm lần 1, điểm 10)

Che ở lớp response là không đủ: dữ liệu còn ra qua xuất file, tải về, outbox (rồi webhook, thông báo), đoạn trích tìm kiếm,
find/suggest, đầu vào AI, diff audit, thông báo lỗi, ETag/cache, trường tuỳ biến động; event được ghi lúc ghi, khi chưa có
người đọc; và lọc/sắp theo trường nhạy cảm vẫn lộ giá trị qua thứ tự hay số lượng. Thiết kế:

- **Một bước egress của kernel** cho mọi đầu ra kernel sở hữu: `Data`/`Page`/`Tagged`, kết quả tool MCP (đổi từ `Value` sang
  kiểu có `Schema`), `ExportPage`/CSV, `Download` có schema, đoạn trích tìm kiếm và find/suggest, payload gửi `TextModel`
  (`instructions`/`input`), `Problem` (giá trị trong `params`). Bước đó dựa trên `Redact` sinh từ schema, chạy theo `principal`
  của người nhận.
- **Schema payload của event, chỉ mục tìm kiếm và webhook** không được chứa trường đã phân loại trừ khi khai lớp của trường đó;
  lúc giao (webhook, thông báo, SSE) kernel kiểm quyền lớp của **bên nhận** (chủ đăng ký webhook, người nhận thông báo) và che
  theo đó. Diff audit lưu trường đã phân loại dưới dạng che sẵn (chỉ ghi "đã đổi"), trừ lớp `financial` cần cho đối soát
  (lưu đủ, đọc audit cần quyền lớp đó).
- **Lọc/sắp theo trường đã phân loại** đòi quyền lớp: tham số lọc/sắp sinh từ schema mang lớp của trường, kernel từ chối
  (`permission_denied`) trước khi chạy truy vấn.
- **ETag tính sau khi che** (hai người khác quyền nhận hai ETag khác nhau); response có trường đã che mang `Vary`/`Cache-Control:
  private`.
- **Ngoại lệ có chủ đích**: bản xuất dữ liệu cá nhân cho **chính chủ** (quyền truy cập dữ liệu theo luật bảo vệ dữ liệu cá
  nhân) không che phần của chính người đó.
- Trường tuỳ biến (`custom_fields`) khai lớp khi tạo trường; giá trị đi qua cùng bước egress.

### 14.2 Mã hoá mức trường (G13) cắm vào `db` — trả lời 5 điểm của `docs/research/g13-field-encryption-audit.md` §4

Thiết kế G13 ở PR #577; mục này chốt phần chạm hợp đồng kernel. Code của G13 đi sau G1 PR 2–5 như thiết kế đó ghi.

1. **Tên và kiểu — chốt, nhưng thêm ở PR 1 của G13, không chen vào PR 2.** Bộ kiểu trường (§3.1) nhận thêm
   `db::Sensitive<T>` (`T`: `String`, `serde_json::Value`, cùng `Option<Sensitive<T>>` cho cột cho NULL). Thêm kiểu trường và
   thuộc tính mới là thay đổi **cộng thêm**: module chưa dùng không đổi gì, và derive hiện từ chối mọi tuỳ chọn lạ trong
   `#[record(…)]` nên không có module nào lỡ viết sẵn. Điều chỉnh so với #577:
   - **Kiểu quyết định, không cần `#[record(encrypted)]`**: trường `Sensitive<T>` luôn mã hoá khi lưu (một cách làm cho một
     việc). Ngoại lệ duy nhất là `#[record(plaintext = "<lý do>")]` trên trường `Sensitive<T>` (lưu rõ trong lúc chuyển, vẫn che
     ở egress); gate `sensitive-plaintext` đếm theo baseline chỉ giảm.
   - `Record` không biết `data_class` (lớp khai trên DTO `#[derive(Schema)]`, kiểu khác), nên "thiếu `encrypted` trên trường
     `data_class = sensitive` là lỗi biên dịch" của #577 không làm được ở derive `Record`. Thay bằng chuỗi kiểu: `Sensitive<T>`
     không cài `Serialize`; `reveal(&principal)` trả `Classified<T>`, và `#[derive(Schema)]` coi trường kiểu `Classified<T>` là
     `data_class = sensitive` **tự động** (sinh `x-autoxide-data-class`, `Redact` che). Giá trị đi từ cột mã hoá ra response
     không thể mất lớp dọc đường. Chiều ngược (cột nhạy cảm mà khai `String`) do rà soát ở PR 3 của G13 bắt, theo danh sách
     trường `data_class = sensitive` của DTO.
   - `scope = global` mà có trường `Sensitive<T>` là lỗi biên dịch: không có khách thì không có DEK.
2. **Chỗ gọi codec.** Ghi: trong `Tx::insert`/`insert_many`, ngay sau bước kiểm `tenant_of(row) == khách của handle` và trước khi
   dựng câu; đọc: trong `Record::read` của handle, sau khi có dòng, với khách của handle (handle `AllTenants`: khách lấy từ
   `tenant_id` của chính dòng). AAD cần khoá chính của dòng, nên:
   - `Col<R, Sensitive<_>>` **không** cài `Settable`: không `update().set(…)` hàng loạt (không biết khoá chính từng dòng).
     Đổi giá trị nhạy cảm bằng `tx.update_row(&record)` (ghi cả dòng theo khoá chính, mới ở PR 1 G13) hay `insert`.
   - `on_conflict_update` không nhận cột `Sensitive` trong danh sách cập nhật (dòng va chạm có khoá chính khác dòng chèn).
   - Không `eq`/`order_*`/`select`/hàm gộp/`group_by` trên cột `Sensitive` (thiếu impl → lỗi biên dịch); tra đúng giá trị qua
     cột `BlindIndex<T>` có `.matches(&value)`, handle tính HMAC bằng khoá của khách.
   - `db.sql`: `Sensitive` không chuyển được sang `db::Value` nên không bind được; đọc kết quả `db.sql` thành record có trường
     `Sensitive` cũng bị chặn (derive sinh dấu `HasSensitive`, hàm đọc của `Sql` đòi `R: NoSensitive`).
   - Cầu tạm `ConnectionTrait` (G2) không đi qua codec: module chỉ được khai `Sensitive` khi đã chuyển **hết** sang `db`
     (baseline `wrapped-libraries` không còn tên nó). `compliance` và `ledger` vì vậy chuyển cụm trước PR 3 của G13.
3. **Hạng và crate.** Việc tách crate đã dời sau 1.0, nên tất cả nằm trong `autoxide-kernel`, trừ phần chạy:
   - `db::Sensitive<T>`, `db::BlindIndex<T>`, `#[record(plaintext)]`: **ổn định** ở `autoxide_kernel::db` (là kiểu trường của
     `Record`; dời đường dẫn sau này là phá vỡ chữ ký của module — đúng điều #577 muốn tránh). Điều kiện: ≥ 2 module dùng trước
     1.0 (`compliance`, `ledger`); không kịp thì ở `unstable::db` tới khi đủ.
   - `KeyProvider`, `AuditAnchor`: **thử nghiệm**, `autoxide_kernel::unstable::security` — là hợp đồng của bộ nối nhà cung cấp
     (KMS, Vault, Object Lock, RFC 3161), cùng tiêu chí với `payment`/`sms` (§2.1). `audit` (official) dùng `AuditAnchor` bên
     trong được, miễn `api` của nó không lộ kiểu này (gate `stable-signatures`).
   - Codec `enc:v1`, bộ nhớ đệm DEK, `LocalKeyProvider`: **nội bộ** — codec và cache ở kernel (handle gọi), không công khai;
     `LocalKeyProvider`, lệnh `autoxide keys …`, job `kernel.fields.rekey` thuộc `autoxide-host` khi tách.
4. **`kernel_data_keys`: đúng, bảng cấp khách của kernel**, có `tenant_id`, ở mọi placement, **không** nằm trong
   `control_plane_tables`, đi theo khách khi dời, cùng loại với `kernel_shares` (§14). Chỉ kernel ghi (không có `Record` công
   khai). Hai hệ quả phải làm cùng PR 1 của G13:
   - **Tách / gộp khách đổi `tenant_id` của dòng** (`relocate::kernel_tables` đặt lại `tenant_id`), mà AAD có `tenant_id`: bước
     tách/gộp phải mã hoá lại cột `Sensitive` (mở bằng DEK của khách nguồn, khoá bằng DEK của khách đích). `Relocation` của module
     liệt kê cột `Sensitive` từ `Module::records` (đã có `RecordInfo`), không bắt module khai lần nữa. Dời placement giữ
     `tenant_id` nên không cần.
   - Xuất khách sang bản cài khác (`transfer`): DEK bọc bằng KEK của bản cài nguồn vô dụng ở đích → bản xuất mang giá trị đã
     giải mã trong file mã hoá của lệnh xuất (như hiện nay với dữ liệu), hay bọc lại DEK bằng khoá của đích; chọn ở PR 2 của G13.
5. **Egress: đúng, `Sensitive` = `data_class = sensitive`, che kể cả khi lưu rõ.** Che dựa trên lớp của trường DTO, không dựa
   trên cách lưu; `plaintext` chỉ đổi cách lưu, không bao giờ đổi việc che. Hai lớp kiểm khác nhau: `reveal(&principal)` kiểm
   **người đang thao tác** (và ghi audit "đọc dữ liệu nhạy cảm"); bước egress (§14.1) kiểm **người nhận** (chủ webhook, người
   nhận thông báo, bên tải file xuất) — dữ liệu đã `reveal` cho người A vẫn bị che khi đi tới người B thiếu quyền. Payload event
   và chỉ mục tìm kiếm không chứa được `Sensitive` (không `Serialize`); muốn mang thì khai trường DTO với lớp, như §14.1.

## 15. Kết quả ba spike (trước PR 2)

Mã nháp ở nhánh `claude/kernel-contract-spikes` (PR nháp, **không merge**): crate `spikes/db-api` (nguyên mẫu `db`),
`spikes/macros` (nguyên mẫu `#[derive(Schema)]`), và `content` đổi sang derive nháp.

### 15.1 Spike (a): lớp `db` chạy các truy vấn khó nhất trên cả hai database

Một test (`spikes/db-api/tests/hardest.rs`) dựng lại, chỉ bằng API của `db` (không có kiểu SeaORM/sea-query trong mã
"module"), các hình dạng khó nhất của bốn module, có dữ liệu của hai khách giống hình dạng nhau, chạy **xanh trên SQLite và
Postgres 16**:

1. `ledger` — tổng có dấu theo bên nợ: `sum(case_when(SIDE = 'debit', AMOUNT, -AMOUNT))` gom theo tài khoản, lọc khoảng ngày.
2. `inventory` — lượng đang giữ `sum(QUANTITY - FULFILLED - RELEASED)` gom theo kho, lọc `IN` danh sách.
3. `reporting` — gom theo (nút, ngày, tiền tệ) với hai `SUM` ép `BIGINT`, lọc `IN` + khoảng ngày; `max(ngày)` trả `Option`.
4. `catalog` — bộ lọc thuộc tính dựng động (`Cond::any()` nhiều giá trị) qua subquery, cộng tìm chữ trong subquery thứ hai;
   `%` và `_` trong chữ tìm là chữ thường, không phải ký tự đại diện.
5. Khoá: `for_share()` trên kỳ kế toán, `for_update()` trên mức tồn, `lock_key` theo tên, `adjust` có cận (trừ quá thì 0 dòng;
   `MAX + 1` tràn thì 0 dòng, không lỗi, giống nhau hai database).
6. Upsert trên khoá unique **không** có `tenant_id`: khách B va dòng của khách A → lỗi `db.unique_conflict`, dòng của A không đổi;
   upsert của chính A vẫn cập nhật; `insert` dòng mang `tenant_id` khác bị từ chối trước khi gửi câu.
7. SQL viết tay trên handle của khách: thiếu `:tenant` thì lỗi; có thì kernel gắn giá trị.
8. Cách ly: khách B không đọc, sửa, xoá được dòng của A dù biết ID; `join` cùng module lọc khách ở bảng được nối; `join` sang bảng
   của module khác bị từ chối.

Hai lối vượt thành **lỗi biên dịch** (doc test `compile_fail`, kèm một doc test đối chứng biên dịch được): truy vấn record cấp
khách qua handle control-plane, và `update().set(TENANT_ID, …)`.

Điều chỉnh thiết kế rút ra: cột `tenant_id` dùng kiểu `TenantKey` (không `Settable`); ràng buộc handle bằng trait
`AcceptedBy<Handle>` trên `Scope`; hàm gộp số nguyên luôn `CAST(COALESCE(…, 0) AS BIGINT)`; `on_conflict` nhận danh sách cột đã
xoá kiểu (`ColName`). Chưa phủ trong spike (để PR 2): `insert_many`, `left_join`, `exists`, `page*`, `count_distinct`, và bộ
sinh `#[derive(Record)]` thật (spike dùng `macro_rules!` đứng thay).

### 15.2 Spike (b): `#[derive(Schema)]` bản sao bóng giữ `openapi.json` từng byte

- Đổi **toàn bộ** 135 kiểu `ToSchema` của `content` (module có nhiều thuộc tính đặc biệt nhất: 99 chỗ `flatten`/`value_type`/
  `inline`/`no_recursion`/`deprecated`) sang derive nháp: `docs/openapi.json` (8.807.180 byte) và `docs/mcp-tools.json` (518.613
  byte) sinh lại **giống từng byte**; module không còn gọi tên `utoipa` ở derive.
- Kiểu generic (`Wrapped<T>` có doc comment, `#[deprecated]`, `#[serde(flatten)]`, `rename_all`, enum có biến thể dữ liệu):
  `schema()`, `name()`, `schemas()` giống hệt derive gốc của utoipa.
- Hai điều chỉnh do spike lộ ra: utoipa 6 còn cài trait ẩn `utoipa::__dev::ComposeSchema` (dùng khi kiểu là tham số generic như
  `Data<T>`) — bản sao phải chuyển tiếp nó; và `PartialSchema` đến từ impl chung của utoipa cho mọi kiểu có `ComposeSchema`, nên
  **không** cài `PartialSchema` riêng (trùng impl). Vì đây là API `__dev` (ẩn) của utoipa, test của `autoxide-macros` phải so
  `openapi.json` mỗi lần nâng utoipa (đã có trong luật nâng ở §4.2).
- Chi phí biên dịch: `cargo check -p autoxide-content` gia tăng 34,6 s (bản sao) so với 33,2 s (derive gốc), **+4%**, dưới ngưỡng
  5% — trên module nặng schema nhất.

### 15.3 Spike (c): orphan rule, vòng phụ thuộc, số đo tách crate

- Orphan rule: `Timestamp`/`Date` cài `ValueType`/`TryGetable`/`Nullable` của SeaORM; `Error`, `Data`, `Page`, `Created`, `Tagged`
  cài `IntoResponse` của axum; `PageQuery`… cài `IntoParams`, `Currency`/`DecimalString` cài `ToSchema` của utoipa;
  `Principal`, `TenantContext`… cài `FromRequestParts`. Tách lớp vẫn làm được nếu crate "giá trị" phụ thuộc cả SeaORM, axum,
  utoipa (không phải bỏ `?`/`From`), nhưng khi đó lớp "giá trị" không còn nhẹ.
- Phụ thuộc ngược (đếm `use crate::…` giữa các nhóm đích): 157 chỗ, trong đó 117 từ `http` (phần server: tầng khách, hạn mức,
  nhật ký request) sang `access`/`tenancy`/`entitlement`… — phần đó thuộc `autoxide-host`.
- Số đo và ước tính: bảng ở D-20261126-kernel-crates. Tóm tắt: sửa một file kernel → `cargo build -p autoxide` 455 s, `cargo
  check` ~210 s; kernel chỉ chiếm 25–30%, còn lại là ~40 module biên dịch lại vì đều phụ thuộc kernel. Tách theo lớp rút 6–9%
  — dưới ngưỡng 20% → **dời sau 1.0**, trước 1.0 chỉ tách `autoxide-host` (đã quyết).
- Cái rút thời gian thật nằm ở phía module (70–75%), không ở kernel: ghi để xét sau 1.0 (vd module chỉ phụ thuộc phần kernel
  mình dùng, hay giảm mã sinh ra của derive).

### 15.4 Spike F9 (trước PR 4): `#[endpoint]` + `Params` trên module trọn vẹn

Mã nháp ở nhánh `claude/kernel-contract-spike-f9` (**không merge**), dựng trên PR 2 (#568). `autoxide-macros` có thêm
`#[endpoint]`, `#[derive(Schema)]`, `#[derive(Params)]`; kernel có `http::{Endpoint, Routes, ModuleRouter}` và `routes!`.
Hai module chuyển trọn: `notes` (6 endpoint, dùng `etag`) và `audit` (3 endpoint, `IntoParams` có `#[into_params]`,
`#[param(example)]`, `#[param(inline)]` trên enum cũng đổi sang `Schema`). **Cả hai không còn khai `utoipa` / `utoipa-axum`**.

- **Giống từng byte.** `audit` có trong binary: `cargo xtask regen --check` không lệch `docs/openapi.json` lẫn
  `docs/mcp-tools.json`; đối chứng: sửa một chữ trong mô tả thì lệnh báo lệch. `notes` không có trong binary: test
  `spike_f9_openapi_giong_tung_byte` so tài liệu OpenAPI của app có `notes` với bản chụp từ mã cũ; đối chứng tương tự cho đỏ.
  So sau khi qua `serde_json::Value` (khoá sắp theo tên), như `autoxide openapi` làm: extension của operation nằm trong
  `HashMap` của utoipa nên thứ tự khoá đổi mỗi lần chạy, so chuỗi thô là sai.
- **Thời gian biên dịch: không đo được khác biệt.** `-Ztime-passes` của `audit`: 2,04 s (mới) và 2,08 s (cũ). Đo xen kẽ
  bản mới / bản cũ, chạm file nguồn rồi biên dịch lại riêng module, 6 lần mỗi ô, lấy trung vị:

  | Module | `check` cũ → mới | `build` cũ → mới |
  | --- | --- | --- |
  | `notes` | 1,43 → 1,43 s (+0,5%) | 2,76 → 2,94 s (+6,4%) |
  | `audit` | 2,55 → 2,52 s (−1,0%) | 4,62 → 4,57 s (−1,1%) |

  Lượt đo đầu (đo hết bản cũ rồi mới đo bản mới) cho `audit` +20%, `notes` −12%: đó là nhiễu do điều kiện máy đổi giữa hai
  đợt. Luật đo cho PR 4 (§4.2): **đo xen kẽ hai bản trong cùng một đợt**, không đo hai đợt nối tiếp.
- **Cách làm đã chạy** (giữ cho PR 4):
  - `#[endpoint]` giữ nguyên hàm của module, sinh thêm một struct đánh dấu **cùng tên hàm** cài `http::Endpoint`.
    Struct phải có ngoặc nhọn (`struct x {}`): struct đơn vị `struct x;` chiếm cả namespace giá trị nên trùng tên hàm.
    `routes![a, b]` vì vậy chỉ cần tên hàm, không cần `paste`.
  - Trong thân `Endpoint::__describe` có một bản sao chữ ký của hàm (thân `loop {}`, không bao giờ chạy) mang
    `#[utoipa::path]`, với `utoipa` lấy từ `::autoxide_kernel::__private`. Bản sao giữ nguyên mẫu tham số, vì feature
    `axum_extras` của utoipa đọc extractor (`Path(id): Path<Uuid>`, `Query<T>`) để điền tham số.
  - Ba đường tắt đã dùng: `permission = "…"` (sinh `x-permissions` + `security`), `operation_id` mặc định là tên hàm, `etag`
    trên response (header `ETag` chuẩn).
  - `Routes` thêm từng endpoint một: axum gộp các method của cùng một path, utoipa gộp operation, nên không cần gom theo
    path như `utoipa_axum::routes!`.
- **Còn để PR 4 quyết:**
  - Endpoint không cần permission (`x-permissions: []`, như `GET /audit/activity`) vẫn viết tay `extensions` + `security`.
    Cần một dạng ngắn, ví dụ `permission = none`.
  - Response 401/403 chuẩn chưa sinh tự động: mô tả 403 khác nhau theo endpoint (nêu các mã lỗi). Chỉ sinh khi module
    không tự viết.
  - `Module::routes` còn trả `ModuleRouter`, bí danh tạm của `OpenApiRouter`, tới khi manifest (PR 5) nhận `Routes`.
  - `ValidJson` vẫn dựa trên `validator`, như §4.3.
- **Bẫy khi đo:** nhiều worktree dùng chung `CARGO_TARGET_DIR` ghi đè bản build của nhau, vì hash của crate path trong
  workspace không gồm đường dẫn. Mỗi worktree một thư mục target, như luật chia agent.

## 16. Mở rộng hợp đồng: CLI và định dạng file (71-G14 mục 4, D-20261126-operations-baseline)

`COMPATIBILITY.md` đã hứa về lệnh `autoxide` bằng lời nhưng không có gì kiểm; file sao lưu (`backup.json` ghi bản Autoxide đã
tạo, không có số bản định dạng) và file xuất dữ liệu (dữ liệu cá nhân, dời khách, xuất danh sách) chưa có lời hứa nào.

- **CLI**: `cargo xtask regen` sinh `docs/cli.json` từ cây lệnh của `clap` (lệnh, tham số, giá trị mặc định, biến môi trường
  tương ứng) cộng bảng mã thoát khai ở một chỗ (`ExitCode` của `crates/app`, mỗi mã một ý nghĩa ổn định) và JSON Schema của
  mọi đầu ra `--json`. Quy tắc như REST: thêm lệnh / tham số tuỳ chọn / mã thoát / trường là không phá vỡ; bỏ, đổi tên, đổi
  nghĩa là phá vỡ — tên cũ giữ làm bí danh có cảnh báo tới bản lớn (như `Setting::renamed_from`). Chữ in cho người đọc không
  là hợp đồng.
- **Định dạng file**: mỗi loại file (sao lưu, gói dời khách, xuất dữ liệu cá nhân, xuất danh sách) ghi `format` (tên loại) và
  `format_version` (số nguyên) ở đầu (`backup.json` / dòng đầu của file xuất). Luật: bản 1.x **đọc** được file của mọi bản
  1.y ≤ x (đọc bản cũ bằng hàm nâng từng bước `vN → vN+1` trong bộ đọc, không sửa file); ghi luôn bản mới nhất; đọc file mới
  hơn mình thì báo lỗi rõ (`backup.format_too_new` kèm bản cần có). Bản lớn mới (2.0) đọc được file của bản 1.x cuối.
- Sửa sau kiểm lần 2 (F6):
  - File của bản `0.x` (chưa có `format_version`) là **v0**; bộ đọc có bước nâng `v0 → v1` (suy loại file từ cấu trúc
    `backup.json` hiện có), có test bằng file mẫu do bản `0.x` đã phát hành tạo ra.
  - File mẫu **do binary đã phát hành tạo ra** (bước phát hành chạy binary vừa dựng trên dữ liệu mẫu cố định rồi commit kết
    quả), không bao giờ sửa tay — gate giữ như migration đã commit (đổi nội dung file mẫu đã có là đỏ).
  - Khôi phục: dựng database ở đúng trạng thái migration mà file sao lưu ghi (tập migration của bản đã tạo nó), nạp dữ liệu,
    rồi chạy `up` tới bản hiện tại; so kết quả **qua API** (đọc lại bản ghi mẫu bằng REST), không so bảng.
  - Khôi phục chéo: sao lưu là dữ liệu độc lập database (không phải dump SQL), nên SQLite → Postgres và Postgres → SQLite
    **được hỗ trợ** và có test trong CI theo cả hai chiều; giới hạn duy nhất là cỡ (SQLite theo `docs/` sức chứa của G14).
- **Kiểm**: mỗi bản phát hành commit một file mẫu cho mỗi loại (`tests/fixtures/formats/<loại>/v<N>.*`, sinh từ dữ liệu mẫu cố
  định); test đọc mọi file mẫu bằng bản hiện tại, trên cả hai database (khôi phục sao lưu mẫu rồi so dữ liệu). `contract-diff`
  so `docs/cli.json` và JSON Schema của từng định dạng với bản phát hành gần nhất: schema của định dạng đổi mà `format_version`
  không tăng → đỏ.
- Thứ tự: PR 5c (cùng đợt với 5b, sau manifest), vì `contract-diff` dùng chung.

## 17. Đường chuyển 0.x → 1.0 cho người viết module (71-G14 mục 5)

Hướng dẫn `docs/guides/migrating-to-1.0.md` + `.vi.md`, viết theo từng bước, mỗi bước là một thay đổi biên dịch được (thứ tự
như các cụm trong repo: Cargo.toml → derive/HTTP → manifest → truy vấn → migration), kèm bảng "cũ → mới" sinh từ chính bảng
luật của công cụ dưới đây (một nguồn).

Công cụ `autoxide migrate-module <đường dẫn>` (trong công cụ dòng lệnh độc lập của 71-G6, dùng được không cần mã nguồn core;
trong repo gọi qua `cargo xtask migrate-module`):

- **Cách làm**: phân tích cú pháp bằng `syn` (đã là phụ thuộc của `autoxide-macros`) với `proc-macro2` bật `span-locations`
  để biết vị trí từng nút, rồi **sửa văn bản gốc theo vị trí** (không in lại cả file) — giữ nguyên comment, định dạng, thứ tự;
  chạy `rustfmt` sau cùng. Không dùng rust-analyzer SSR: cần cả rust-analyzer chạy được trên dự án của người dùng, khó tự động
  hoá trong CI và khó cài cho người mới. Không in lại bằng `prettyplease`: mất comment.
- **Luật máy móc** (chắc chắn đúng, áp thẳng): sửa `Cargo.toml` (bỏ thư viện bọc, thêm feature `unstable` nếu dùng đường
  thử nghiệm); `use` theo bảng đường dẫn; `derive(ToSchema|IntoParams|Validate)` → `derive(Schema|Params|Validate)` của kernel;
  `#[utoipa::path(` → `#[endpoint(` và gộp `x-permissions` + `security` + 401/403 thành `permission = "…"` khi đúng mẫu chuẩn;
  `OpenApiRouter::new().routes(routes!(…))…` → `routes![…]`; `StatusCode::NO_CONTENT` → `NoContent`; `jiff::SignedDuration` →
  `time::Duration`; 45 hàm của `Module` → một hàm `manifest()` (thân mỗi hàm cũ là một biểu thức, chuyển thành một lời gọi
  builder) và các đăng ký lúc chạy vào `start`; entity `DeriveEntityModel` → `#[derive(Record)]` (trường giữ nguyên, thuộc tính
  ánh xạ, `scope` suy từ việc có `tenant_id` + `TenantScoped`); chuỗi truy vấn SeaORM theo mẫu thường gặp (`Entity::find()
  .filter(Column::X.eq(v)).scoped(&tenant).one(conn)` → `tenant.db().read().find::<R>().filter(R::X.eq(v)).one()`).
- **Phần không chắc**: công cụ không đoán; chèn `// TODO(autoxide-migrate): <luật, đường dẫn tài liệu>` tại chỗ và in danh
  sách; `cargo check` dẫn phần còn lại.
- Sửa sau kiểm lần 2 (F7):
  - `syn` không nhìn vào thân macro (`macro_rules!`, `json!`, `select!`, `vec!`…): công cụ quét token của thân macro tìm đường
    dẫn thư viện đã bọc (`sea_orm::`, `utoipa::`, `axum::`…) và chèn TODO tại chỗ, không sửa.
  - Vị trí: cột của `span-locations` đếm theo ký tự; công cụ đổi (dòng, cột) sang byte offset bằng bảng đầu dòng của file và
    đếm ký tự UTF-8 (comment tiếng Việt), có test riêng cho dòng có chữ có dấu trước chỗ sửa.
  - Chỉ định dạng lại phần đã sửa (chạy `rustfmt` trên file rồi chỉ giữ thay đổi nằm trong khoảng đã sửa; file người dùng không
    đổi gì ngoài chỗ sửa).
  - Chuỗi gọi SeaORM không có thông tin kiểu (công cụ chỉ thấy cú pháp): luật truy vấn chỉ áp cho mẫu nhận ra chắc chắn từ cú
    pháp (`<path>::Entity::find()` … `.one/.all(...)` với `Column::X` cùng đường dẫn entity); còn lại là TODO.
  - Đích đo: ≥ 80% chỗ gọi được sửa tự động, đo trên **một module thật cỡ vừa** của repo (vd `forms` hay `support`) ngoài `notes`
    và `examples/external-module`, báo số đếm theo từng luật (áp được / TODO) trong PR.
- Đích đo ban đầu: ≥ 80% chỗ gọi của một module điển hình được sửa tự động (đo trên `notes` và
  `examples/external-module`, ghi số ở PR).
- **Tự kiểm**: test của công cụ là cặp file trước/sau cho từng luật; chạy lại trên kết quả không đổi gì (idempotent).
- **Dùng trong repo**: mọi PR chuyển cụm module (7–12) chạy công cụ trước rồi mới sửa tay phần TODO — vừa giảm công, vừa thử
  công cụ trên ~6.000 chỗ thật trước khi người ngoài dùng. CI chạy công cụ trên bản sao `examples/external-module` ở trạng thái
  0.x (giữ một bản chụp ở `xtask/fixtures/external-module-0x/`) và kiểm kết quả biên dịch + test xanh.
- Thứ tự: công cụ làm ở PR 6b (sau khi lớp mới của PR 2–5 có đủ, trước cụm module đầu tiên); luật mới thêm theo từng PR lớp
  của kernel nếu lớp đó còn đổi.

## Quyết định đi kèm

- D-20261126-contract-tiers — ba hạng, cơ chế, danh sách xuất phát
- D-20261126-kernel-data-layer — `db`, `Record`, handle theo khách, SQL viết tay, migration `Plan`
- D-20261126-migration-rewrite-proof — ngoại lệ có điều kiện cho luật "migration đã commit thì không sửa"
- D-20261126-kernel-web-layer — `endpoint`, `routes!`, `Schema`, `Validate`, extractor, `autoxide-macros`
- D-20261126-module-manifest — `Module` bốn hàm, `Manifest`, `StartContext` gọn
- D-20261126-kernel-crates — tách crate theo lớp, `autoxide-host`
- D-20261126-tenant-isolation — ba lớp cách ly, RLS tuỳ chọn, hạn mức request chưa xác thực
- D-20261126-one-settings-system — hợp nhất cài đặt theo khách
- D-20261126-contract-versioning — đánh phiên bản event, webhook, phần mở rộng `x-*` (§13), CLI và định dạng file (§16)
- D-20261126-module-migration-tool — công cụ `migrate-module` và hướng dẫn chuyển 0.x → 1.0 (§17)
- (sẽ ghi ở PR 6) quyết định chuyển hạng module lúc 1.0 (§2.1)
