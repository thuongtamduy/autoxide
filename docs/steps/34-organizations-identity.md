# Bước 34 — Tổ chức, quyền theo độ sâu, danh tính

> Lộ trình: `docs/ROADMAP.md` bước 34 · Thiết kế: `docs/research/business-foundation.md` trụ 2, mục 7
> (dòng "nhân sự nền tảng") · Quyết định: D-185, D-190, D-192, D-193; nền cũ D-046 (role), D-063 /
> D-162 (đặt lại mật khẩu chạy trong job), D-117 (token dùng một lần), D-181 (`rows`), D-184
> (`reference`), D-038 (hàm mới của `Module` luôn có mặc định).

## Mục tiêu

Xong bước này, trong mỗi khách hàng có **cây tổ chức** (trụ sở → vùng → cửa hàng / kho / người
bán…), quyền có **độ sâu** `own` / `unit` / `subtree` / `tenant` như Dynamics 365, dữ liệu nghiệp vụ
mang `org_id` và service lọc theo nhánh bằng một công cụ của kernel. Danh tính có hai lớp: **nhân sự**
là tài khoản toàn nền tảng làm cho nhiều khách / nút; **người mua** là tài khoản riêng từng khách.
Có danh mục **đơn vị hành chính Việt Nam sau sáp nhập** (module `address`) dùng chung, uỷ quyền
theo cây khách hàng, đăng nhập bằng link, mời người dùng, và nhân sự nền tảng chỉ vào dữ liệu khách
khi có quyền cấp có thời hạn. Phục vụ mô hình 2 (nhiều cửa hàng), 4 (chuỗi cha-con), sàn (người
bán là nút loại `seller`); mô hình 1 và 3 không phải làm gì thêm (cây một nút, mặc định).

## Bắt đầu khi

Bước 33 đã merge (gate đầy đủ xanh). Bước này dựa vào các thứ bước 33 làm ra — **đọc
`docs/steps/33-*.md` và code thật trước**, chỗ nào tên khác thì theo code thật:

- `autoxide_kernel::tenancy`: `TenantId`, `OrgId` (newtype UUIDv7), `TenantContext` (extractor),
  `DatabaseRegistry`, công cụ lọc theo khách cho truy vấn SeaORM; bảng `kernel_tenants` (có khách
  cha), `kernel_tenant_domains` (tên miền → khách / nút), `kernel_placements`; **khách mặc định** ở
  chế độ `single`.
- `Principal` mang tenant hiện tại (`principal.tenant()`); access token mang khách đang chọn.
- Mọi bảng cấp khách hàng có `tenant_id`, unique / index bắt đầu bằng nó; testkit có bài rò rỉ
  giữa khách; `cargo xtask check` biết phân loại bảng (mặt điều khiển / cấp khách hàng).

Giả định cần kiểm khi bắt đầu (nếu bước 33 làm khác, sửa đặc tả này trước khi code, ghi DECISIONS):

1. **Khách vận hành** (operator tenant) = khách mặc định của bước 33; database của nó là database
   mặt điều khiển. Permission của nhân sự nền tảng là permission họ có ở khách này.
2. Có event (hay hook) "khách vừa được tạo" — đặc tả này gọi là `kernel.tenant_created` (payload
   `tenant_id`, `parent_id`). Bước 33 chưa có thì bước này thêm vào module nội bộ `kernel`.
3. Có cách đổi "khách đang chọn" cho access token. Chưa có thì bước này thêm
   `POST /api/v1/auth/me/tenant` (mục 6 dưới).

## Phạm vi

Crate mới: `modules/address` (`autoxide-address`), `modules/organizations` (`autoxide-organizations`).

Sửa: `crates/kernel` (`access.rs`, `tenancy`, `event` — trường `Actor`, `module.rs` — hàm
`org_settings`), `crates/testkit` (`TestApp::builder(…).member(…)`, bài rò rỉ giữa hai nhánh),
`modules/auth` (membership có nút, role có độ sâu, realm, API key gắn nút, uỷ quyền, link đăng nhập,
lời mời, quyền vào hỗ trợ), `modules/notes` (ví dụ `org_id`), `crates/app/src/modules.rs`, lệnh CLI
`autoxide user invite`, `docs/MODULE_GUIDE.md` + `.vi.md`, `docs/DEPLOY.md` + `.vi.md` (cấu hình
mới), `CHANGELOG.md`, `.env.example`, `docs/openapi.json`.

**Thứ tự đăng ký (khác bảng tên chung — xem "Agent tự quyết" #1):** `mail`, `ai`, `files`,
**`address`**, **`organizations`**, `auth`, … — `organizations` cần `dyn AdministrativeUnits` để kiểm
địa chỉ nút; `auth` cần `dyn OrgDirectory` để kiểm nút khi cấp membership, API key, lời mời.
`organizations` không cần gì của `auth`.

NGOÀI phạm vi: kho báo cáo gộp, cha thao tác dữ liệu con bằng job phân phối, tách / gộp nhánh (bước
38); KYC, cờ "đã xác thực số điện thoại" (37); gói, hạn mức (35 — bước này chỉ để chỗ gọi
`reserve`); hồ sơ khách hàng CRM, điểm thưởng (47); sổ địa chỉ người mua (44); hạn mức theo nút
(vd số nhân sự mỗi cửa hàng). `org_id` thêm vào `notes`, và vào `content`, `media` (`forms` tuỳ chọn)
theo mục "Bổ sung theo đối chiếu thiết kế 2" ở cuối file (D-208, thay câu cũ "chỉ `notes`"); `newsletter`,
`comments` giữ dữ liệu cấp khách (quyền của chúng chỉ có độ sâu `tenant`).

## Thiết kế

### Kiểu và trait

Phác thảo — tra API thật bằng skill `check-library-api`, để `cargo check` xác nhận.

```rust
// autoxide_kernel::access (sửa)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Scope { Own, Unit, Subtree, Tenant }           // thứ tự = rộng dần

impl Permission {
    /// Permission áp lên dữ liệu có `org_id`: role cấp được với độ sâu < `tenant`.
    pub const fn scoped(self) -> Self;
    pub fn is_scoped(&self) -> bool;
}

pub enum Realm { Staff, Customer }

pub struct Grant { pub org: OrgId, pub scope: Scope }

/// Đích của một kiểm quyền trên một bản ghi.
pub struct OrgTarget { pub org: OrgId, pub owner: Option<Uuid> }

impl Principal {
    pub fn realm(&self) -> Realm;                         // người mua: không có permission nào
    pub fn grants(&self, permission: &Permission) -> &[Grant]; // rỗng: không có quyền
    /// Hàng rào của API key gắn nút: mọi quyền chỉ trong nhánh này.
    pub fn fence(&self) -> Option<OrgId>;
    pub fn access(&self) -> AccessPath;                   // Direct | Delegation(id) | Support(id)
    /// Điều kiện SeaORM cho danh sách: `Condition::all()` khi có `tenant` (không hàng rào);
    /// lỗi `permission_denied` khi không có grant nào.
    pub fn scope_condition<C: ColumnTrait>(&self, permission: &Permission, org: C,
        owner: Option<C>) -> Result<Condition, Error>;
    /// Kiểm một bản ghi (đọc / sửa / xoá). Cần kết nối để tra nhánh.
    pub async fn require_at<D: ConnectionTrait>(&self, db: &D, permission: &Permission,
        target: OrgTarget) -> Result<(), Error>;          // `org_out_of_scope`
    /// Như trên + nút và mọi tổ tiên đang `active`. Gọi TRONG transaction ghi của module.
    pub async fn require_write_at<D: ConnectionTrait>(&self, tx: &D, permission: &Permission,
        target: OrgTarget) -> Result<(), Error>;          // + `org_frozen`
}
// `require(&P)` giữ nguyên nghĩa: có P ở bất kỳ độ sâu nào. `has_all_permissions` = mọi
// permission ở độ sâu `tenant`. `Principal::new` / `with_all_permissions` giữ (testkit, bản cũ) và
// cho độ sâu `tenant` ở nút gốc.

// autoxide_kernel::tenancy (thêm) — cấu trúc cây là của kernel, xem "Bảng dữ liệu"
pub struct OrgTree;
impl OrgTree {
    pub async fn lock<C>(tx: &C, tenant: TenantId) -> Result<(), Error>;   // khoá dòng gốc
    pub async fn insert_root<C>(tx: &C, tenant: TenantId) -> Result<OrgId, Error>; // id = tenant id
    pub async fn insert<C>(tx: &C, tenant: TenantId, id: OrgId, parent: OrgId, max_depth: u8)
        -> Result<(), Error>;
    pub async fn move_subtree<C>(tx: &C, tenant: TenantId, id: OrgId, new_parent: OrgId,
        max_depth: u8) -> Result<(), Error>;
    pub async fn set_writable<C>(tx: &C, tenant: TenantId, id: OrgId, writable: bool)
        -> Result<(), Error>;
    pub async fn ancestors<C>(db: &C, tenant: TenantId, id: OrgId) -> Result<Vec<OrgId>, Error>;
    pub async fn is_within<C>(db: &C, tenant: TenantId, id: OrgId, roots: &[OrgId])
        -> Result<bool, Error>;
    pub async fn ensure_writable<C>(db: &C, tenant: TenantId, id: OrgId) -> Result<(), Error>;
    pub fn subtree_condition<Col: ColumnTrait>(tenant: TenantId, roots: &[OrgId], column: Col)
        -> Condition;   // column IN (SELECT descendant_id FROM kernel_org_closure WHERE …)
}
/// Khuôn "giá trị kế thừa": giá trị của tổ tiên gần nhất. `ancestors` gần nhất trước.
pub fn nearest<T>(ancestors: &[OrgId], values: impl IntoIterator<Item = (OrgId, T)>)
    -> Option<(OrgId, T)>;

// autoxide_kernel::tenancy — cài đặt theo cây do module khai (API mới, như PermissionCatalog)
pub struct OrgSetting { /* key `<module>.<tên>`, description, default: Option<Value>,
                           validate: fn(&Value) -> Result<Value, String> */ }
// Module::org_settings(&self) -> Vec<OrgSetting> { Vec::new() }
// StartContext::org_settings(&self) -> &OrgSettingCatalog

// autoxide_kernel::event (sửa): Actor thêm `access: AccessPath` (mặc định Direct); outbox thêm
// cột `actor_access` (+ `actor_access_id`) bằng một migration kernel chung, áp như
// `outbox_actor_migration` đã làm.

// autoxide_organizations::api
pub struct Unit { pub id: OrgId, pub tenant_id: TenantId, pub parent_id: Option<OrgId>,
    pub unit_type: String, pub name: String, pub code: Option<String>, pub status: UnitStatus,
    pub depth: u8, pub address: Option<autoxide_address::api::Address> }
pub enum UnitStatus { Pending, Active, Suspended, Archived }
#[async_trait] pub trait OrgDirectory: Send + Sync {
    async fn unit(&self, tenant: TenantId, id: OrgId) -> Result<Option<Unit>, Error>;
    async fn root(&self, tenant: TenantId) -> Result<Unit, Error>;
    async fn ancestors(&self, tenant: TenantId, id: OrgId) -> Result<Vec<Unit>, Error>;
    async fn is_within(&self, tenant: TenantId, id: OrgId, root: OrgId) -> Result<bool, Error>;
    async fn setting(&self, tenant: TenantId, id: OrgId, key: &str)
        -> Result<Option<EffectiveSetting>, Error>;   // giá trị + nút nguồn
}

// autoxide_address::api
pub struct Address { pub country: String, pub province_code: String, pub province_name: String,
    pub commune_code: String, pub commune_name: String, pub line: String }
pub struct AddressInput { pub country: String, pub province_code: String,
    pub commune_code: String, pub line: String }
pub struct AdministrativeUnit { pub code: String, pub name: String, pub kind: UnitKind,
    pub parent_code: Option<String>, pub valid_from: String, pub valid_to: Option<String> }
#[async_trait] pub trait AdministrativeUnits: Send + Sync {
    async fn provinces(&self, country: &str, as_of: Option<&str>) -> Result<Vec<AdministrativeUnit>, Error>;
    async fn communes(&self, province: &str, as_of: Option<&str>) -> Result<Vec<AdministrativeUnit>, Error>;
    /// Kiểm và điền tên; lỗi gắn tiền tố trường `field` (vd `address.commune_code`).
    async fn validate(&self, input: AddressInput, field: &str) -> Result<Address, Vec<FieldError>>;
}

// autoxide_auth::api (thêm) — cho CLI `autoxide user invite` và bước 35 (cấp phát)
#[async_trait] pub trait Invitations: Send + Sync {
    async fn invite(&self, invitation: NewInvitation) -> Result<InvitationSummary, Error>;
}
// `api::User` thêm `realm`, `tenant_id` (hàm tạo + `#[non_exhaustive]` theo D-170).
```

**Nghĩa của độ sâu** (một grant = membership tại nút N với role R, R cấp permission P ở độ sâu S):

| S | Đọc / sửa bản ghi `(org, owner)` | Tạo bản ghi ở nút X |
|---|---|---|
| `own` | `owner = user_id` (ở bất kỳ nút nào của khách) | chỉ X = N, `owner = user_id` |
| `unit` | `org = N` | X = N |
| `subtree` | `org` thuộc nhánh N (gồm N) | X thuộc nhánh N |
| `tenant` | mọi bản ghi của khách | mọi X |

Quyền của người = hợp mọi grant. Permission không `scoped()` chỉ cấp được ở `tenant`, và có hiệu
lực toàn khách dù membership ở nút nào (như privilege cấp tổ chức của Dynamics). API key gắn nút K:
mọi điều kiện thêm `AND org thuộc nhánh K` (hàng rào); key gắn gốc thì không có hàng rào. Ghi vào
nút đang `pending` / `suspended` / `archived` hay có tổ tiên như vậy → `org_frozen`, kể cả với admin.

### Bảng dữ liệu

Kiểu: `uuid`, `string_len`, `timestamp_with_time_zone` (UTC), `big_integer`, `boolean`, `text` —
chạy như nhau trên hai database. **Không dùng `ltree`, không `WITH RECURSIVE`**: bảng tổ tiên –
con cháu (closure table) cho mọi câu tra nhánh một câu có index. Ngày hiệu lực của văn bản lưu
`string_len(10)` dạng `YYYY-MM-DD` (ngày lịch, so chuỗi đúng thứ tự, không lệch múi giờ). Mọi
migration có `down`.

**Kernel (cấp khách hàng, mọi database của đội):**

- `kernel_org_units`: `tenant_id`, `id` (PK cùng `tenant_id`), `parent_id` (null chỉ ở gốc),
  `depth` (`integer`, gốc = 0), `writable` (`boolean`: `false` khi nút không `active`), `updated_at`.
  Index `(tenant_id, parent_id)`.
- `kernel_org_closure`: `tenant_id`, `ancestor_id`, `descendant_id`, `depth` (khoảng cách; dòng
  tự trỏ depth 0). PK `(tenant_id, ancestor_id, descendant_id)`; index `(tenant_id,
  descendant_id, depth)`.
- Nút gốc của khách có `id = tenant_id` (ghi DECISIONS): dữ liệu cấp khách có `org_id = tenant_id`,
  nâng cấp chỉ cần gán `org_id = tenant_id`, và bước 38 tách nhánh thành khách mới giữ được ID.
- Dời nhánh (mọi câu chạy được trên cả hai database): sau `OrgTree::lock`, kiểm `new_parent` không
  thuộc nhánh của `id` (`organizations.cycle`) và `depth(new_parent) + 1 + chiều_cao(nhánh) ≤ max`;
  `DELETE FROM kernel_org_closure WHERE tenant_id = ? AND descendant_id IN (SELECT descendant_id
  … WHERE ancestor_id = :id) AND ancestor_id NOT IN (SELECT descendant_id … WHERE ancestor_id =
  :id)`; `INSERT … SELECT a.ancestor_id, d.descendant_id, a.depth + d.depth + 1 FROM closure a
  CROSS JOIN closure d WHERE a.descendant_id = :new_parent AND d.ancestor_id = :id`; cập nhật
  `depth` của cả nhánh bằng một `UPDATE … WHERE id IN (subquery)`. SQLite không cho subquery đọc
  chính bảng đang `DELETE` trong một số dạng — đọc danh sách nhánh vào bộ nhớ trước nếu cần (nhánh
  ≤ số nút của khách).

**`address` (mặt điều khiển, dùng chung mọi khách — không có `tenant_id`, ghi rõ trong phân loại):**

- `address_dataset`: `id`, `country` (`VN`), `effective_from` (`YYYY-MM-DD`), `title` (vd "QĐ
  19/2025/QĐ-TTg"), `source_url`, `retrieved_on`, `sha256` (unique), `province_count`,
  `commune_count`, `imported_at`.
- `address_unit`: `id` (UUIDv7), `country`, `level` (`integer`: 1 tỉnh, 2 xã), `code`
  (`string_len(10)`), `name`, `name_folded` (`search::fold`), `kind` (`province` / `city` / `ward`
  / `commune` / `special_zone`), `parent_code` (null ở tỉnh), `valid_from`, `valid_to` (null =
  còn hiệu lực), `dataset_id`. Unique `(country, code, valid_from)`; index `(country, level,
  parent_code, valid_to)`, `(country, name_folded)`.

**`organizations` (cấp khách hàng):**

- `organizations_unit`: `tenant_id`, `id` (= `kernel_org_units.id`), `unit_type`, `name`,
  `name_folded`, `code` (null được; mã nội bộ, vd mã cửa hàng), `status` (`pending` / `active` /
  `suspended` / `archived`), `status_reason`, cột địa chỉ theo quy ước của `address`
  (`address_country`, `address_province_code`, `address_province_name`, `address_commune_code`,
  `address_commune_name`, `address_line`, đều null được), `created_by`, `created_at`, `updated_at`.
  Unique `(tenant_id, code)` (chỉ khi `code` khác null — dùng unique trên `(tenant_id, code)` vì cả
  hai database coi NULL là khác nhau); index `(tenant_id, status)`, `(tenant_id, name_folded)`.
- `organizations_unit_type`: `tenant_id`, `code` (`[a-z][a-z0-9_]{0,31}`; `tenant` dành cho gốc),
  `name`, `description`, `created_at`, `updated_at`. PK `(tenant_id, code)`.
- `organizations_setting`: `tenant_id`, `org_id`, `key`, `value` (`text`, JSON đã kiểm),
  `updated_by`, `updated_at`. PK `(tenant_id, org_id, key)`.
- Migration nâng cấp: với mỗi khách đang có trên database này → nút gốc (kernel + `organizations_unit`
  loại `tenant`, tên = tên khách). Khách tạo sau: nghe `kernel.tenant_created`. `OrgDirectory::root`
  tự tạo gốc còn thiếu (dưới `OrgTree::lock`) để chịu được event đến chậm.

**`auth` (sửa; phân loại):**

| Bảng | Lớp | Đổi ở bước này |
|---|---|---|
| `auth_user`, phiên, MFA, passkey, `auth_tokens` | cấp khách hàng | thêm `realm` (`staff` / `customer`, mặc định `staff`); unique `(tenant_id, realm, email)`. Nhân sự là dòng của khách vận hành (database mặt điều khiển); người mua là dòng của khách mình (database của khách) |
| `auth_role`, `auth_role_permission` | mặt điều khiển | role thuộc một khách (`tenant_id`); `auth_role_permission` thêm `scope` (mặc định `tenant`); mỗi khách một role `admin` dựng sẵn (`all_permissions`) |
| `auth_user_role` (= membership) | mặt điều khiển | thêm `org_id`; `tenant_id` = khách mà membership có hiệu lực; unique `(tenant_id, user_id, org_id, role_id)`; index `(tenant_id, org_id)` |
| `auth_api_key` | mặt điều khiển | thêm `org_id` (nút của hàng rào; gốc = không rào) |
| `auth_invitation` (mới) | mặt điều khiển | `tenant_id`, `id`, `email`, `name`, `org_id`, `invited_by`, `status` (`pending` / `accepted` / `revoked`), `expires_at`, `accepted_user_id`, `token_id` (token đang hiệu lực), `created_at`, `updated_at`; index `(tenant_id, status, created_at)`, `(tenant_id, email)` |
| `auth_invitation_role` (mới) | mặt điều khiển | `invitation_id`, `role_id`; PK hai cột |
| `auth_delegation` (mới) | mặt điều khiển | `tenant_id` (khách con, nơi quyền có hiệu lực), `id`, `parent_tenant_id`, `role_id` (role của khách cha), `org_id` (nút ở con, mặc định gốc), `reason`, `granted_by`, `expires_at` (null được), `revoked_at`, `revoked_by`, `created_at`; index `(tenant_id, revoked_at)`, `(parent_tenant_id, role_id)` |
| `auth_support_grant` (mới) | mặt điều khiển | `tenant_id` (khách được vào), `id`, `staff_user_id`, `role_id` (role của khách vận hành), `basis` (`support` / `legal_request`), `reason` (10–500 ký tự), `legal_reference` (số văn bản yêu cầu; bắt buộc khi `legal_request`), `granted_by`, `starts_at`, `expires_at`, `revoked_at`, `created_at`; index `(tenant_id, staff_user_id, expires_at)` |

Nâng cấp (một migration, cả hai database, test nâng cấp của bước 31 phải xanh): mọi `auth_user`
→ `realm = staff`; mọi role và `auth_user_role` thuộc khách mặc định, `org_id = tenant_id` (gốc);
mọi `auth_role_permission.scope = tenant`; mọi API key `org_id = tenant_id`. Kết quả: mọi người giữ
đúng quyền cũ. Bảng `auth_user_role` có foreign key sang `auth_role` / `auth_user` (cùng module, cùng
database mặt điều khiển) — giữ. Nếu bước 33 đã xếp `auth_role` vào lớp cấp khách hàng thì bước này
đổi lớp và chép dòng của khách `dedicated` (nếu có) về mặt điều khiển bằng task
`auth.move_roles` — ghi DECISIONS.

**`notes`:** thêm `org_id` (`uuid`, NOT NULL; dòng cũ = `tenant_id`), index `(tenant_id, org_id)`.

### API REST

Mọi endpoint mới có `///` (summary) và `description` cho từng response; thân JSON bằng `ValidJson`;
danh sách bằng `fetch_page`; tạo trả `Created`. "Scoped" = service lọc bằng `scope_condition` /
`require_at`.

**`/api/v1/address`** (công khai, không cần đăng nhập, `ETag` = phiên bản bộ dữ liệu, `Cache-Control:
public, max-age=86400`):

| Method, path | Trả | Ghi chú |
|---|---|---|
| `GET /provinces?country=VN&as_of=` | `Data<Vec<Unit>>` (34 dòng) | không phân trang (tập nhỏ cố định, như `/auth/permissions`) |
| `GET /provinces/{code}/communes?as_of=` | `Data<Vec<Unit>>` | 404 `address.unit_not_found` |
| `GET /units?q=&province=&level=&limit=&cursor=` | `Page<Unit>` | tìm theo `name_folded` — "ha noi", "Hà Nội" đều ra |
| `GET /units/{code}` | `Data<Unit>` kèm cấp trên | |
| `POST /validate` | `Data<Address>` hoặc 422 | điền tên theo mã; kiểm xã thuộc tỉnh |

**`/api/v1/organizations`** (permission: `organizations.units.read` scoped, `organizations.units.write`
scoped, `organizations.units.manage` scoped, `organizations.types.manage`,
`organizations.settings.write` scoped; với `units.*`, độ sâu `own` được coi như `unit`):

| Method, path | Permission | Ghi chú |
|---|---|---|
| `GET /units?parent_id=&type=&status=&q=` | units.read | scoped trên cột `id` |
| `GET /units/{id}` | units.read | `ETag`; ngoài phạm vi → 404 (không lộ tồn tại) |
| `GET /units/{id}/ancestors` | units.read | gần nhất trước, gồm gốc |
| `POST /units` `{parent_id, unit_type, name, code?, address?}` | units.write tại `parent_id` | `active` nếu người tạo có `units.manage` tại cha, không thì `pending`; gọi `reserve("organizations.units")` khi bước 35 có |
| `PATCH /units/{id}` `{name?, code?, address?}` | units.write | `If-Match` |
| `POST /units/{id}/move` `{parent_id}` | units.write tại nút và tại cha mới | khoá cây của khách |
| `POST /units/{id}/status` `{status, reason}` | units.manage | chuyển hợp lệ: `pending→active`, `active↔suspended`, `active/suspended→archived`, `archived→active`; gốc không đổi được |
| `GET/POST /unit-types`, `PUT/DELETE /unit-types/{code}` | types.manage (đọc: units.read) | xoá loại đang dùng → 409 |
| `GET /settings` | đăng nhập | danh mục khoá (`OrgSettingCatalog`): khoá, mô tả, mặc định |
| `GET /units/{id}/settings` | units.read | giá trị hiệu lực: `{key, value, source_unit_id, inherited}` |
| `PUT /units/{id}/settings/{key}` `{value}` · `DELETE …/{key}` | settings.write | `DELETE` = quay về kế thừa |

Khoá cài đặt do `organizations` khai: `organizations.timezone` (IANA, mặc định `UTC`),
`organizations.currency` (`Currency::parse`), `organizations.language` (`Language` của kernel,
mặc định `en`). `auth` khai `auth.customer_registration` (bool, mặc định `true`) và
`auth.support_access` (bool, mặc định `true`). Hồ sơ thuế do module `tax` khai ở bước 36 bằng
cùng cơ chế; module khác (vd giá ở bước 41) dùng khuôn `OrgTree::ancestors` + `tenancy::nearest`
trên bảng của chính mình.

**`/api/v1/auth`** (thêm; endpoint cũ giữ hình dạng — mọi trường mới là tuỳ chọn):

| Method, path | Permission | Ghi chú |
|---|---|---|
| `GET /permissions` | roles.read | thêm `scoped: bool` |
| `POST/PUT /roles…` | roles.manage | thêm `grants: [{permission, scope}]` cạnh `permissions` (= độ sâu `tenant`); scope < tenant cho permission không scoped → 422 `auth.scope_not_supported` |
| `GET /users/{id}/memberships` · `PUT …` `{memberships: [{org_id, role_ids}]}` | users.read · roles.manage | `PUT /users/{id}/roles` cũ = membership ở gốc; người gán phải có mọi permission của role trong phạm vi nút (`require_covers` mở rộng) |
| `GET /memberships?org_id=&role_id=` | users.read (scoped) | ai làm ở nút nào |
| `GET /me/memberships` · `GET /me/tenants` | đăng nhập | khách / nút người gọi vào được, kèm đường vào (`direct` / `delegation` / `support`) |
| `POST /me/tenant` `{tenant_id}` | đăng nhập | cặp token mới cho khách đã chọn (nếu bước 33 chưa có) |
| `POST /api-keys` | (như cũ) | thêm `org_id` (mặc định gốc; phải trong phạm vi người tạo) |
| `POST /register`, `/login`, `/login/link`, `/password-reset` | công khai | thêm `realm` (mặc định `staff`); `customer` cần ngữ cảnh khách (tên miền / header) và `auth.customer_registration = true` |
| `GET /customers?q=` · `GET/PATCH /customers/{id}` | `auth.customers.read` / `auth.customers.manage` | tài khoản người mua của khách hiện tại (khoá / mở, không xem mật khẩu) |
| `POST /login/link` `{email, realm?, bind_device?}` | công khai, 202 | luôn xếp job; trả `{device_secret}` khi `bind_device` (sinh ngẫu nhiên mọi lần, cả khi email không có) |
| `POST /login/link/confirm` `{token, device_secret?}` | công khai | tokens, hoặc `{mfa_token}` như `/login` |
| `GET /invitations?status=` · `POST /invitations` `{email, name?, org_id, role_ids, reference?}` | `auth.invitations.manage` (scoped) | người mời phải có mọi permission của role tại `org_id` |
| `DELETE /invitations/{id}` · `POST /invitations/{id}/resend` | invitations.manage | resend thu hồi token cũ (`token_id`) |
| `POST /invitations/accept` `{token, name?, password?}` | công khai / đăng nhập | email chưa có tài khoản nhân sự: tạo (email coi là đã xác minh), đặt mật khẩu nếu gửi, trả tokens (đăng ký passkey tiếp bằng `/me/passkeys/registration`); đã có tài khoản: phải đăng nhập đúng tài khoản đó (`auth.invitation_sign_in_required` / `auth.invitation_email_mismatch`), chỉ thêm membership |
| `GET /delegations?direction=incoming\|outgoing` · `POST /delegations` `{child_tenant_id, role_id, org_id?, expires_at?, reason}` · `DELETE /delegations/{id}` | `auth.delegations.read` / `auth.delegations.manage` (ở khách cha) | khách con phải là con cháu trong `kernel_tenants`; con xem được (incoming), chỉ cha hay nền tảng thu hồi |
| `GET /support-access` | `auth.support_access.read` (ở khách) | quản trị của khách xem mọi lần cấp và lần vào |
| `POST /support-grants` `{tenant_id, staff_user_id, role_id, basis, reason, legal_reference?, hours}` · `DELETE /support-grants/{id}` · `GET /support-grants?tenant_id=` | `auth.support.grant` (ở khách vận hành) | khách tắt `auth.support_access` → chỉ `legal_request` được |

**Tính `Principal`** (đường nóng, mặt điều khiển): một câu `auth_user_role ⋈ auth_role ⟕
auth_role_permission` lọc `user_id`, `tenant_id` (như D-076), cộng một câu cho uỷ quyền (role R
người đó giữ **ở gốc** khách cha, có `auth_delegation` còn hiệu lực sang khách hiện tại) và một câu
cho `auth_support_grant` còn hạn — chỉ chạy hai câu sau khi người gọi không có membership trực tiếp
(hay gộp `UNION ALL`, đo rồi chọn). Không tra cây lúc xác thực: grant mang nút gốc, phần nhánh do
`scope_condition` đẩy xuống SQL. Người mua: chỉ kiểm tài khoản, không có permission.

### Tool MCP · Event · Job · Cấu hình · Permission · Mã lỗi

**Tool MCP** (gọi cùng service với REST): `address.list_provinces`, `address.list_communes`,
`address.search_units`, `address.validate_address` (đều `read_only`); `organizations.list_units`,
`get_unit`, `create_unit`, `update_unit`, `move_unit`, `change_unit_status` (`destructive` khi
archive), `get_settings`, `set_setting`; `auth.list_memberships`, `set_memberships`,
`create_invitation`, `list_invitations`, `revoke_invitation`. Không có tool cho link đăng nhập,
uỷ quyền, quyền vào hỗ trợ (việc nhạy cảm chỉ qua REST có người).

**Event** (ghi cùng transaction qua outbox, bên nhận chịu nhận lại): `organizations.unit_created`,
`unit_updated`, `unit_moved` (cha cũ, cha mới), `unit_status_changed`, `unit_type_changed`,
`setting_changed`; `address.dataset_imported`; `auth.memberships_changed`, `auth.invitation_created`,
`invitation_accepted`, `invitation_revoked`, `auth.login_link_used`, `auth.customer_registered`,
`auth.delegation_granted`, `delegation_revoked`, `delegated_access_started` (ghi ở **cả** khách con
và khách cha — hai outbox, có thể hai database: ghi ở con trong transaction cấp token, ở cha qua job
`auth.mirror_event` để chạy lại được), `auth.support_access_granted`, `support_access_started`,
`support_access_revoked` (ghi với `tenant_id` của khách được vào để audit của khách thấy). Mọi event
trong lúc vào bằng uỷ quyền / hỗ trợ mang `Actor.access`.

**Job:** `auth.send_login_link` (như `auth.send_password_reset`, D-162), `auth.send_invitation`,
`auth.mirror_event`. Job dọn `auth.cleanup` có sẵn xoá thêm lời mời hết hạn > 90 ngày và grant hỗ
trợ đã hết hạn > thời hạn lưu audit (chỉ dòng grant, event audit giữ).

**Cấu hình (`Setting`, sinh lại `.env.example`):** `AUTOXIDE_ORGANIZATIONS_MAX_DEPTH` (8; 2–32),
`AUTOXIDE_AUTH_LOGIN_LINK` (`false`), `AUTOXIDE_AUTH_LOGIN_LINK_URL` (trang có `{token}`; không đặt
thì mail chứa token), `AUTOXIDE_AUTH_LOGIN_LINK_MINUTES` (15; 5–60), `AUTOXIDE_AUTH_INVITATION_URL`,
`AUTOXIDE_AUTH_INVITATION_DAYS` (7; 1–30), `AUTOXIDE_AUTH_SUPPORT_ACCESS_MAX_HOURS` (8; 1–72).

**Permission mới:** `organizations.units.read` / `.write` / `.manage` (scoped),
`organizations.types.manage`, `organizations.settings.write` (scoped); `auth.invitations.manage`
(scoped), `auth.customers.read`, `auth.customers.manage`, `auth.delegations.read`,
`auth.delegations.manage`, `auth.support_access.read`, `auth.support.grant`; `notes.read` /
`notes.write` thành scoped.

**Mã lỗi** (thêm bản dịch vào `locales/vi.json` của crate tương ứng; kernel ở locales của kernel):

| Mã | HTTP | Câu tiếng Việt |
|---|---|---|
| `org_out_of_scope` | 403 | Nút `{org}` nằm ngoài phạm vi quyền `{permission}` của bạn. |
| `org_frozen` | 403 | Nút `{org}` (hoặc một nút cấp trên) đang chờ duyệt, tạm khoá hay đã lưu trữ nên không ghi được. |
| `organizations.unit_not_found` | 404 | Không có nút tổ chức `{id}`. |
| `organizations.unit_code_taken` | 409 | Mã `{code}` đã dùng cho một nút khác. |
| `organizations.unit_type_not_found` | 404 | Không có loại nút `{code}`. |
| `organizations.unit_type_in_use` | 409 | Loại nút `{code}` đang được dùng; đổi loại các nút đó trước. |
| `organizations.depth_exceeded` | 422 | Cây tổ chức chỉ sâu tối đa {max} cấp. |
| `organizations.cycle` | 422 | Không dời được một nút vào chính nó hay nhánh con của nó. |
| `organizations.root_immutable` | 409 | Không dời, khoá hay lưu trữ được nút gốc của khách hàng. |
| `organizations.invalid_transition` | 409 | Không chuyển được nút từ `{from}` sang `{to}`. |
| `organizations.setting_unknown` | 404 | Không có cài đặt `{key}`; xem `GET /api/v1/organizations/settings`. |
| `organizations.setting_invalid` | 422 | Giá trị của `{key}` không hợp lệ: {reason} |
| `address.unit_not_found` | 404 | Không có đơn vị hành chính mã `{code}`. |
| `address.commune_not_in_province` | 422 | Xã / phường `{commune}` không thuộc tỉnh / thành `{province}`. |
| `address.country_unsupported` | 422 | Chưa có danh mục đơn vị hành chính cho quốc gia `{country}`. |
| `auth.scope_not_supported` | 422 | Permission `{permission}` chỉ cấp được ở phạm vi toàn khách hàng. |
| `auth.membership_org_invalid` | 422 | Nút `{org}` không tồn tại hoặc không thuộc khách hàng này. |
| `auth.tenant_not_available` | 403 | Bạn không có quyền vào khách hàng `{tenant}`. |
| `auth.login_link_disabled` | 404 | Đăng nhập bằng link chưa được bật trên máy chủ này. |
| `auth.login_link_invalid` | 401 | Link đăng nhập sai, đã dùng hoặc đã hết hạn; hãy xin link mới. |
| `auth.invitation_not_found` | 404 | Không có lời mời `{id}`. |
| `auth.invitation_invalid` | 400 | Lời mời sai, đã dùng, đã bị thu hồi hoặc đã hết hạn. |
| `auth.invitation_sign_in_required` | 409 | Email này đã có tài khoản: đăng nhập rồi chấp nhận lời mời. |
| `auth.invitation_email_mismatch` | 403 | Lời mời gửi cho email khác với tài khoản bạn đang dùng. |
| `auth.customer_registration_closed` | 403 | Cửa hàng này chưa mở đăng ký tài khoản người mua. |
| `auth.delegation_not_found` | 404 | Không có uỷ quyền `{id}`. |
| `auth.delegation_not_descendant` | 422 | Khách `{tenant}` không phải khách con của khách hiện tại. |
| `auth.support_access_disabled` | 403 | Khách hàng đã tắt quyền vào để hỗ trợ; chỉ yêu cầu của cơ quan có thẩm quyền được cấp. |
| `auth.support_grant_not_found` | 404 | Không có quyền vào hỗ trợ `{id}`. |
| `auth.support_duration_exceeded` | 422 | Quyền vào hỗ trợ dài tối đa {max} giờ. |
| `notes.org_required` | 422 | Bạn làm ở nhiều nút: hãy chọn `org_id` cho ghi chú. |

## Thứ tự làm

Mỗi dòng một commit (hoặc vài commit nhỏ), `./gate.sh --quick` trước mỗi commit, đánh dấu ROADMAP khi
mục xong:

1. **Kernel: độ sâu và cây.** `Scope`, `Permission::scoped`, `Grant`, `OrgTarget`, `Realm`,
   `AccessPath`; `Principal::{grants, fence, scope_condition, require_at, require_write_at}`;
   `kernel_org_units`, `kernel_org_closure`, `tenancy::OrgTree`, `tenancy::nearest`;
   `Module::org_settings` + `OrgSettingCatalog`; `Actor.access` + migration outbox; mã lỗi
   kernel. Testkit: `.member(token, user, org, &[("notes.read", Scope::Unit)])`. Ghi DECISIONS
   (độ sâu; gốc = tenant id; cấu trúc cây là của kernel). → ROADMAP mục 1, 2.
2. **Module `address`** (không phụ thuộc gì): bảng, task `address.import`, dữ liệu VN gốc, REST, MCP,
   trait `AdministrativeUnits`, kiểu `Address`, doctor. → mục 5.
3. **Module `organizations`**: loại nút, nút, vòng đời, dời nút, cài đặt kế thừa, REST, MCP, event,
   gốc cho khách cũ và khách mới; đăng ký trước `auth`. → mục 1, 3.
4. **`auth`: membership có nút, role có độ sâu, API key gắn nút**, migration nâng cấp, `Principal`
   mới, endpoint membership. → mục 2, 4 (phần nâng cấp, API key).
5. **`notes` làm ví dụ** + bài rò rỉ giữa hai nhánh anh em trong testkit, chạy cho `notes` và
   `organizations`; hướng dẫn viết module (EN + VI): `org_id`, độ sâu, khuôn kế thừa. → mục 2, 3.
6. **Danh tính hai lớp**: `realm`, người mua theo khách, `/me/tenants`, đổi khách đang chọn,
   endpoint `customers`, role `admin` cho mỗi khách mới (nghe `kernel.tenant_created`). → mục 4.
7. **Uỷ quyền theo cây khách hàng.** → mục 6.
8. **Đăng nhập bằng link.** → mục 7.
9. **Mời người dùng** + `autoxide user invite --email … --role … [--org …] [--tenant …]` (qua
   `api::Invitations`). → mục 8.
10. **Quyền vào hỗ trợ của nhân sự nền tảng.** → mục 9.
11. Đo đường nóng (`scripts/loadtest.sh`, `GET /auth/me` bằng access token và API key; trước bước 4 và
    sau bước 10), ghi số đo vào DECISIONS; `docs/openapi.json`; `CHANGELOG.md`; tài liệu deploy; gate
    đầy đủ; phát hành (`docs/RELEASING.md`).

## Test bắt buộc

Mọi test chạy trên SQLite và Postgres (`TestDatabase::run`, `TestApp::builder`); không bỏ test cho
database nào.

- Kernel: `do_sau_quyet_dinh_dieu_kien_loc` (bảng nghĩa ở trên, từng ô), `hang_rao_api_key_cat_quyen`,
  `nut_bi_khoa_thi_ca_nhanh_khong_ghi_duoc`, `permission_khong_scoped_co_hieu_luc_toan_khach`,
  `doi_nhanh_cap_nhat_bang_to_tien` (so closure với cây tính lại từ `parent_id` sau 200 lần dời ngẫu
  nhiên), `khong_tao_duoc_vong`, `vuot_do_sau_bi_tu_choi`, `nearest_lay_to_tien_gan_nhat`.
- **Đồng thời:** `doi_nut_cheo_nhau_dong_thoi_khong_tao_vong` (A→dưới B và B→dưới A cùng lúc: đúng
  một bên thành công, cây hợp lệ); `khoa_nut_va_ghi_du_lieu_dong_thoi` (sau khi lệnh khoá commit,
  không lệnh ghi nào vào nhánh thành công); `chap_nhan_loi_moi_dong_thoi` (16 request cùng token:
  một thành công); `link_dang_nhap_dung_mot_lan_dong_thoi`; `gan_membership_dong_thoi`.
- **Rò rỉ:** testkit thêm `assert_no_branch_leak(app, module_spec)` — cây gốc → A, B (anh em) → A1;
  người có `subtree` ở A ghi, người có `subtree` ở B đọc danh sách / `GET` từng bản ghi / tool MCP:
  không thấy gì (404, danh sách rỗng); người có `unit` ở A không thấy A1; người có `own` chỉ thấy bản
  ghi mình. Chạy cho `notes` và `organizations`. Bài rò rỉ giữa khách của bước 33 chạy cho
  `organizations` và `address` (address là dữ liệu chung: kiểm khách B không ghi được).
- Nâng cấp: database ở phiên bản trước bước 34 có user, role, API key, ghi chú → migrate → mọi người
  giữ đúng permission cũ (so `GET /auth/me` trước / sau), API key vẫn chạy, `PUT /users/{id}/roles`
  cũ vẫn chạy.
- `address`: `du_lieu_goc_du_34_tinh_3321_xa`, `xa_nao_cung_thuoc_mot_tinh_co_that`,
  `tim_khong_dau_ra_co_dau`, `nhap_lai_cung_bo_khong_doi_gi`, `bo_moi_dong_don_vi_cu_va_mo_don_vi_moi`
  (bộ thử nhỏ trong test), `etag_tra_304`, `validate_bao_xa_khong_thuoc_tinh`.
- `auth`: `nguoi_mua_hai_khach_cung_email_la_hai_tai_khoan`, `nguoi_mua_khong_co_permission_nao`,
  `uy_quyen_cha_vao_con_va_audit_hai_phia`, `uy_quyen_khong_cho_khach_khong_phai_con`,
  `thu_hoi_uy_quyen_co_hieu_luc_request_ke`, `ho_tro_het_han_thi_mat_quyen`,
  `khach_tat_ho_tro_chi_con_yeu_cau_co_tham_quyen`, `link_dang_nhap_tra_loi_nhu_nhau_du_email_khong_co`
  (cùng mã, cùng thân, cùng số câu ghi — như test của D-162), `link_dang_nhap_van_hoi_mfa`,
  `link_gan_thiet_bi_sai_secret_bi_tu_choi`, `loi_moi_het_han_thu_hoi_gui_lai`,
  `nguoi_moi_khong_cap_duoc_quyen_minh_khong_co`, `api_key_ngoai_nhanh_bi_tu_choi`,
  `admin_moi_khach_khong_vao_khach_khac`.

## Tiêu chí xong

- [ ] Mọi mục của bước 34 trong ROADMAP là `[x]`; gate đầy đủ xanh trên SQLite và Postgres.
- [ ] `cargo xtask check` xanh: bảng mới đã phân loại; `code` mới có bản dịch; crate mới có
      `license.workspace` và `[lints] workspace = true`, chỉ mở `api` và `module()`.
- [ ] `docs/openapi.json` sinh lại; endpoint mới đủ mô tả; gate chặn thay đổi phá vỡ (bước 31) xanh
      — không trường nào cũ đổi nghĩa.
- [ ] `.env.example` sinh lại; `docs/DEPLOY.md` + `DEPLOY.vi.md` có link đăng nhập, lời mời, độ sâu
      cây; `docs/MODULE_GUIDE.md` + `.vi.md` có mục "Dữ liệu theo nút" (`org_id`, `scoped()`,
      `scope_condition`, `require_write_at`, khuôn kế thừa, bài rò rỉ nhánh) — hai bản cùng nội dung.
- [ ] `modules/address/data/` có bộ dữ liệu VN kèm manifest (nguồn, URL, ngày tải, SHA-256, số đơn
      vị); `autoxide doctor` báo bộ dữ liệu đang dùng và cảnh báo khi bản đóng gói mới hơn bản đã nhập.
- [ ] DECISIONS có mục cho: độ sâu và nghĩa từng mức; cây là của kernel, gốc = tenant id; phân loại
      bảng `auth` và realm; uỷ quyền; quyền vào hỗ trợ (kèm căn cứ pháp lý); link đăng nhập; lời mời;
      nguồn dữ liệu hành chính; thứ tự đăng ký; số đo đường nóng.
- [ ] `CHANGELOG.md` `[Unreleased]` có dòng Added / Changed (vd `notes` thành dữ liệu theo nút).
- [ ] Đánh dấu ROADMAP, commit, phát hành theo `docs/RELEASING.md` (D-171).

## Chỗ dễ sai

- **NULL trong unique:** Postgres và SQLite đều coi hai NULL là khác nhau — unique `(tenant_id, code)`
  cho phép nhiều nút không mã (đúng ý), nhưng đừng dùng NULL để nghĩa "toàn khách" trong khoá unique
  của membership; vì vậy gốc có ID thật (`= tenant_id`).
- **Closure table trên SQLite:** `DELETE … WHERE x IN (SELECT … FROM cùng_bảng)` chạy được nhưng
  phải xét điều kiện trước khi xoá dòng nào — viết test so với cây tính lại. Danh sách `IN` dài:
  SQLite giới hạn 32 766 tham số, Postgres 65 535 — luôn dùng subquery qua closure, không đổ ID ra
  bộ nhớ rồi `IN (…)`.
- **Deadlock khi dời nút:** luôn `OrgTree::lock` (khoá dòng gốc của khách) TRƯỚC mọi dòng khác; dời
  nút và đổi trạng thái của cùng khách nối đuôi nhau. Không khoá theo thứ tự nút.
- **Đóng băng phải kiểm trong transaction ghi của module** (`require_write_at(&tx, …)`), không kiểm
  trước rồi mới mở transaction — có khe giữa hai lúc. Job / event ghi dữ liệu theo nút không có
  `Principal`: gọi `OrgTree::ensure_writable(&tx, …)` trực tiếp.
- **404 thay vì 403 khi đọc ngoài phạm vi** — không lộ bản ghi tồn tại; 403 `org_out_of_scope` chỉ
  khi tạo / dời vào nút người gọi tự nêu.
- **Thời gian trả lời của link đăng nhập:** request chỉ sinh `device_secret` ngẫu nhiên và xếp job
  cho mọi email — không tra tài khoản trong request (D-154, D-162). `device_secret` lưu dạng băm
  trong `data` của token.
- **Lời mời và nâng quyền:** người có `auth.invitations.manage` không được mời kèm role có permission
  mình không có (như `require_covers`), và không mời vào nút ngoài phạm vi của mình.
- **Uỷ quyền và hỗ trợ không xếp chồng ngầm:** người vừa có membership trực tiếp vừa có uỷ quyền thì
  grant là hợp; `Actor.access` ghi `Direct` khi có membership trực tiếp ở khách đó (ghi rõ trong
  DECISIONS). Hết hạn / thu hồi có hiệu lực ở request kế (không cache grant quá một request).
- **Event hai phía có thể khác database:** không mở hai transaction lồng nhau; bên cha đi qua job,
  job chịu chạy lại (mã tham chiếu = ID event gốc).
- **Đổi realm của email:** cùng email có thể là nhân sự và người mua ở cùng khách (chế độ `single`);
  mọi câu tra theo email phải lọc cả `tenant_id` và `realm`.
- **Bản dữ liệu hành chính:** mã xã 5 chữ số, mã tỉnh 2 chữ số — lưu chuỗi (giữ số 0 đầu). Không
  sinh dữ liệu bằng AI hay chép từ trang không chính thức.

## Căn cứ pháp lý

- Đơn vị hành chính hai cấp: Nghị quyết 202/2025/QH15 (sắp xếp đơn vị hành chính cấp tỉnh), Luật Tổ
  chức chính quyền địa phương 2025 — chính quyền hai cấp từ 01/7/2025; **QĐ 19/2025/QĐ-TTg** ban
  hành Bảng danh mục và mã số đơn vị hành chính Việt Nam, hiệu lực 01/7/2025: 34 tỉnh / thành
  (28 tỉnh, 6 thành phố trực thuộc trung ương) và 3.321 đơn vị cấp xã (theo các nguồn tổng hợp:
  2.621 xã, 687 phường, 13 đặc khu — **cần đối chiếu văn bản gốc**). Nguồn dữ liệu: văn bản gốc trên
  Cơ sở dữ liệu quốc gia về văn bản pháp luật / Công báo, và danh mục điện tử của cơ quan thống kê
  (trang "Danh mục hành chính Việt Nam") — agent kiểm URL hiện hành, ghi URL, ngày tải, SHA-256 vào
  manifest. Danh mục là văn bản hành chính, không được bảo hộ quyền tác giả (Luật SHTT Điều 15 — cần
  đối chiếu, D-193).
- Nhân sự nền tảng vào dữ liệu khách: Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15 (hiệu lực 01/01/2026)
  và NĐ 356/2025/NĐ-CP — xử lý dữ liệu cá nhân phải có căn cứ, có mục đích, để lại vết; cung cấp dữ
  liệu cho cơ quan có thẩm quyền theo yêu cầu bằng văn bản (lưu số văn bản) — **cần đối chiếu điều
  khoản cụ thể** về trường hợp xử lý không cần đồng ý và nghĩa vụ ghi nhật ký.
- Email mời, link đăng nhập là thư giao dịch, không phải quảng cáo (NĐ 91/2020 không áp cho thư giao
  dịch — cần đối chiếu khi bước 37 dựng `notifications`).

## Agent làm bước này tự quyết

Ghi DECISIONS cho mỗi lựa chọn có ảnh hưởng lâu dài (skill `record-decision`).

1. **Thứ tự đăng ký `address` → `organizations` → `auth`** (khác bảng tên chung "auth → organizations
   → address"). Khuyến nghị giữ như đặc tả. Nếu muốn giữ `auth` trước, phải thêm một tay cầm gắn
   muộn trong kernel cho `OrgDirectory` (như `ResourceDirectory`) — phức tạp hơn.
2. **Cấu trúc cây ở kernel** (`kernel_org_units`, `kernel_org_closure`) thay vì bảng
   `organizations_closure`: lọc theo nhánh cần subquery trong câu SQL của module; module không được
   đọc bảng của module khác (luật 1), còn kernel thì được. `organizations` giữ phần nghĩa (tên, loại,
   trạng thái, địa chỉ, cài đặt). Khuyến nghị giữ.
3. Gộp ba câu tính `Principal` thành một `UNION ALL` hay giữ riêng — đo bằng `scripts/loadtest.sh`.
4. Kiểm múi giờ IANA: thêm `chrono-tz` (ghi `STACK.md`) hay chỉ kiểm dạng `Khu_vực/Thành_phố` —
   khuyến nghị `chrono-tz` (chuẩn, cùng hệ `chrono` đang dùng).
5. Thứ tự hiển thị đơn vị hành chính (theo mã hay theo tên) — khuyến nghị theo mã (thứ tự của văn bản).
6. Người mua đăng ký có cần xác minh email trước khi đăng nhập không — khuyến nghị không chặn, đánh
   dấu `email_verified_at` như tài khoản thường; bước 37 thêm cờ xác thực số điện thoại.
7. Thêm `Permission::scoped_without_own()` hay coi `own` = `unit` cho permission trên chính nút —
   khuyến nghị cách sau (đơn giản, ghi trong mô tả permission).

## Bổ sung theo D-200 (thiết kế giao diện 2)

**Hồ sơ công khai của nút.** Module `organizations` thêm cho mỗi nút: `public` (mặc định tắt), tên hiển thị,
`Address`, giờ mở cửa theo thứ trong tuần (và ngày nghỉ đặc biệt), `pickup` (nhận hàng tại quầy), tên miền
gắn với nút. `GET /api/v1/organizations/public/stores` (công khai; khách suy từ tên miền như mọi endpoint
công khai) trả các nút `public` của khách, sắp theo thứ tự khách đặt, `ETag` / `Cache-Control` như
`/api/v1/address`. Không trả thành viên, cài đặt nội bộ hay nút không công khai. Test: nút không công khai
không lộ; hai khách cùng database không thấy cửa hàng của nhau.

## Bổ sung theo D-202 (tạo tài khoản từ số đã xác thực)

`auth` thêm `POST /api/v1/auth/phone-signups {lookup_token, display_name?}` (công khai, rate limit theo IP):
lấy số điện thoại đã qua OTP từ token xem đơn của `orders` (trait `OrderLookups` do `orders` công bố ở
bước 45 — đăng ký `auth` trước thì lấy qua chỗ giữ của kernel hay `Subscription::optional`, agent bước 45
chọn), tạo tài khoản trong khách với số đó đã xác thực, đăng nhập bằng passkey (bước 12) hay OTP; không bắt
buộc mật khẩu. Số đã thuộc một tài khoản → 409 `auth.phone_taken` (gợi ý đăng nhập). Phần này làm ở bước
45 khi `orders` có token xem đơn; ghi ở đây vì danh tính và đăng nhập thuộc `auth`.

## Bổ sung theo đối chiếu thiết kế 2 (D-208…D-218)

> Nguồn: `docs/prompts/ui-trace-roadmap.md`; mã `A-…` / `B-…` trỏ tới `docs/research/ui-trace/`.

- **Nội dung theo website và nút (D-208, A-b1, A-c3).** Thay câu "`org_id` chỉ thêm vào `notes`…" ở mục
  Phạm vi: `content` và `media` thêm `org_id` (nút sở hữu, mặc định nút gốc khi nâng cấp), `forms` thêm
  `org_id` NULL được; quyền `content.*`, `media.*`, `forms.*` thành `scoped()`; đọc công khai lọc theo
  website (tên miền → nút qua `kernel_tenant_domains.org_id`, bài của nút và nhánh trên được chia sẻ xuống
  thì hiện); menu và ngôn ngữ bật theo website; bộ lọc quản trị `owner_org`, `site`. `newsletter`,
  `comments` vẫn cấp khách. Media riêng của người bán trên sàn dùng chính mục này. Test:
  `bai_cua_cua_hang_a_khong_hien_tren_website_cua_cua_hang_b`, `nguoi_ban_chi_thay_media_cua_minh`.
- **Xác nhận lại danh tính (A-b6, A-c4).** `POST /api/v1/auth/me/reauth {method: passkey|totp|email_link}`
  trả token ngắn hạn (`AUTOXIDE_AUTH_REAUTH_MINUTES`, mặc định 10) dùng một lần cho lệnh nhạy cảm (tạo API
  key, đổi email, tắt MFA, đổi role admin) thay cho `current_password` — tài khoản chỉ có passkey cũng làm
  được. Bước 54 (P5, step-up) dùng chính token này.
- **Ngôn ngữ và múi giờ ưa thích (A-b4).** `auth_user.locale`, `auth_user.time_zone` (IANA), `PATCH
  /api/v1/auth/me`; mail và `notifications` gửi chậm (job) dùng hai giá trị này thay cho ngôn ngữ của
  request gốc. Thay mục đã hoãn ở D-062.
- **Khách tự thu hồi quyền vào hỗ trợ (D-211, A-b5).** `DELETE /api/v1/auth/support-access/{id}`
  (`auth.support_access.revoke` ở khách); trần giờ giữ như cũ; lần cấp `legal_request` không thu hồi được.
- **Nhỏ (A-c11).** `GET /api/v1/auth/memberships?org_id&inherited=true` (membership hiệu lực kể cả kế thừa từ
  nút trên). **Thấp (A-b10):** chính sách bắt buộc MFA theo role (`auth_roles.requires_mfa`),
  `auth_user.last_login_at`.
