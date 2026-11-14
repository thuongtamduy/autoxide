# Bước 51 — Hợp đồng cho khung quản trị và mặt tiền

> Lộ trình: `docs/ROADMAP.md` bước 51 · Thiết kế: `docs/research/ui-design.md` · Quyết định: D-038
> (hàm mới của `Module` có mặc định), D-097 (giao diện là repo riêng), D-123 (sổ đối tượng), D-155 (sổ
> module), D-185, D-200
> Đã gộp các quyết định: D-201, D-207, D-212, D-213, D-220, D-223, D-224 (D-228); P19–P21: D-235.
> Mã `A-…` / `B-…` trỏ tới `docs/research/ui-trace/` (nguồn: `docs/prompts/ui-trace-roadmap.md`); P18 và
> `ActorKind` có nguồn `docs/prompts/ui-phase2-api.md`, màn tương ứng ở `docs/research/ui-traceability.md`.

## Mục tiêu

Giao diện quản trị của thiết kế 2 dựng **menu, hộp việc, tìm kiếm và trang tổng quan từ dữ liệu của
core**, không viết cứng: cùng một giao diện phục vụ cửa hàng độc lập, chuỗi, khách SaaS và sàn chỉ bằng
cấu hình. Module khai phần của mình qua `Module` hay `StartContext` (mặc định rỗng, D-038), kernel gom
lại và lọc theo khách, gói, permission ở nút tổ chức đang chọn. Website công khai nhúng được bản ghi
của module khác vào bài viết mà `content` không biết khái niệm của module đó.

Theo nghiên cứu các trang quản trị hàng đầu (D-201), core còn có: schema bộ lọc và view đã lưu, thao tác
hàng loạt, tìm có toán tử và mở thẳng theo mã, hộp việc theo người, giải thích quyền, agent là người dùng
thật với lệnh chờ duyệt, dòng hoạt động và ghi chú trên mọi bản ghi; cùng job API chung, xuất danh sách
chung (D-213), cờ theo người dùng (D-207), danh sách lý do theo khách (D-223); và để khách tự chỉnh mà không
cần sửa code (D-235): trường tự định nghĩa trên bản ghi, nhãn gắn bản ghi, đổi câu chữ hiển thị. Mọi phần: hàm mới của
`Module` có mặc định rỗng, chạy và có test trên cả hai database, áp như nhau cho REST và MCP.

## Bắt đầu khi

| Phần | Bắt đầu | Cần có |
|---|---|---|
| P1 Sổ điều hướng | SAU bước 35 | `Principal` theo nút, `Scope` (34); `Entitlements::{module_enabled, has_feature}` (35) |
| P2 Hộp việc · P3 Tìm xuyên phân hệ · P4 Số liệu tóm tắt | SAU bước 34 | `TenantContext` (33), `Scope` (34); P2 dùng SSE của bước 40 nếu đã có |
| P5 Hoạt động gần đây | SAU bước 34 | module `audit` có cột tổ chức (33–34) |
| P6 Nhúng bản ghi trong nội dung | ngay sau bước 33 | sổ đối tượng (D-123), `content` |
| P7–P13 (D-201) | SAU P1–P6 | P1–P6 |
| P14 Cờ theo người dùng | đã làm (D-207) | — |
| P15 Job API · P16 Xuất chung (D-213) | trong bước 51 | P16 dùng schema bộ lọc của P7 và job của P15 |
| P17 Nhập dữ liệu (D-220) | thuộc bước 56 | phần `content` / `media` / `newsletter` làm được khi P16 đã có |
| P18 Danh sách lý do (D-223) | trong bước 51 | — |
| P19 Trường tự định nghĩa · P20 Nhãn · P21 Đổi câu chữ (D-235) | trong bước 51, sau P7, P8 | P19, P20 dùng schema bộ lọc (P7) và thao tác hàng loạt (P8); hạn mức của bước 35 |

Bước 41–49 khai menu, nguồn việc, nguồn tìm, số liệu của module mình **ngay trong bước đó** nếu bước 51
đã xong phần tương ứng; chưa xong thì bước 51 khai thay cho mọi module đang có. Bước 51 đánh dấu xong khi
mọi module đã có khai báo.

## Phạm vi

Sửa: `crates/kernel/src/{module.rs, module/registry.rs, resource.rs, http/…}`, `modules/audit`,
`modules/content`, mọi module đang có (khai menu và nguồn), `crates/testkit`, `docs/openapi.json`,
`docs/MODULE_GUIDE.md` + `.vi.md`, `CHANGELOG.md`.

File mới: `kernel/src/navigation.rs`, `kernel/src/work.rs`, `kernel/src/finder.rs`,
`kernel/src/summary.rs`, `kernel/src/http/workspace.rs`.

Bảng mới của kernel (cấp khách hàng, có `tenant_id`): `kernel_saved_views` (P7), `kernel_work_states` (P10),
`kernel_approval_requests` (P12), `kernel_record_notes` (P13), `kernel_reason_codes` (P18), `kernel_custom_fields`
(P19), `kernel_tags` và `kernel_record_tags` (P20), `kernel_text_overrides` (P21); `kernel_user_flags` (P14, đã có).
File mới thêm: `kernel/src/custom_fields.rs`, `kernel/src/tags.rs`.

NGOÀI phạm vi: giao diện (repo riêng); tuỳ biến menu theo từng người dùng (ghim, ẩn); trợ lý AI trong
quản trị (bước 52); full-text search riêng (vẫn là "chứa" của `kernel::search`); nhập dữ liệu ở kernel
(dùng module `imports` của bước 56 — P17, D-220).

## Thiết kế

### P1 — Sổ điều hướng

```rust
// kernel/src/navigation.rs — chữ ký phác thảo, theo code thật khi làm
pub struct NavGroup {
    pub id: &'static str,          // "sales" — nhóm do module khai, không có danh sách cứng trong kernel
    pub label: &'static str,       // i18n key, vd "nav.sales"
    pub icon: &'static str,        // tên icon trung lập, giao diện tự ánh xạ
    pub order: u16,                // thứ tự trên thanh phân hệ, cố định cho mọi khách
}
pub struct NavItem {
    pub id: &'static str,          // "orders.list" — cũng là khoá màn hình cho giao diện
    pub group: &'static str,
    pub section: Option<&'static str>, // i18n key của nhóm con trong hàng tab
    pub label: &'static str,
    pub permission: &'static Permission,
    pub feature: Option<&'static str>, // thiếu tính năng thì mục bị khoá, không ẩn
    pub counter: Option<&'static str>, // id nguồn việc của P2 để hiện số đếm
    pub order: u16,
}
pub struct Navigation { pub groups: Vec<NavGroup>, pub items: Vec<NavItem> }

trait Module { fn navigation(&self) -> Navigation { Navigation::default() } }
```

- Kernel tự khai nhóm `home` (Tổng quan: bảng điều khiển, hộp việc, hoạt động) và `settings`.
- Hai module khai cùng một nhóm phải giống hệt nhau (nhãn, icon, thứ tự); khác thì lỗi lúc khởi động
  (như kiểm tên module).
- `GET /api/v1/kernel/navigation?org_id` → `Data<NavigationView>`: nhóm có ít nhất một mục hiện, mục
  sắp theo `order`, nhãn đã dịch theo `Accept-Language`, mỗi mục `state: available | locked` và
  `required_feature` khi bị khoá.
- Mục ẩn khi module chưa bật cho khách hoặc người gọi thiếu permission ở `org_id`; mục khoá khi module
  bật mà thiếu tính năng.
- `ETag` = vân tay của (bản dựng, tập permission của người gọi ở nút, phiên bản quyền dùng của khách).
- Tool MCP `kernel.navigation`. Permission: chỉ cần đăng nhập.

### P2 — Hộp việc và số đếm

```rust
#[async_trait]
pub trait WorkSource: Send + Sync {
    fn id(&self) -> &'static str;                  // "orders.unconfirmed"
    fn permission(&self) -> &'static Permission;
    async fn count(&self, tenant: &TenantContext, scope: &Scope) -> Result<u64, Error>;
    async fn items(&self, tenant: &TenantContext, scope: &Scope, limit: u16) -> Result<Vec<WorkItem>, Error>;
}
pub struct WorkItem {
    pub source: &'static str,
    pub severity: Severity,        // Blocking | Attention | Info
    pub title: Message,            // i18n key + tham số
    pub detail: Option<Message>,
    pub due_at: Option<DateTime<Utc>>,
    pub target: Target,            // { item: id màn hình của P1, record_id: Option<Uuid> }
    // P10: `key` ổn định (`<nguồn>:<id bản ghi>`) và `reason` (permission, nút)
}
// đăng ký trong Module::start: context.work_source(Arc<dyn WorkSource>)?
```

- Việc **tính từ dữ liệu của module** (đơn chưa xác nhận, KYC chờ, trang bắt buộc còn thiếu), không có
  bảng việc riêng: xử lý xong bản ghi thì việc tự biến mất. Không có nút "Đã xử lý" và không có trạng thái
  "đã xử lý" hay "bỏ qua" lưu riêng — người dùng chỉ hoãn và giao (P10, D-212).
- `GET /api/v1/kernel/work?org_id&limit` → gom từ các nguồn người gọi có permission, sắp theo
  `severity` rồi `due_at`; mỗi nguồn có hạn thời gian (cấu hình, mặc định 300 ms), nguồn quá hạn hay
  lỗi thì bỏ qua và ghi vào `incomplete: [source]` trong response, không làm hỏng cả hộp.
- `GET /api/v1/kernel/work/counts?org_id` → `{source: count}` cho số đếm trên menu.
- Câu đếm phải có index; đo trên bảng lớn trước khi khai. Cache số đếm ngắn hạn trong bộ nhớ, khoá gồm
  khách, người gọi, nút (D-190).
- Có SSE (bước 40) thì phát event `kernel.work_changed` để giao diện tải lại số đếm.
- Khác hộp thư `notifications` (`/me/inbox`, bước 37): đó là tin đã gửi cho một người.

### P3 — Tìm xuyên phân hệ

```rust
#[async_trait]
pub trait FinderSource: Send + Sync {
    fn kind(&self) -> &'static str;                // loại đối tượng, vd "orders.order"
    fn permission(&self) -> &'static Permission;
    async fn find(&self, tenant: &TenantContext, scope: &Scope, query: &str, limit: u16) -> Result<Vec<Found>, Error>;
}
pub struct Found { pub kind: &'static str, pub id: Uuid, pub title: String, pub subtitle: Option<String>, pub target: Target }
```

- `GET /api/v1/kernel/find?q&org_id&limit` → kết quả nhóm theo `kind`; `q` tối thiểu 2 ký tự, so bằng
  `kernel::search::fold` (không dấu vẫn tìm được). Hạn thời gian mỗi nguồn như P2.
- Mỗi nguồn tự áp phạm vi nút; nguồn người gọi không có permission thì không chạy.
- Lệnh ("tạo mới", "đi tới") giao diện tự dựng từ sổ điều hướng; core không có danh sách lệnh.
- Rate limit theo người gọi (lớp chung của kernel).
- Truy vấn có toán tử, tham số phạm vi và mở thẳng theo mã: P9.

### P4 — Số liệu tóm tắt

```rust
#[async_trait]
pub trait SummaryMetric: Send + Sync {
    fn id(&self) -> &'static str;                  // "orders.revenue_today"
    fn group(&self) -> &'static str;               // nhóm điều hướng của P1
    fn permission(&self) -> &'static Permission;
    fn label(&self) -> &'static str;               // i18n key
    /// i18n key của câu định nghĩa (D-207): tính từ lúc nào, múi giờ nào, gồm thuế không, trừ hoàn
    /// tiền không, COD tính lúc nào — đúng như code của `value`. Mặc định `None` (D-038).
    fn definition(&self) -> Option<&'static str> { None }
    async fn value(&self, tenant: &TenantContext, scope: &Scope) -> Result<MetricValue, Error>;
}
pub enum MetricValue { Money(Money), Count(u64), Ratio { numerator: u64, denominator: u64 } }
```

- `GET /api/v1/kernel/summary?group&org_id` → các số liệu của nhóm (hoặc của mọi nhóm khi bỏ `group`,
  cho trang Tổng quan), mỗi số kèm `note` tuỳ chọn và `definition` (D-207: câu định nghĩa đã dịch theo
  ngôn ngữ của request, `null` khi số liệu không khai; mô tả trường trong OpenAPI). Hạn thời gian mỗi
  số như P2.
- Gate (D-207): `cargo xtask check` hay test của app kiểm mọi `SummaryMetric` của module đang có đều khai
  `definition` và key đó có bản dịch `vi` (như kiểm `code` lỗi). Câu định nghĩa nằm cạnh code tính số,
  không viết riêng ở repo giao diện, để hộp ⓘ "Doanh thu hôm nay tính thế nào?" luôn đúng. Phần định nghĩa
  chưa làm — làm cùng P4.
- Số liệu nặng (doanh thu theo ngày) đọc từ bảng dẫn xuất của module hay của `reporting` (bước 38),
  không quét bảng giao dịch mỗi lần mở trang.

### P5 — Hoạt động gần đây (module `audit`)

- `GET /api/v1/audit/activity?org_id&limit` → câu đọc được ("CN Quận 3 xác nhận đơn BM-…"), dịch theo
  i18n key `<event>.activity` mà module khai trên `EventInfo` (mặc định không có: event đó không hiện).
- Chỉ trả mục mà người gọi có permission đọc loại bản ghi đó ở nút của bản ghi; khác `GET /api/v1/audit`
  (dành cho người có `audit.read`, xem trước / sau đầy đủ).

### P6 — Nhúng bản ghi của module khác trong nội dung

- `Resource` của sổ đối tượng (D-123) thêm `image: Option<Uuid>` (ID media) — thêm trường, không phá
  vỡ (`#[non_exhaustive]`).
- Thân bài viết của `content` (ProseMirror JSON) có loại node `embed` với `attrs { kind, id }`. Khi lưu,
  `content` kiểm `kind` đã đăng ký và đối tượng tồn tại qua sổ đối tượng; khi đọc công khai, trả kèm
  `embeds: { "<kind>:<id>": {title, summary, url, image} }` cho các đối tượng `public`, bỏ đối tượng
  không còn công khai. `content` không biết sản phẩm là gì.
- Giá, tồn kho của sản phẩm nhúng thì website tự gọi API công khai của `catalog`, `inventory` (luôn
  mới, không chụp vào bài).
- Bước 41 đăng ký loại `catalog.product`.

### P7 — Schema bộ lọc và view đã lưu (D-201)

- Mỗi endpoint danh sách khai **schema bộ lọc** (tên tham số, kiểu, giá trị cho phép, toán tử) bằng một
  phần mở rộng trong OpenAPI (`x-autoxide-filters`), sinh từ cùng kiểu Rust với `IntoParams` — không
  viết hai lần. Giao diện và agent dựng chip lọc từ đó.
- `GET/POST/PATCH/DELETE /api/v1/me/views` (bảng `kernel_saved_views`, cấp khách hàng, có `tenant_id`):
  `resource` (id danh sách, vd `orders.list`), `params` (kiểm theo schema khi lưu — sai thì 422 liệt kê
  từng tham số), `columns`, `group_by`, `density`, `scope` (`personal` / `node` / `tenant`, kèm `org_id`),
  `owner`. View phạm vi nút cần permission `kernel.views.share`. View hệ thống do module khai, không xoá được.
- **Bộ lọc nội dung (A-c5).** Bài viết: `review=pending`, `scheduled=true`, `ai_generated=true`,
  `missing_locale=<locale>`; media: `missing_alt=true` (kể cả thiếu bản dịch `alt` của một ngôn ngữ, D-202).
  Khai trong schema bộ lọc như mọi bộ lọc khác.

### P8 — Thao tác hàng loạt (D-201)

- Module khai `BulkAction { id, permission, run }`. `POST /api/v1/kernel/bulk/{resource}/{action}`
  nhận **một trong hai**: `ids` (tối đa 500) hoặc `filter` + `expected_count`; số khớp lúc chạy khác
  `expected_count` thì 409 `kernel.bulk_count_changed`. `Idempotency-Key` bắt buộc.
- Chạy bằng job; mỗi bản ghi kiểm quyền riêng ở nút của nó; kết quả từng bản ghi (`ok` / `skipped`
  kèm lý do / `failed` kèm `code`) đọc ở `GET /api/v1/kernel/jobs/{id}` của P15 (có tiến độ), đẩy qua SSE
  nếu bước 40 đã có.

### P9 — Tìm nâng cao và mở thẳng (D-201)

- `FinderSource` nhận truy vấn đã phân tích: từ khoá còn lại + toán tử `is:<loại>`, `<trường>:<giá trị>`,
  so sánh `>`, `<`, khoảng `..`, phủ định `-`. Toán tử nào nguồn không hiểu thì bỏ, ghi vào `ignored`.
- Tham số `scope=node|subtree|accessible` cho tìm và cho danh sách; `accessible` = mọi nút người gọi có
  quyền (bước 34 `/me/memberships`).
- `GET /api/v1/kernel/resolve?code=` — module đăng ký bộ nhận dạng mã (số đơn, mã vận đơn, số hoá đơn);
  trả bản ghi đầu tiên người gọi được xem, không trả gì nếu không có quyền (không lộ sự tồn tại).

### P10 — Hộp việc theo người, việc thiết lập (D-201)

- `WorkItem` có `key` ổn định (`<nguồn>:<id bản ghi>`) và `reason` (permission, nút). Bảng
  `kernel_work_states` (cấp khách hàng): `key`, `user_id`, `snoozed_until`, `assignee_id`. `POST
  /api/v1/me/work/{key}/snooze {until}`, `DELETE /api/v1/me/work/{key}/snooze` (bỏ hoãn; A-c10, A-c11), `POST
  …/assign {user_id}` (người nhận phải có quyền của nguồn). Việc tự hết khi dữ liệu không còn khớp; trạng
  thái mồ côi dọn bằng job. Không có "Đã xử lý" (D-212).
- `SetupStep` (đăng ký như `WorkSource`): `id`, `label`, `models` (mô hình áp dụng), `blocking`
  (chặn bán), `done(tenant) -> bool` tính từ dữ liệu. `GET /api/v1/kernel/setup` trả danh sách và tiến độ.

### P11 — Giải thích quyền (D-201)

`POST /api/v1/me/can` nhận tối đa 50 cặp `{permission, org_id}`; mỗi cặp trả `allowed` và khi không:
`reason` (`missing_permission`, `module_not_enabled`, `feature_not_in_plan`, `quota_exceeded`),
`required_feature`, `upgrade_to` (gói rẻ nhất có tính năng), `grantors` (tối đa 5 người cấp được ở nút
đó — chỉ trả cho người gọi có quyền xem thành viên). Kết quả khớp đúng logic kiểm quyền thật: test so
từng lý do với lỗi thật mà endpoint tương ứng trả.

### P12 — Agent là người dùng, lệnh cần duyệt (D-201, D-213, D-224)

- API key có `kind`: `human` (mặc định), `agent`, `service`.
- **`ActorKind` (D-224).** Enum của kernel: `human`, `agent`, `service`, `system`, `support`, `app`, `scim`.
  Audit ghi `actor_kind`; danh sách audit lọc được theo nó, nhiều giá trị:
  `GET /api/v1/audit?actor_kind=agent,service`; index (`tenant_id`, `actor_kind`, `id`).
- Module đánh dấu lệnh rủi ro: `Command::requires_approval_for(ActorKind::Agent)`; `ModuleTool` thêm
  `requires_approval_for_agent` (A-c12). Agent gọi lệnh đó thì nhận `202` mã `approval_required` kèm
  `approval_request` (`id`, `summary`, `expires_at`, mặc định 24 giờ, đường dẫn duyệt). Người có cùng
  permission duyệt thì lệnh chạy với `Idempotency-Key` gốc; từ chối hay hết hạn thì agent nhận lý do. Mỗi **loại**
  yêu cầu duyệt khai permission của người duyệt (mặc định: cùng permission với lệnh); loại mà người xin không có
  quyền ở khách — vd `support_access` của bước 54, nhân sự nền tảng xin vào — khai permission duyệt riêng
  (`auth.support_access.approve`, D-232). Bảng `kernel_approval_requests` cấp khách hàng. Như nhau cho REST và tool MCP (luật kiến trúc 8). Yêu cầu
  đang chờ vào hộp việc (P2).
- **Yêu cầu duyệt đầy đủ (D-213, A-c2).** `GET /api/v1/kernel/approvals?status=`,
  `GET /api/v1/kernel/approvals/{id}`, `POST /api/v1/kernel/approvals/{id}/approve`, `POST …/reject {reason}`;
  event `kernel.approval_decided` (agent biết kết quả mà không hỏi lại liên tục); tool MCP `list_approvals`,
  `get_approval` (`read_only`), `decide_approval`.

### P13 — Dòng hoạt động và ghi chú trên mọi bản ghi (D-201)

- `GET /api/v1/kernel/records/{kind}/{id}/activity` gộp audit của bản ghi và ghi chú, chỉ khi người gọi
  đọc được bản ghi (hỏi bộ tra của sổ đối tượng, D-123).
- `POST …/notes {body}` — ghi chú nội bộ (bảng `kernel_record_notes`, cấp khách hàng); `@` nhắc người
  có quyền đọc bản ghi, gửi thông báo qua `notifications` nếu đã có. Module `customers` (bước 47) dùng
  chỗ này thay vì tự có ghi chú.

### P14 — Cờ theo người dùng (đã làm, D-207)

Bảng `kernel_user_flags` của module nội bộ `kernel` (`id`, `user_id`, `key` ≤ 100 ký tự `a-z0-9._-`,
`value` ≤ 200 ký tự, `updated_at`; duy nhất (`user_id`, `key`)). API dưới `/api/v1/me` (tiền tố dành
riêng cho endpoint "của tôi" do kernel phục vụ; `me` là tên module bị cấm):

- `GET /api/v1/me/flags?prefix=hint.` — cờ của chính người gọi, theo thứ tự khoá;
- `PUT /api/v1/me/flags/{key}` `{ "value": "dismissed" }` — ghi đè, gọi lại an toàn (`ON CONFLICT`);
- `DELETE /api/v1/me/flags/{key}` — 204, kể cả khi không có.

Chỉ cần đăng nhập, không permission; không endpoint đọc cờ người khác. Tối đa `AUTOXIDE_USER_FLAGS_MAX`
(500) cờ mỗi người, vượt → 409 `kernel.flag_limit_reached`. Cờ có trong bản xuất dữ liệu cá nhân
(`PersonalData`) và bị xoá khi nhận `privacy::USER_DELETED` — cơ chế đã có của kernel, không chờ bước 37.
**Bước 33** thêm `tenant_id` cho bảng này như mọi bảng cấp khách hàng, cùng test
`flag_cua_khach_nay_khong_lo_sang_khach_khac`.

### P15 — Job API chung (D-213, A-b2, A-c1)

`GET /api/v1/kernel/jobs?status=&kind=&org_id=` (job người gọi được xem: của mình, hay theo permission
`kernel.jobs.read` trong phạm vi nút), `GET /api/v1/kernel/jobs/{id}` (`status`, `progress` {done, total},
`result`, `file` {url ký có hạn, size, expires_at}), `POST /api/v1/kernel/jobs/{id}/retry`
(`kernel.jobs.manage`, có audit). File kết quả giữ `AUTOXIDE_JOBS_FILE_RETENTION_DAYS` (mặc định 7). P8
dùng endpoint này cho tiến độ thao tác hàng loạt.

### P16 — Xuất danh sách theo bộ lọc (D-213, A-b8, B-c6)

`POST /api/v1/kernel/exports {resource, params, columns, format: csv}`: `params` kiểm theo schema bộ lọc của
P7, chạy bằng job (P15), ghi CSV qua `autoxide_kernel::csv` (chống CSV injection), xong thì báo qua
`notifications`. Module khai danh sách xuất được qua `Module::exports` (`resource`, cột, permission, hàm đọc
theo lô) — mặc định rỗng (D-038). Thay cho xuất riêng của `forms`, `catalog` (41:237), `orders`.

### P17 — Nhập dữ liệu: dùng module `imports` (bước 56), không làm ở kernel (D-220)

Một cách nhập cho cả dự án: module `imports` của bước 56 (chạy thử trước, ánh xạ cột là dữ liệu có phiên
bản, chạy lại không trùng, lỗi theo dòng và cột, ghi qua `api` của module đích). D-220 mở rộng nó:

- Thêm nguồn **`autoxide`**: định dạng CSV của chính Autoxide, **cùng cột với xuất chung (P16)** — xuất ở
  khách này nhập được ở khách khác hay sau khi sửa trên bảng tính.
- Thêm đích ngoài thương mại: `content` (bài, trang, kèm chuyển hướng 301 như bước 56 đã có), `media` (từ
  URL qua `autoxide_kernel::net`, kiểm loại thật như tải lên), `newsletter` (người đăng ký **chỉ** kèm bằng chứng
  đồng ý đầy đủ; không có thì không nhập — như bước 56 không mang đồng ý quảng cáo cũ).
- Đích ngoài thương mại không phải chờ bước 47: phần nhập `content` / `media` / `newsletter` của `imports` làm
  được khi bước 51 P16 (xuất chung) đã có; phần thương mại vẫn theo điều kiện bắt đầu của bước 56.

### P18 — Danh sách lý do theo khách (D-223)

- `Module::reason_lists() -> Vec<ReasonList>` (mặc định rỗng); `ReasonList { name, defaults: [ReasonDefault {
  code, legal }] }`. Tên danh sách `<module>.<việc>`, mã `snake_case`, nhãn mặc định là i18n key
  `<module>.reason.<việc>.<mã>`. Bảng `kernel_reason_codes` (`tenant_id`, `list`, `code`, `labels` JSON,
  `active`, `position`, `builtin`, `updated_at`; unique (`tenant_id`, `list`, `code`)). Khởi động đồng bộ mã
  mặc định (thêm mới, không xoá).
- `GET /api/v1/kernel/reasons/{list}?include_inactive` (người có permission đọc của module sở hữu danh sách hay
  `kernel.reasons.manage`), `PUT /api/v1/kernel/reasons/{list}/{code}` (`labels`, `active`, `position`),
  `kernel.reasons.manage`. Mã `legal` không tắt được (`kernel.reason_legal`, 409); mã sai định dạng
  `kernel.reason_invalid` (422).
- `ReasonCodes::ensure(tenant, list, code)` cho service (đọc qua cache theo khách, xoá khi ghi). Dòng dữ liệu
  lưu mã, không lưu nhãn — đổi nhãn không sửa lịch sử; mã đã dùng không xoá được, chỉ tắt.

### P19 — Trường tự định nghĩa trên bản ghi (D-235)

Khách thêm ô dữ liệu của riêng mình cho một loại bản ghi (vd "ngày sinh con" cho khách hàng, "mã công trình" cho
đơn, "người phụ trách" cho bài viết) mà không sửa code.

- **Module chọn tham gia:** `Module::custom_field_targets() -> Vec<CustomFieldTarget>` (mặc định rỗng — D-038).
  `CustomFieldTarget { target, read_permission, write_permission, visibilities }` — `target` là tên loại bản ghi
  của sổ đối tượng (`customers.customer`, `orders.order`, `content.entry`…), `visibilities` là mức hiện mà loại đó
  hỗ trợ.
- **Định nghĩa** (`kernel_custom_fields`, cấp khách hàng): `key` (`snake_case`, không đổi sau khi tạo), `data_type`
  (`text`, `long_text`, `integer`, `decimal` + `scale`, `boolean`, `date`, `datetime`, `option`, `options`, `url`,
  `email`, `phone`, `media` (ID `media`), `reference` (loại + ID trong sổ đối tượng)), `labels` / `help` theo ngôn
  ngữ, `required`, `options`, ràng buộc (độ dài, khoảng giá trị — **không** nhận biểu thức chính quy do khách viết),
  `default`, `visibility` (`internal` — chỉ nhân sự; `public` — có trong đọc công khai; `buyer_input` — người dùng
  cuối nhập được, chỉ loại bản ghi khai hỗ trợ, vd checkout của bước 45), `filterable`, `personal_data`, `position`,
  `active`, `version`.
- **Giá trị nằm trên bản ghi** của module sở hữu: cột `custom_fields` (JSON trong `text`) ở bảng của module — đọc
  bản ghi không phải nối bảng. Service của module gọi `CustomFields::validate(tenant, target, incoming, current)`:
  kiểm theo định nghĩa đang có, gom mọi lỗi báo một lần (D-083, đường dẫn `custom_fields.<key>`), trả giá trị đã
  chuẩn hoá. Định nghĩa đọc từ cache theo khách (xoá khi ghi).
- **Lọc:** chỉ trường `filterable` được lọc. Module thêm migration `custom_fields::index_migration("<module>")`
  (như `outbox_migration`) tạo bảng `<module>_custom_index` (`tenant_id`, `record_id`, `field_id`, `value_text`,
  `value_int`; index (`tenant_id`, `field_id`, `value_text`) và (`tenant_id`, `field_id`, `value_int`)); service ghi
  index **cùng transaction** với bản ghi bằng `CustomIndex::write`. Danh sách nhận `cf.<key>=<toán tử>:<giá trị>`
  (`eq`, `in`, `gte`, `lte`, `prefix`), kernel dựng điều kiện `id IN (subquery)` — câu SQL thường, chạy trên cả hai
  database, không dùng toán tử JSON riêng của database nào. Ngày, số thập phân, boolean quy về `value_int`; chữ
  quy về chữ thường bỏ dấu cho `prefix`.
- **API:** `GET/POST /api/v1/kernel/custom-fields?target=`, `GET/PATCH/DELETE …/{id}` (`kernel.custom_fields.manage`).
  Bản ghi nhận và trả `custom_fields: {key: value}`; trong OpenAPI là object có `additionalProperties`, giao diện và
  agent đọc định nghĩa từ endpoint trên để dựng form và chip lọc (P7). Tool MCP `kernel.list_custom_fields`
  (`read_only`).
- **Đổi định nghĩa:** đổi nhãn, thêm lựa chọn, nới ràng buộc thì được; đổi `data_type` hay `key` khi đã có giá trị
  thì không (`kernel.custom_field_type_locked`, 409) — tạo trường mới. Xoá = `active=false` ngay, job dọn giá trị
  và index theo lô.
- **Dữ liệu cá nhân:** trường `personal_data` đi vào xuất / xoá dữ liệu cá nhân của module sở hữu (bước 37) bằng
  hàm trợ giúp của kernel; không ghi giá trị vào log hay event (event chỉ mang tên trường đổi).
- **Dùng lại khắp nơi:** xuất (P16) và nhập (bước 56) có cột `cf.<key>`; tự động hoá (58) đặt điều kiện và gán giá trị;
  sao chép sang khách thật (38) chép định nghĩa.
- **Giới hạn:** hạn mức gói `kernel.custom_fields` (số trường mỗi loại) và `kernel.custom_fields_filterable`; trần
  máy chủ `AUTOXIDE_CUSTOM_FIELDS_MAX` (100 mỗi loại); giá trị mỗi bản ghi tối đa `AUTOXIDE_CUSTOM_FIELDS_MAX_BYTES`
  (16384).
- **Khác thuộc tính sản phẩm (bước 41):** thuộc tính mô tả hàng cho người mua (biến thể, bộ lọc cửa hàng, nguồn cấp
  sàn); trường tự định nghĩa trên `catalog.product` chỉ có mức `internal` (dữ liệu nội bộ của khách).
- Bước 51 khai đích cho `content.entry`; module của bước 41–49, 53, 59–62 khai đích của mình ở bước đó.

### P20 — Nhãn gắn bản ghi (D-235)

- `kernel_tags` (`tenant_id`, `id`, `name`, `normalized` (chữ thường, bỏ dấu), `color` (tên màu trong bảng cố định
  của kernel), `description?`, `created_by`, `created_at`; unique (`tenant_id`, `normalized`)) và `kernel_record_tags`
  (`tenant_id`, `kind`, `record_id`, `tag_id`, `created_by`, `created_at`; unique (`tenant_id`, `kind`, `record_id`,
  `tag_id`); index (`tenant_id`, `tag_id`, `kind`, `record_id`)). Bảng thuộc kernel vì lọc danh sách cần subquery
  trong câu SQL của module (cùng lý do với `kernel_org_closure`).
- Loại bản ghi của sổ đối tượng khai `taggable` (thêm trường, `#[non_exhaustive]`). Gắn / gỡ cần quyền **ghi** bản
  ghi (sổ đối tượng thêm bộ tra quyền ghi); tạo nhãn mới khi gắn được phép trừ khi khách bật cài đặt
  `kernel.tags.restricted` (khi đó cần `kernel.tags.manage`).
- API: `GET/POST /api/v1/kernel/tags`, `PATCH/DELETE /api/v1/kernel/tags/{id}`, `POST …/{id}/merge {into}`
  (`kernel.tags.manage`); `PUT /api/v1/kernel/records/{kind}/{id}/tags {tag_ids}`; thao tác hàng loạt `kernel.tag_add` /
  `kernel.tag_remove` cho mọi loại `taggable` (P8). Danh sách nhận `tags_any=` / `tags_all=` (kernel dựng điều kiện);
  bản ghi trả `tags` — nạp theo lô một lần mỗi trang, không N+1.
- Event `kernel.record_tags_changed {kind, id, added, removed}` cho tự động hoá (58). Nhãn là dữ liệu nội bộ,
  không có trong đọc công khai.
- Giới hạn: hạn mức gói `kernel.tags` (số nhãn mỗi khách), tối đa 50 nhãn mỗi bản ghi.
- **Không phải thứ đã có:** thẻ (tag) của `content` là phân loại **công khai** của bài viết (có trang, có slug) — giữ
  nguyên. Nhãn của `media` (D-094) đã là hợp đồng API — giữ nguyên, không chuyển. Từ bước 51, loại bản ghi mới cần
  nhãn nội bộ dùng P20, không tự làm bảng nhãn riêng.

### P21 — Đổi câu chữ hiển thị theo khách (D-235)

- Module đánh dấu i18n key **đổi được**: `Module::overridable_texts() -> Vec<OverridableText { key, legal }>`
  (mặc định rỗng) — thường là thuật ngữ và câu hướng tới người dùng cuối (vd `orders.term.order`, trạng thái đơn
  hiển thị cho người mua). Mã lỗi (`code`) và câu của lỗi **không** đổi được: đó là hợp đồng với máy và phải nói
  đúng cách sửa. Key `legal` (câu đồng ý, chân trang pháp lý) không đổi được.
- Không gian `ui.*` cho repo giao diện: khách ghi đè câu của giao diện; core chỉ kiểm dạng key, độ dài và giữ —
  tối đa `AUTOXIDE_TEXT_OVERRIDES_UI_MAX` (1000) key.
- `kernel_text_overrides` (`tenant_id`, `locale`, `key`, `text`, `updated_by`, `updated_at`; unique (`tenant_id`,
  `locale`, `key`)). Kiểm khi lưu: chữ thuần (không HTML), tối đa 500 ký tự, tập chỗ điền `{tên}` **trùng khớp** bản
  mặc định (thiếu hay thừa → 422 liệt kê). Cho phép cả ngôn ngữ core chưa có bản dịch nếu khách đã bật ngôn ngữ đó
  (bước 34) — thiếu câu thì về tiếng Anh như luật i18n.
- Đọc: bộ dịch của kernel tra bản của khách trước, rồi bản của module, rồi tiếng Anh. Bản của khách nằm trong
  cache bộ nhớ theo khách (một map, xoá khi ghi) — đường nóng không query.
- API: `GET /api/v1/kernel/texts?locale&prefix&overridden` (key đổi được, câu mặc định, câu của khách),
  `PUT/DELETE /api/v1/kernel/texts/{locale}/{key}` (`kernel.texts.manage`; xoá = về mặc định);
  `GET /api/v1/kernel/public/texts?locale&prefix=ui.` (công khai, ETag) cho giao diện.

### Cài đặt có nối ra ngoài (A-c10, A-c11)

`GET /api/v1/kernel/settings` thêm `last_used_at`, `last_error` cho mỗi cài đặt có nối ra ngoài; `POST
/api/v1/kernel/settings/{name}/test` gọi thử qua trait của module (không trong transaction ghi).

### Permission · Mã lỗi · Cấu hình

P1–P4 không có permission mới (lọc theo permission sẵn có của từng nguồn). Permission mới:
`kernel.views.share` (P7), `kernel.jobs.read`, `kernel.jobs.manage` (P15), `kernel.reasons.manage` (P18),
`kernel.custom_fields.manage` (P19), `kernel.tags.manage` (P20), `kernel.texts.manage` (P21).

Mã lỗi mới: `kernel.navigation_conflict` (lúc khởi động, không ra API), `kernel.find_query_too_short` (422),
`content.embed_unknown_kind` (422), `content.embed_not_found` (422), `kernel.bulk_count_changed` (409, P8),
`approval_required` (202, P12), `kernel.flag_limit_reached` (409, P14), `kernel.reason_legal` (409, P18),
`kernel.reason_invalid` (422, P18), `kernel.custom_field_invalid` (422, P19 — gom lỗi), `kernel.custom_field_unknown`
(422), `kernel.custom_field_type_locked` (409), `kernel.custom_field_not_filterable` (422), `kernel.tag_limit_reached`
(409, P20), `kernel.text_not_overridable` (422, P21), `kernel.text_placeholders_mismatch` (422). Thêm bản dịch tiếng Việt.

Cấu hình: hạn thời gian mỗi nguồn (P2–P4, mặc định 300 ms), `AUTOXIDE_USER_FLAGS_MAX` (500),
`AUTOXIDE_JOBS_FILE_RETENTION_DAYS` (7), `AUTOXIDE_CUSTOM_FIELDS_MAX` (100), `AUTOXIDE_CUSTOM_FIELDS_MAX_BYTES` (16384),
`AUTOXIDE_TEXT_OVERRIDES_UI_MAX` (1000).

## Thứ tự làm

1. P6 (làm được ngay sau 33): `Resource::image`, node `embed` của `content`, test lưu / đọc.
2. P1: kiểu và `Module::navigation`, kiểm xung đột nhóm, endpoint, tool MCP; khai menu cho mọi module
   đang có.
3. P2: `WorkSource`, endpoint hộp việc và số đếm, hạn thời gian, cache; nguồn việc cho `comments`
   (chờ duyệt), `forms` (bài nộp chưa xử lý), `content` (chờ duyệt), `compliance` nếu đã có.
4. P3: `FinderSource`, endpoint; nguồn cho `content`, `media`, `auth` (người dùng).
5. P4: `SummaryMetric`, `definition` và gate kiểm định nghĩa (D-207), endpoint; số liệu cho module đang có.
6. P5: `audit/activity`, `EventInfo::activity`; câu hoạt động cho event đang có.
7. P15 job API chung (P8 và P16 dùng).
8. P7 schema bộ lọc, view đã lưu, bộ lọc nội dung; P16 xuất chung.
9. P8 thao tác hàng loạt; P9 tìm nâng cao và `resolve`.
10. P10 hộp việc theo người, `SetupStep`; P11 `/me/can`.
11. P12 `ActorKind`, loại API key, yêu cầu duyệt đầy đủ; P13 dòng hoạt động và ghi chú.
12. P18 danh sách lý do; cài đặt có nối ra ngoài.
13. P19 trường tự định nghĩa (đích `content.entry`), P20 nhãn, P21 đổi câu chữ.
14. MODULE_GUIDE (EN + VI) mục "khai phần của module cho giao diện" (gồm khai đích trường tự định nghĩa, `taggable`,
    câu đổi được); sinh lại `docs/openapi.json`;
    CHANGELOG; đánh dấu ROADMAP.

P14 đã làm (D-207). P17 làm ở bước 56 (module `imports`).

## Test bắt buộc

Mọi test chạm database qua `TestDatabase::run` (SQLite và Postgres), qua REST / MCP bằng `TestApp`.

- `navigation_an_muc_khi_module_chua_bat`, `navigation_khoa_muc_khi_thieu_tinh_nang`,
  `navigation_theo_permission_o_nut_dang_chon` (hai nút anh em, quyền ở một nút).
- `navigation_hai_module_khai_nhom_khac_nhau_thi_loi_khoi_dong`.
- `work_khong_lo_viec_cua_nhanh_khac` (bài rò rỉ giữa hai nhánh anh em của testkit),
  `work_nguon_cham_khong_lam_hong_ca_hop` (nguồn giả ngủ quá hạn → `incomplete`).
- `find_khong_dau_van_tim_duoc`, `find_khong_tra_ban_ghi_ngoai_quyen`.
- `summary_theo_pham_vi_nut`.
- `activity_chi_hien_ban_ghi_nguoi_goi_doc_duoc`.
- `embed_bo_doi_tuong_khong_con_cong_khai`, `embed_kind_chua_dang_ky_bi_tu_choi`.
- P7–P13 (D-201): `views_params_sai_schema_bi_tu_choi`, `bulk_filter_dem_lech_thi_409`,
  `bulk_moi_ban_ghi_kiem_quyen_rieng`, `resolve_khong_lo_ban_ghi_ngoai_quyen`, `work_snooze_het_han_thi_hien_lai`,
  `can_tra_dung_ly_do_nhu_loi_that`, `agent_goi_lenh_rui_ro_nhan_202_approval_required`,
  `approval_het_han_khong_chay_lenh`, `activity_khong_lo_ban_ghi_ngoai_quyen`.
- P18 (D-223): `ma_mac_dinh_tu_dong_co`, `khach_them_ma_rieng`, `ma_legal_khong_tat_duoc`,
  `ma_da_tat_bi_tu_choi_khi_ghi`, `hai_khach_khong_thay_ma_cua_nhau`.
- P19–P21 (D-235): `truong_tu_dinh_nghia_gom_loi_mot_lan`, `loc_theo_truong_dung_tren_ca_hai_database`,
  `index_ghi_cung_transaction_voi_ban_ghi` (lỗi giữa chừng → không còn dòng index), `doi_kieu_truong_da_co_gia_tri_409`,
  `truong_internal_khong_lo_o_doc_cong_khai`, `truong_du_lieu_ca_nhan_vao_xuat_va_xoa`, `vuot_han_muc_truong_theo_goi`,
  `nhan_trung_ten_khac_dau_la_mot` (`VIP` / `vip`), `gan_nhan_can_quyen_ghi_ban_ghi`, `loc_tags_all_va_tags_any`,
  `danh_sach_nap_nhan_mot_lan_moi_trang` (đếm query), `gop_nhan_chuyen_het_ban_ghi`,
  `cau_cua_khach_thay_cau_mac_dinh`, `cho_dien_lech_bi_tu_choi`, `ma_loi_khong_doi_duoc`, `cau_legal_khong_doi_duoc`,
  `doc_cau_khong_cham_database_khi_cache_nong`, **đồng thời** `hai_nguoi_cung_tao_nhan_cung_ten_chi_mot_dong`; bài rò rỉ
  giữa khách cho cả bốn bảng.
- P14 (đã có, D-207): `flag_ghi_lai_cung_key_la_ghi_de`, `khong_doc_duoc_flag_cua_nguoi_khac`,
  `key_sai_dinh_dang_bi_tu_choi`, `vuot_han_muc_flag_bao_loi_ro`, `xoa_tai_khoan_xoa_flag`,
  `put_dong_thoi_cung_key_chi_mot_dong`, `loc_theo_tien_to_khong_coi_gach_duoi_la_ky_tu_dai_dien`,
  `co_cua_toi_qua_http`; bước 33 thêm `flag_cua_khach_nay_khong_lo_sang_khach_khac`.

## Tiêu chí xong

Mọi mục của bước 51 trong ROADMAP `[x]`; mọi module đang có khai menu, nguồn việc, nguồn tìm và số liệu
phù hợp; `docs/openapi.json` mô tả đủ từng endpoint; `./gate.sh` xanh trên cả hai database.

## Chỗ dễ sai

- **Rò rỉ qua số đếm**: số đếm cũng là dữ liệu; nguồn phải áp phạm vi nút như danh sách.
- **Cache thiếu khách trong khoá**: hai khách cùng một người dùng thấy số của nhau (D-190).
- **Một nguồn chậm kéo cả trang**: luôn có hạn thời gian và `incomplete`.
- **Nhóm điều hướng viết cứng trong kernel**: kernel trung lập với nghiệp vụ; nhóm "Bán hàng" do module
  thương mại khai.
- **Ẩn mục bị khoá**: thiết kế yêu cầu hiện kèm nhãn khoá để người dùng biết nâng gói mở được gì.
- **Lọc trường tự định nghĩa bằng toán tử JSON của database:** chỉ một bên có, và không có index — luôn qua bảng
  `<module>_custom_index`.
- **Index lệch với bản ghi:** ghi index ngoài transaction thì lỗi giữa chừng để lại dữ liệu sai; luôn cùng transaction.
- **Biểu thức chính quy do khách viết:** có thể làm treo máy chủ (ReDoS) — không nhận, chỉ có ràng buộc đơn giản.
- **Câu của khách đổi nghĩa thông báo pháp lý hay lỗi:** chỉ key module cho phép; key `legal` và mã lỗi không đổi được.

## Agent làm bước này tự quyết

Tên chính xác của endpoint và kiểu; mặc định hạn thời gian mỗi nguồn; thời gian cache số đếm. Ghi quyết định.
