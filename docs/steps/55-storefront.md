# Bước 55 — Giao diện cửa hàng qua API: banner, bộ sưu tập, thông tin cửa hàng

> Lộ trình: `docs/ROADMAP.md` bước 55 · Luật chung: `docs/steps/README.md` · Quyết định: D-219, D-026
> (core chỉ có API), D-191 (module thương mại), D-195 / D-198 (luật chung giữa các bước)

## Mục tiêu

Website và app của khách tự vẽ giao diện; core không có trang quản trị hay giao diện (D-026). Bước này cho
giao diện **dữ liệu** để dựng trang chủ, trang danh mục, chân trang:

- banner theo vị trí, có lịch;
- bộ sưu tập sản phẩm;
- thông tin cửa hàng công khai.

Đổi banner hay bộ sưu tập là đổi dữ liệu qua API hay MCP, không sửa code giao diện. Phục vụ mọi mô hình:

- **Khách độc lập:** một bộ banner.
- **Chuỗi:** banner chung của chuỗi và banner riêng của từng cửa hàng.
- **Sàn:** banner của sàn; banner trong gian hàng của người bán để sau, xem "Agent làm bước này tự quyết".

## Bắt đầu khi

- **Bước 41 xong:** `catalog` có sản phẩm, danh mục, slug, đọc công khai.
- **Banner và thông tin cửa hàng** chỉ cần bước 34 (cây tổ chức, `Address`, cài đặt kế thừa). Làm được
  trước bước 41 nếu có người rảnh; khi đó link tới sản phẩm hay bộ sưu tập chờ `catalog` gắn vào điểm cắm.
- **Bước 37:** dấu `ai_assisted`, thông tin chủ sở hữu và logo Bộ Công Thương cho chân trang. Thiếu bước 37
  thì bỏ hai phần đó, không chặn bước này.

## Phạm vi

- **`content`:**
  - bảng vị trí và banner, API quản trị và công khai, tool MCP;
  - điểm cắm `ContentLinks` trong `content::api` (thêm `pub mod api` nếu bước 37 chưa thêm).
- **`catalog`:**
  - bảng bộ sưu tập, API quản trị và công khai, tool MCP;
  - gắn bộ dựng link cho sản phẩm và bộ sưu tập vào `ContentLinks`;
  - số bán của sản phẩm cho kiểu sắp "bán chạy".
- **`organizations`:** hồ sơ công khai của nút (thông tin cửa hàng, giờ mở cửa) và endpoint công khai.
- **`promotions`:** gắn bộ dựng link chương trình vào `ContentLinks`; phát event khi chương trình kết thúc
  (nếu bước 43 chưa có).
- **Ngoài phạm vi:**
  - giao diện, theme, trình kéo thả trang (D-026);
  - đo lượt xem / bấm banner (ở "Để sau" của lộ trình);
  - A/B test;
  - banner trong gian hàng người bán của sàn (ghi quyết định nếu làm).

## Thiết kế

### Kiểu và trait

Phác thảo — tra API thật bằng skill `check-library-api` và code của bước 33–43 trước khi viết.

```rust
// autoxide_content::api — điểm cắm cho module đăng ký SAU content (README, "Điểm cắm")
pub enum LinkKind { Url, Entry, Term, Product, Collection, Promotion }

#[async_trait::async_trait]
pub trait LinkTarget: Send + Sync {
    /// Loại mà bộ này lo (Product, Collection hoặc Promotion).
    fn kind(&self) -> LinkKind;
    /// Kiểm khi lưu banner: đối tượng có tồn tại trong khách không.
    async fn exists(&self, tenant: &TenantContext, id: Uuid) -> Result<bool, Error>;
    /// Khi đọc công khai: đường dẫn trong site và còn hiển thị được không (vd sản phẩm ngừng bán,
    /// chương trình hết hạn → None). Nhận nhiều ID một lần, không gọi từng cái.
    async fn resolve(&self, tenant: &TenantContext, ids: &[Uuid], locale: &str, at: DateTime<Utc>)
        -> Result<HashMap<Uuid, ResolvedLink>, Error>;
}
pub struct ResolvedLink { pub href: String, pub label: Option<String> }

pub trait ContentLinks: Send + Sync {
    /// Gắn lúc `Module::start` của module sau. Gắn hai lần cùng một loại là lỗi khởi động.
    fn attach(&self, target: Arc<dyn LinkTarget>) -> Result<(), Error>;
}
```

`content` công bố `dyn ContentLinks` trong `start`. Bên trong `content` giữ một sổ
(`RwLock<HashMap<LinkKind, Arc<dyn LinkTarget>>>`) như `ResourceDirectory` của kernel. Loại `Url`, `Entry`,
`Term` do `content` tự lo.

### Bảng dữ liệu

Mọi bảng cấp khách hàng có `tenant_id`; unique và index bắt đầu bằng nó (D-185). Kiểu chạy trên cả hai
database: UUID, text, integer, timestamp UTC. Không dùng enum của Postgres: text có kiểm ở service.
Migration mới, có `down`.

| Bảng | Cột chính | Khoá / index |
|---|---|---|
| `content_placements` | `tenant_id`, `id`, `key` (vd `home_hero`, `[a-z0-9_]{1,64}`), `name`, `description`, `max_items` (mặc định 10, ≤ 50), `created_at`, `updated_at`, `version` | unique (`tenant_id`, `key`) |
| `content_banners` | `tenant_id`, `id`, `placement_id`, `org_id` (null = mọi cửa hàng của khách), `locale`, `title`, `subtitle`, `cta_label`, `media_id`, `mobile_media_id` null, `alt`, `link_kind`, `link_url` null, `link_target_id` null, `starts_at` null, `ends_at` null, `position`, `status` (`draft` / `active` / `archived`), `ai_assisted`, `created_by`, `created_at`, `updated_at`, `version` | index (`tenant_id`, `placement_id`, `locale`, `status`, `position`); index (`tenant_id`, `link_kind`, `link_target_id`) |
| `catalog_collections` | `tenant_id`, `id`, `org_id` null, `kind` (`manual` / `rule`), `rules` text (JSON đã kiểm), `sort` (`manual` / `newest` / `price_asc` / `price_desc` / `best_selling`), `media_id` null, `status`, `starts_at`, `ends_at`, `version`, thời điểm | index (`tenant_id`, `status`) |
| `catalog_collection_texts` | `tenant_id`, `collection_id`, `locale`, `title`, `slug`, `description`, `meta_title`, `meta_description` | unique (`tenant_id`, `locale`, `slug`) |
| `catalog_collection_items` | `tenant_id`, `collection_id`, `product_id`, `position` | unique (`tenant_id`, `collection_id`, `product_id`); index (`tenant_id`, `collection_id`, `position`) |
| `catalog_product_stats` | `tenant_id`, `product_id`, `org_id`, `sold_30d`, `sold_total`, `updated_at` | unique (`tenant_id`, `product_id`, `org_id`) |

Thông tin cửa hàng là **cài đặt kế thừa của nút** do `organizations` khai qua `Module::org_settings`
(bước 34), không phải bảng mới:
- `storefront.name`, `.logo_media_id`, `.icon_media_id`, `.phone`, `.email`;
- `.address` (`Address`), `.opening_hours`, `.holidays`, `.social_links`.

Giá trị theo ngôn ngữ thì khoá có hậu tố `.<locale>`.

Quy tắc của bộ sưu tập `rule` là JSON có schema cố định, kiểm ở service:

```json
{ "category_ids": [...], "attributes": {"color": ["red"]}, "price_min": 100000, "price_max": 500000,
  "new_within_days": 30, "on_promotion": false, "match": "all" }
```

- `price_*` là số nguyên đơn vị nhỏ nhất, theo tiền tệ của bảng giá mặc định.
- `on_promotion` cần `promotions`. Thiếu module đó thì quy tắc này bị từ chối khi lưu, với mã
  `catalog.rule_needs_module`.

### API REST

Quản trị (`content.manage` / `catalog.manage`, theo phạm vi cửa hàng của bước 34; banner có `org_id` thì
cần quyền ở nút đó):

| Method + path | Việc |
|---|---|
| `GET/POST /api/v1/content/placements`, `GET/PUT/DELETE /api/v1/content/placements/{key}` | Vị trí; xoá vị trí còn banner → 409 `content.placement_in_use` |
| `GET/POST /api/v1/content/banners` (lọc `placement`, `org_id`, `status`, `locale`), `GET/PATCH/DELETE /api/v1/content/banners/{id}` | Banner; `PATCH` cần `If-Match` (ETag như menu) |
| `PUT /api/v1/content/placements/{key}/order` | Đặt lại thứ tự cả vị trí một lần, `If-Match` |
| `GET/POST /api/v1/catalog/collections`, `GET/PATCH/DELETE /api/v1/catalog/collections/{id}`, `PUT /api/v1/catalog/collections/{id}/items` | Bộ sưu tập; thay cả danh sách sản phẩm chọn tay một lần |
| `GET/PUT /api/v1/organizations/units/{id}/storefront` | Thông tin cửa hàng của nút (giá trị tự đặt và giá trị kế thừa, ghi rõ nút nào cho giá trị) |

Công khai (không cần đăng nhập, `public_cache` như `content`; khách suy từ tên miền, bước 33):

| Method + path | Trả về |
|---|---|
| `GET /api/v1/content/public/placements/{key}?locale=&org_id=` | Banner `active`, trong khung giờ lúc gọi, của `org_id` và các nút tổ tiên (dùng `OrgTree`), sắp theo `position`. Ảnh dạng `PublicMedia` (có `variants`, `webp_url`), link đã dựng (`href`); banner có link không còn hiển thị được thì bỏ |
| `GET /api/v1/catalog/public/collections/{slug}?locale=` + phân trang sản phẩm | Bộ sưu tập và sản phẩm (cùng hình dạng thẻ sản phẩm của bước 41) |
| `GET /api/v1/organizations/public/storefront?org_id=` | Thông tin cửa hàng đã kế thừa, cùng "đang mở cửa" tính theo múi giờ của nút. Bước 37 đã có thì kèm thông tin chủ sở hữu website và đoạn mã logo Bộ Công Thương của tên miền |

**Cache của vị trí banner.**
- `max-age` = min(mặc định của `public_cache`, số giây tới mốc `starts_at` / `ends_at` gần nhất trong
  tương lai của các banner trong vị trí đó). Nhờ vậy CDN không giữ banner đã hết giờ.
- `ETag` từ `version` của các dòng trả về và mốc đó.

### Tool MCP · Event · Job · Permission · Mã lỗi

- **Tool MCP:**
  - `content.list_banners`, `content.save_banner`, `content.order_placement`;
  - `catalog.list_collections`, `catalog.save_collection`.
  - Cùng service, cùng permission với REST (luật kiến trúc 8).
  - Tool không tự đổi `status` sang `active`: chỉ tạo bản nháp. Con người bật lên, như luật AI của D-109.
- **Event:**
  - `content.banner_changed`, `catalog.collection_changed` (outbox, cùng transaction);
  - `content` nhận `promotions.program_ended` bằng `Subscription::optional`, rồi chuyển banner trỏ tới
    chương trình đó sang `archived`. Chịu được nhận lại event.
- **Số bán:** `catalog` nhận `orders.order_completed` và `orders.refund_completed` (tuỳ chọn) để cập nhật
  `catalog_product_stats`, cộng / trừ bằng `rows::adjust`.
- **Lịch:** `catalog.roll_sold_30d`, hằng ngày, tính lại cửa sổ 30 ngày. `unique_key` có khe ngày (README).
- **Permission:** dùng `content.manage`, `catalog.manage`, `organizations.manage`; không thêm permission mới
  trừ khi bước 34 tách sẵn.
- **Mã lỗi** (thêm vào `locales/vi.json`):

  | Mã | Câu tiếng Việt |
  |---|---|
  | `content.placement_not_found` | "Không có vị trí hiển thị `{key}`." |
  | `content.placement_in_use` | "Vị trí còn banner, hãy chuyển hay xoá banner trước." |
  | `content.placement_full` | "Vị trí đã đủ {max} banner." |
  | `content.banner_not_found` | — |
  | `content.link_target_missing` | "Đối tượng được trỏ tới không có." |
  | `content.link_kind_unavailable` | "Chưa cài module cho loại link này." |
  | `content.banner_window_invalid` | "Giờ kết thúc phải sau giờ bắt đầu." |
  | `catalog.collection_not_found` | — |
  | `catalog.rule_invalid` | — |
  | `catalog.rule_needs_module` | — |

## Thứ tự làm

1. `content`: bảng vị trí và banner, service, API quản trị; link loại `Url` / `Entry` / `Term`. Gồm test.
   → mục 1 của ROADMAP.
2. `content`: điểm cắm `ContentLinks`, endpoint công khai, tính `max-age`, nhận event tuỳ chọn của
   `promotions`. → mục 2 và 5.
3. `organizations`: khai cài đặt `storefront.*`, endpoint quản trị và công khai, "đang mở cửa". → mục 4.
4. `catalog`: bộ sưu tập chọn tay, rồi theo quy tắc, rồi số bán và kiểu sắp `best_selling`; gắn
   `LinkTarget` cho sản phẩm và bộ sưu tập. → mục 3 và 5.
5. `promotions`: gắn `LinkTarget` cho chương trình. → mục 5.
6. Tool MCP, tài liệu cho người làm giao diện: thêm mục vào `docs/CONTENT.md` / `.vi.md` và tài liệu của
   `catalog`, có ví dụ trang chủ; CHANGELOG. → mục 6.

## Test bắt buộc

Mọi test chạy trên cả SQLite và Postgres qua `TestDatabase::run` / `TestApp`.

- `banner_ngoai_khung_gio_khong_hien`:
  - giờ giả lập trước `starts_at`, trong khung và sau `ends_at`;
  - `max-age` không vượt mốc gần nhất.
- `banner_cua_cua_hang_va_cua_chuoi`:
  - nút cửa hàng thấy banner của mình và của tổ tiên, không thấy banner của cửa hàng anh em;
  - đây là bài rò rỉ giữa hai nhánh anh em của bước 34.
- `banner_khach_khac_khong_thay`: bài rò rỉ giữa khách của testkit.
- `banner_tro_toi_chuong_trinh_het_han_bi_an`:
  - nhận `promotions.program_ended` hai lần vẫn đúng;
  - cài thiếu `promotions` thì không lưu được link loại `Promotion`.
- `anh_banner_rieng_tu_bi_tu_choi`: `media_id` của ảnh không công khai → lỗi khi lưu, như `content` đang
  làm với ảnh của bài.
- `doi_thu_tu_dong_thoi_mot_ben_thua`: hai `PUT …/order` cùng `If-Match` → một bên 412.
- `bo_suu_tap_theo_quy_tac_dung_ca_hai_database`: cùng dữ liệu, cùng kết quả và thứ tự trên hai database
  (sắp ổn định theo ID khi bằng nhau).
- `so_ban_cong_tru_khi_hoan`: hoàn tiền trừ lại, không âm (`rows::adjust` có sàn 0).
- `thong_tin_cua_hang_ke_thua`:
  - cửa hàng không đặt số điện thoại thì lấy của chuỗi;
  - "đang mở cửa" đúng theo múi giờ của nút, kể cả qua nửa đêm.

## Tiêu chí xong

- [ ] Mọi mục của bước 55 trong ROADMAP `[x]`.
- [ ] `./gate.sh` đầy đủ xanh.
- [ ] `docs/openapi.json` đã sinh lại; mỗi endpoint đủ mô tả.
- [ ] `locales/vi.json` có mọi mã lỗi mới.
- [ ] Tài liệu EN + VI cho người làm giao diện, cùng commit.
- [ ] CHANGELOG ở `[Unreleased]`.
- [ ] Quyết định riêng của bước (vd banner trong gian hàng người bán) ghi DECISIONS.
- [ ] Phát hành theo `docs/RELEASING.md` (D-171).

## Chỗ dễ sai

- **"Bây giờ" khác nhau giữa hai database và giữa máy:** lấy giờ ở ứng dụng (UTC, cắt micro giây như
  `content::service::now`), truyền vào truy vấn. Không dùng `CURRENT_TIMESTAMP` của database.
- **Cache công khai giữ banner hết giờ:** xem cách tính `max-age` ở trên. Đừng dùng `max-age` cố định.
- **N+1:** lấy ảnh bằng `Media::find_many` một lần; link bằng `LinkTarget::resolve` một lần cho mỗi loại.
- **Quy tắc bộ sưu tập thành câu SQL quá đắt:**
  - giới hạn số điều kiện;
  - index có sẵn của `catalog` phải phủ;
  - đo bằng `EXPLAIN` trên bộ dữ liệu lớn (bước 39).
- **`org_id` null nghĩa là "mọi cửa hàng của khách", không phải "không ai thấy".** Viết test riêng.
- **Banner trỏ tới sản phẩm bị xoá:** `resolve` trả thiếu ID đó thì bỏ banner khỏi kết quả công khai. Không
  báo lỗi cho người xem.

## Căn cứ pháp lý

- **Quảng cáo phải trung thực, điều kiện khuyến mại phải minh bạch.** Luật Bảo vệ quyền lợi người tiêu dùng
  19/2023/QH15 (từ 01/7/2024); Luật Quảng cáo 16/2012/QH13 và văn bản sửa đổi — cần đối chiếu văn bản gốc.
  - Banner trỏ tới chương trình đã kết thúc phải tự ẩn.
  - Chữ trên banner do người soạn chịu trách nhiệm; core không tự sinh.
- **Nội dung do AI gợi ý mang dấu `ai_assisted`** (Luật Trí tuệ nhân tạo 134/2025/QH15, từ 01/3/2026 —
  cần đối chiếu; bước 37).
- **Chân trang website bán hàng phải có thông tin chủ sở hữu** (NĐ 52/2013 sửa bởi NĐ 85/2021 và văn bản
  hướng dẫn Luật TMĐT 2025 — cần đối chiếu). Lấy từ hồ sơ pháp lý của bước 37, không nhập lại ở đây.

## Agent làm bước này tự quyết

- **Banner trong gian hàng người bán của sàn:** khuyến nghị để sau. Nếu làm thì banner có `org_id` là nút
  `seller`, và người bán chỉ quản trị trong nút của mình (phạm vi `Unit`).
- **Có cho video trong banner không:** khuyến nghị chỉ ảnh ở bước này.
- **Giới hạn số điều kiện của quy tắc bộ sưu tập:** khuyến nghị 10.
- Ghi các lựa chọn trên vào `docs/DECISIONS.md`.
