## D-20261128-library-layer-plan · Lớp của kernel thay thư viện: kiểu nào bọc, kiểu nào để thư viện, `Validate` tách khỏi `validator`, `time::Duration`, thứ tự cụm module — 2026-11-28 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định: chốt phần còn thiếu của bước 71 G2 (D-20261126-library-isolation), trên nền D-20261126-kernel-data-layer,
D-20261126-kernel-web-layer, D-20261127-http-layer-choices và thiết kế `docs/research/kernel-contract-1-0.md`.

1. **Bốn vùng, kiểu nào bọc.** Mọi kiểu module viết hay nhận đều là kiểu của kernel; thư viện chỉ ở bên trong.

   | Vùng | Module dùng (kernel) | Để thư viện (bên trong kernel, ngoài hợp đồng) |
   | --- | --- | --- |
   | Truy vấn, entity | `db`: `Record`, `Col`, `Expr`, `Cond`, `Value`, `FromRow`, handle `TenantDb`/`TenantRead`/`TenantTx`/`ControlPlane`/`AllTenants`, `Sql`; lỗi là `Error` (`is_unique_violation`) | sea-query (dựng câu), kết nối và pool của SeaORM, sqlx (mã hoá giá trị) |
   | Route, schema API | `http`: `#[endpoint]`, `routes!`, `Routes`, `#[derive(Schema/Params)]`, `Path`/`Query`/`State`/`Headers`/`Body`, `NoContent`/`Download`/`Redirect`; `JsonSchema` cho schema động | axum, utoipa, utoipa-axum, matchit; impl trait của chúng trên kiểu kernel (`ToSchema`, `IntoResponse`, `FromRequestParts`) khai là ngoài hợp đồng; `Response` thô chỉ ở `unstable` |
   | Kiểm đầu vào | `http::Validate` + `#[derive(Validate)]` (năm luật `length`, `range`, `email`, `url`, `nested`), `FieldError`, mã `validation.*` | không thư viện nào sau 1.0 (mục 2) |
   | Migration | `migration::{Migration, Plan, Table, Index, col, DataStep}` (D-20261126-kernel-data-layer mục 5) | sea-query dựng DDL cho từng database |
   | Thời gian | `time::{Timestamp, Date, Duration, TimeZone, TimeOfDay, Weekday, DateTime}` | jiff (mục 3) |
   | Test | `autoxide_testkit` tự có `StatusCode`, `Method` và bộ dựng request (mục 4) | http, tower |

   Thư viện từ vựng (dùng thẳng, được ở chữ ký): `serde`, `serde_json`, `uuid`, `tokio` (không kiểu nào ở chữ ký ổn
   định), `tracing`, `futures-core`, `bytes` — danh sách đóng của thiết kế §1.1.

2. **`http::Validate` tách khỏi `validator` trước 1.0 (mục chặn 1.0 của D-20261127-http-layer-choices).** `#[derive(Validate)]`
   của kernel sinh mã gọi hàm kiểm ẩn của kernel (không còn sinh mã của `validator_derive`), giữ nguyên năm luật, thứ tự
   lỗi và mã (`validation.length`…); bên trong kernel được dùng hàm kiểm email/url của bất kỳ thư viện nào (chi tiết cài đặt).
   Impl chung `impl<T: validator::Validate> Validate for T` giữ trong lúc chuyển (module cũ dùng `validator` trực tiếp), gỡ ở
   PR gỡ đường cũ khi module cuối bỏ `validator`; test so lỗi của hai đường (`http::input::tests`) giữ tới lúc đó. Từ 1.0
   `validator` không còn trong chữ ký hay impl công khai nào của kernel.
3. **`time::Duration` của kernel thay `jiff::SignedDuration`.** Kiểu mới có dấu, độ chính xác micro giây (như `Timestamp`),
   hàm dựng `from_secs`/`from_mins`/`from_hours`/`from_days`/`from_millis`, `as_secs`/`as_millis`, cộng trừ với `Timestamp`
   (tràn thì lỗi, không panic); không có `serde` (không chỗ nào tuần tự hoá khoảng thời gian, cấu hình đọc qua `Setting`). Lịch dân sự: `TimeZone` (IANA, nhúng sẵn),
   `TimeOfDay`, `Weekday`, `DateTime` thay `jiff::tz::TimeZone`, `jiff::civil::{Time, DateTime, Weekday}`. Bí danh cũ
   `time::SignedDuration` và hàm `zone`/`from_local`/`parse_clock` (nhận trả kiểu jiff) là đường cũ theo D-20261127-transition-paths.
   Làm thành một PR kernel riêng ngay trước cụm module đầu tiên dùng `jiff` (cụm 2 dưới đây; `auth` dùng nhiều nhất).
4. **Testkit bọc kiểu `http`.** `autoxide_testkit` có `StatusCode` (số `u16`, hằng cho các mã đang dùng, so sánh được với
   số) và `Method`, response test trả kiểu đó; gửi request tự dựng qua bộ dựng của testkit thay `http::Request` +
   `tower::ServiceExt::oneshot`. Hàm cũ nhận/trả kiểu `http` là đường cũ.
5. **Handle của `db` không còn cài `ConnectionTrait`.** Cầu tạm cho phép chạy câu SeaORM tự dựng trên handle của khách —
   câu đó không qua bộ lọc khách, tức một lối vượt. Thay bằng trait chuyển tiếp `db::Executor`: hàm kernel tự dựng câu
   (và tự lọc khách) — `Outbox::record`, `SyncLog`, `Principal::require_at`/`require_write_at`, `OneTimeTokens`… — nhận
   `&impl Executor`, mà handle và (tới khi module cuối chuyển) kết nối cũ đều thoả. Hàm chạy câu do module dựng
   (`fetch_page*`, `rows::lock_one`…) **không** nhận handle. Module đang chạy câu SeaORM trên handle chuyển các câu đó sang
   `Record` trong cùng PR.
6. **Baseline `raw-database` về rỗng theo cụm, không một PR.** 2.152 chỗ gọi `.database()` ở 32 module (đếm 28/11/2026)
   chính là phần truy vấn của việc chuyển module; mỗi PR cụm xoá mọi dòng `raw-database`, `raw-sql` (trừ chỗ đã đo),
   `wrapped-libraries` và `wrapped-libraries-dev` của module trong cụm. `TenantContext::database()` thành nội bộ
   (`pub(crate)` hay chỉ `autoxide-host`) ở PR 13 của thiết kế, khi baseline rỗng.
7. **Thứ tự cụm** (dưới lên theo phụ thuộc giữa module, để `api` của bên cung cấp đã dùng handle khi bên dùng chuyển; mỗi cụm
   một PR, lớn thì chia đôi; điều kiện trước: `Plan` (PR 3), `Manifest` (PR 5), mục 2, 3, 4):
   1. Lá hạ tầng và bộ nối: `idempotency`, `mail`, `ai`, `files`, `audit`, `webhooks`, `sepay`, `momo`, `esms`, `einvoice_sepay`.
   2. Lá nội dung: `address`, `media`, `comments`, `forms`.
   3. Nền tài khoản: `organizations`, rồi `auth` (PR riêng, chia theo vùng: OAuth, khoá API, MFA, còn lại), rồi `compliance`,
      `notifications`.
   4. Tiền: `tax`, `ledger`, `invoices`, `reporting`, `plans`.
   5. Nội dung: `content`, `menus`, `newsletter`.
   6. Thương mại 1: `catalog`, `inventory`, `promotions`, `support`, `shipping`, `ghn`.
   7. Thương mại 2: `orders`, `customers`, `bookings`, `conversations`, `analytics`, `automations`.

   Cụm 1–2 song song được; từ cụm 3 theo thứ tự. File chụp schema (`migration-schema`) không đổi ở mọi cụm.

Lý do: module chạm thư viện ở bốn vùng trên, nên bốn vùng phải có lớp trước khi chuyển module; bỏ sót một vùng thì module
đã chuyển vẫn phải khai thư viện. `validator` chỉ còn nằm trong mã sinh ra và một impl chung, nhưng impl chung là phần công
khai: bản lớn mới của `validator` đổi impl đó là phá vỡ, trái D-20261126-library-isolation. `SignedDuration` có 239 chỗ ở
module và đang xuất lại ở `autoxide_kernel::time`. Cầu `ConnectionTrait` trên handle đã bị dùng ngoài ý định (OAuth của
`auth`, `ledger` chạy câu SeaORM trên handle — không lọc khách). Đếm `raw-database` cho thấy "về rỗng" bằng chuyển phần truy
vấn của cả 32 module, không làm được trong một PR mà mỗi PR vẫn xanh và kiểm được.

Đã cân nhắc: giữ `http::Validate` gắn `validator` sau 1.0 (bản lớn của `validator` thành bản lớn của Autoxide); bỏ cầu
`ConnectionTrait` bằng cách đổi mọi hàm kernel sang `&impl Handle` ngay (module cũ chưa có handle sẽ vỡ hết); thêm
`TenantContext::read()` trả kết nối SeaORM để hạ `raw-database` (chỉ đổi tên, kiểu thư viện vẫn lộ); `Duration` không dấu
(module đang trừ hai thời điểm ra số âm); thứ tự cụm theo §10 cũ (`plans` phụ thuộc `invoices`/`ledger` nên không đặt ở nền).
