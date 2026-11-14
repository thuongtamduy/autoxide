# Bước 51 — Hợp đồng cho khung quản trị và mặt tiền

> Lộ trình: `docs/ROADMAP.md` bước 51 · Thiết kế: `docs/research/ui-design.md` · Quyết định: D-038
> (hàm mới của `Module` có mặc định), D-097 (giao diện là repo riêng), D-123 (sổ đối tượng), D-155 (sổ
> module), D-185, D-200

## Mục tiêu

Giao diện quản trị của thiết kế 2 dựng **menu, hộp việc, tìm kiếm và trang tổng quan từ dữ liệu của
core**, không viết cứng: cùng một giao diện phục vụ cửa hàng độc lập, chuỗi, khách SaaS và sàn chỉ bằng
cấu hình. Module khai phần của mình qua `Module` hay `StartContext` (mặc định rỗng, D-038), kernel gom
lại và lọc theo khách, gói, permission ở nút tổ chức đang chọn. Website công khai nhúng được bản ghi
của module khác vào bài viết mà `content` không biết khái niệm của module đó.

## Bắt đầu khi

| Phần | Bắt đầu | Cần có |
|---|---|---|
| P1 Sổ điều hướng | SAU bước 35 | `Principal` theo nút, `Scope` (34); `Entitlements::{module_enabled, has_feature}` (35) |
| P2 Hộp việc · P3 Tìm xuyên phân hệ · P4 Số liệu tóm tắt | SAU bước 34 | `TenantContext` (33), `Scope` (34); P2 dùng SSE của bước 40 nếu đã có |
| P5 Hoạt động gần đây | SAU bước 34 | module `audit` có cột tổ chức (33–34) |
| P6 Nhúng bản ghi trong nội dung | ngay sau bước 33 | sổ đối tượng (D-123), `content` |

Bước 41–49 khai menu, nguồn việc, nguồn tìm, số liệu của module mình **ngay trong bước đó** nếu bước 51
đã xong phần tương ứng; chưa xong thì bước 51 khai thay cho mọi module đang có. Bước 51 đánh dấu xong khi
mọi module đã có khai báo.

## Phạm vi

Sửa: `crates/kernel/src/{module.rs, module/registry.rs, resource.rs, http/…}`, `modules/audit`,
`modules/content`, mọi module đang có (khai menu và nguồn), `crates/testkit`, `docs/openapi.json`,
`docs/MODULE_GUIDE.md` + `.vi.md`, `CHANGELOG.md`.

File mới: `kernel/src/navigation.rs`, `kernel/src/work.rs`, `kernel/src/finder.rs`,
`kernel/src/summary.rs`, `kernel/src/http/workspace.rs`.

NGOÀI phạm vi: giao diện (repo riêng); tuỳ biến menu theo từng người dùng (ghim, ẩn); trợ lý AI trong
quản trị; full-text search riêng (vẫn là "chứa" của `kernel::search`).

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
}
// đăng ký trong Module::start: context.work_source(Arc<dyn WorkSource>)?
```

- Việc **tính từ dữ liệu của module** (đơn chưa xác nhận, KYC chờ, trang bắt buộc còn thiếu), không có
  bảng việc riêng: xử lý xong bản ghi thì việc tự biến mất. Không có trạng thái "bỏ qua" trong bước này.
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
  không viết riêng ở repo giao diện, để hộp ⓘ "Doanh thu hôm nay tính thế nào?" luôn đúng.
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

### Permission · Mã lỗi

Không permission mới (P1–P4 lọc theo permission sẵn có của từng nguồn). Mã lỗi mới:
`kernel.navigation_conflict` (lúc khởi động, không ra API), `kernel.find_query_too_short` (422),
`content.embed_unknown_kind` (422), `content.embed_not_found` (422). Thêm bản dịch tiếng Việt.

## Thứ tự làm

1. P6 (làm được ngay sau 33): `Resource::image`, node `embed` của `content`, test lưu / đọc.
2. P1: kiểu và `Module::navigation`, kiểm xung đột nhóm, endpoint, tool MCP; khai menu cho mọi module
   đang có.
3. P2: `WorkSource`, endpoint hộp việc và số đếm, hạn thời gian, cache; nguồn việc cho `comments`
   (chờ duyệt), `forms` (bài nộp chưa xử lý), `content` (chờ duyệt), `compliance` nếu đã có.
4. P3: `FinderSource`, endpoint; nguồn cho `content`, `media`, `auth` (người dùng).
5. P4: `SummaryMetric`, endpoint; số liệu cho module đang có.
6. P5: `audit/activity`, `EventInfo::activity`; câu hoạt động cho event đang có.
7. MODULE_GUIDE (EN + VI) mục "khai phần của module cho giao diện"; sinh lại `docs/openapi.json`;
   CHANGELOG; đánh dấu ROADMAP.

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

## Agent làm bước này tự quyết

Tên chính xác của endpoint và kiểu; mặc định hạn thời gian mỗi nguồn; thời gian cache số đếm; có thêm
"bỏ qua việc" (snooze) hay không — nếu thêm thì là bảng cấp khách hàng có `tenant_id`, ghi quyết định.

## Bổ sung theo D-201 (nghiên cứu các trang quản trị hàng đầu)

Bảy mục thêm vào bước 51, làm sau P1–P6, cùng luật: hàm mới của `Module` có mặc định rỗng, chạy và có
test trên cả hai database, áp như nhau cho REST và MCP.

### P7 — Schema bộ lọc và view đã lưu

- Mỗi endpoint danh sách khai **schema bộ lọc** (tên tham số, kiểu, giá trị cho phép, toán tử) bằng một
  phần mở rộng trong OpenAPI (`x-autoxide-filters`), sinh từ cùng kiểu Rust với `IntoParams` — không
  viết hai lần. Giao diện và agent dựng chip lọc từ đó.
- `GET/POST/PATCH/DELETE /api/v1/me/views` (bảng `kernel_saved_views`, cấp khách hàng, có `tenant_id`):
  `resource` (id danh sách, vd `orders.list`), `params` (kiểm theo schema khi lưu — sai thì 422 liệt kê
  từng tham số), `columns`, `group_by`, `density`, `scope` (`personal` / `node` / `tenant`, kèm `org_id`),
  `owner`. View phạm vi nút cần permission `kernel.views.share`. View hệ thống do module khai, không xoá được.

### P8 — Thao tác hàng loạt

- Module khai `BulkAction { id, permission, run }`. `POST /api/v1/kernel/bulk/{resource}/{action}`
  nhận **một trong hai**: `ids` (tối đa 500) hoặc `filter` + `expected_count`; số khớp lúc chạy khác
  `expected_count` thì 409 `kernel.bulk_count_changed`. `Idempotency-Key` bắt buộc.
- Chạy bằng job; mỗi bản ghi kiểm quyền riêng ở nút của nó; kết quả từng bản ghi (`ok` / `skipped`
  kèm lý do / `failed` kèm `code`) đọc ở `GET /api/v1/kernel/jobs/{id}` có phần trăm tiến độ, đẩy qua
  SSE nếu bước 40 đã có.

### P9 — Tìm nâng cao và mở thẳng

- `FinderSource` nhận truy vấn đã phân tích: từ khoá còn lại + toán tử `is:<loại>`, `<trường>:<giá trị>`,
  so sánh `>`, `<`, khoảng `..`, phủ định `-`. Toán tử nào nguồn không hiểu thì bỏ, ghi vào `ignored`.
- Tham số `scope=node|subtree|accessible` cho tìm và cho danh sách; `accessible` = mọi nút người gọi có
  quyền (bước 34 `/me/memberships`).
- `GET /api/v1/kernel/resolve?code=` — module đăng ký bộ nhận dạng mã (số đơn, mã vận đơn, số hoá đơn);
  trả bản ghi đầu tiên người gọi được xem, không trả gì nếu không có quyền (không lộ sự tồn tại).

### P10 — Hộp việc theo người, việc thiết lập

- `WorkItem` có `key` ổn định (`<nguồn>:<id bản ghi>`) và `reason` (permission, nút). Bảng
  `kernel_work_states` (cấp khách hàng): `key`, `user_id`, `snoozed_until`, `assignee_id`. `POST
  /api/v1/me/work/{key}/snooze {until}`, `POST …/assign {user_id}` (người nhận phải có quyền của nguồn).
  Việc tự hết khi dữ liệu không còn khớp; trạng thái mồ côi dọn bằng job.
- `SetupStep` (đăng ký như `WorkSource`): `id`, `label`, `models` (mô hình áp dụng), `blocking`
  (chặn bán), `done(tenant) -> bool` tính từ dữ liệu. `GET /api/v1/kernel/setup` trả danh sách và tiến độ.

### P11 — Giải thích quyền

`POST /api/v1/me/can` nhận tối đa 50 cặp `{permission, org_id}`; mỗi cặp trả `allowed` và khi không:
`reason` (`missing_permission`, `module_not_enabled`, `feature_not_in_plan`, `quota_exceeded`),
`required_feature`, `upgrade_to` (gói rẻ nhất có tính năng), `grantors` (tối đa 5 người cấp được ở nút
đó — chỉ trả cho người gọi có quyền xem thành viên). Kết quả khớp đúng logic kiểm quyền thật: test so
từng lý do với lỗi thật mà endpoint tương ứng trả.

### P12 — Agent là người dùng, lệnh cần duyệt

- API key có `kind`: `human` (mặc định), `agent`, `service`. Audit ghi `actor_kind`; danh sách audit lọc
  được theo nó.
- Module đánh dấu lệnh rủi ro: `Command::requires_approval_for(ActorKind::Agent)`. Agent gọi lệnh đó
  thì nhận `202` mã `approval_required` kèm `approval_request` (`id`, `summary`, `expires_at`, mặc định
  24 giờ, đường dẫn duyệt). Người có cùng permission duyệt (`POST /api/v1/kernel/approvals/{id}/approve`)
  thì lệnh chạy với `Idempotency-Key` gốc; từ chối hay hết hạn thì agent nhận lý do. Bảng
  `kernel_approval_requests` cấp khách hàng. Như nhau cho REST và tool MCP (luật kiến trúc 8). Yêu cầu
  đang chờ vào hộp việc (P2).

### P13 — Dòng hoạt động và ghi chú trên mọi bản ghi

- `GET /api/v1/kernel/records/{kind}/{id}/activity` gộp audit của bản ghi và ghi chú, chỉ khi người gọi
  đọc được bản ghi (hỏi bộ tra của sổ đối tượng, D-123).
- `POST …/notes {body}` — ghi chú nội bộ (bảng `kernel_record_notes`, cấp khách hàng); `@` nhắc người
  có quyền đọc bản ghi, gửi thông báo qua `notifications` nếu đã có. Module `customers` (bước 47) dùng
  chỗ này thay vì tự có ghi chú.

### Test thêm

`views_params_sai_schema_bi_tu_choi`, `bulk_filter_dem_lech_thi_409`, `bulk_moi_ban_ghi_kiem_quyen_rieng`,
`resolve_khong_lo_ban_ghi_ngoai_quyen`, `work_snooze_het_han_thi_hien_lai`, `can_tra_dung_ly_do_nhu_loi_that`,
`agent_goi_lenh_rui_ro_nhan_202_approval_required`, `approval_het_han_khong_chay_lenh`,
`activity_khong_lo_ban_ghi_ngoai_quyen` — trên cả hai database.

## Bổ sung theo D-207 (trợ giúp trong giao diện)

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

Test đã có (cả hai database): `flag_ghi_lai_cung_key_la_ghi_de`, `khong_doc_duoc_flag_cua_nguoi_khac`,
`key_sai_dinh_dang_bi_tu_choi`, `vuot_han_muc_flag_bao_loi_ro`, `xoa_tai_khoan_xoa_flag`,
`put_dong_thoi_cung_key_chi_mot_dong`, `loc_theo_tien_to_khong_coi_gach_duoi_la_ky_tu_dai_dien`,
`co_cua_toi_qua_http`.

### P4 — Định nghĩa số liệu (chưa làm)

Xem `SummaryMetric::definition` và trường `definition` ở mục P4 phía trên; làm cùng P4.

## Bổ sung theo đối chiếu thiết kế 2 (D-208…D-218)

> Nguồn: `docs/prompts/ui-trace-roadmap.md`; mã `A-…` / `B-…` trỏ tới `docs/research/ui-trace/`.

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

### P12 bổ sung — Yêu cầu duyệt đầy đủ (D-213, A-c2)

`GET /api/v1/kernel/approvals?status=`, `GET /api/v1/kernel/approvals/{id}`, `POST …/approve`, `POST
…/reject {reason}`; event `kernel.approval_decided` (agent biết kết quả mà không hỏi lại liên tục); tool MCP
`list_approvals`, `get_approval` (`read_only`), `decide_approval`. `ModuleTool` thêm
`requires_approval_for_agent` (A-c12).

### P7 bổ sung — Bộ lọc nội dung (A-c5)

Bài viết: `review=pending`, `scheduled=true`, `ai_generated=true`, `missing_locale=<locale>`; media:
`missing_alt=true` (kể cả thiếu bản dịch `alt` của một ngôn ngữ, D-202). Khai trong schema bộ lọc như mọi
bộ lọc khác.

### Nhỏ (A-c10, A-c11)

- `DELETE /api/v1/me/work/{key}/snooze` (bỏ hoãn).
- `GET /api/v1/kernel/settings` thêm `last_used_at`, `last_error` cho mỗi cài đặt có nối ra ngoài; `POST
  /api/v1/kernel/settings/{name}/test` gọi thử qua trait của module (không trong transaction ghi).
- P10: hộp việc không có "Đã xử lý" (D-212).
