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
    async fn value(&self, tenant: &TenantContext, scope: &Scope) -> Result<MetricValue, Error>;
}
pub enum MetricValue { Money(Money), Count(u64), Ratio { numerator: u64, denominator: u64 } }
```

- `GET /api/v1/kernel/summary?group&org_id` → các số liệu của nhóm (hoặc của mọi nhóm khi bỏ `group`,
  cho trang Tổng quan), mỗi số kèm `note` tuỳ chọn. Hạn thời gian mỗi số như P2.
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
