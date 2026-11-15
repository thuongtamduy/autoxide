# Bước 55 — Giao diện cửa hàng qua API: banner, bộ sưu tập, thông tin cửa hàng

> Lộ trình: `docs/ROADMAP.md` bước 55 · Luật chung: `docs/steps/README.md` · Quyết định: D-219, D-026
> (core chỉ có API), D-191 (module thương mại), D-195 / D-198 (luật chung giữa các bước), D-236 (kiểu vị trí, bộ vị
> trí chuẩn, chọn trang hiện, banner dạng chữ, khối vùng banner)

## Mục tiêu

Website và app của khách tự vẽ giao diện; core không có trang quản trị hay giao diện (D-026). Bước này cho
giao diện **dữ liệu** để dựng trang chủ, trang danh mục, chân trang:

- banner theo vị trí, có lịch; vị trí có kiểu hiển thị (một banner, trượt, lưới, dải chữ, popup…), kích thước ảnh
  khuyến nghị và bộ vị trí chuẩn có sẵn; banner chọn được trang nào hiện (D-236);
- bộ sưu tập sản phẩm;
- thông tin cửa hàng công khai.

Đổi banner hay bộ sưu tập là đổi dữ liệu qua API hay MCP, không sửa code giao diện. Phục vụ mọi mô hình:

- **Khách độc lập:** một bộ banner.
- **Chuỗi:** banner chung của chuỗi và banner riêng của từng cửa hàng.
- **Sàn:** banner của sàn; banner trong gian hàng của từng người bán, người bán tự quản trong nút của mình, sàn
  duyệt theo chính sách là dữ liệu, link chỉ tới hàng của chính người bán (D-233).

## Bắt đầu khi

- **Bước 41 xong:** `catalog` có sản phẩm, danh mục, slug, đọc công khai.
- **Banner và thông tin cửa hàng** chỉ cần bước 34 (cây tổ chức, `Address`, cài đặt kế thừa). Làm được
  trước bước 41 nếu có người rảnh; khi đó link tới sản phẩm hay bộ sưu tập chờ `catalog` gắn vào điểm cắm.
- **Bước 37:** dấu `ai_assisted`, thông tin chủ sở hữu và logo Bộ Công Thương cho chân trang. Thiếu bước 37
  thì bỏ hai phần đó, không chặn bước này.
- **Bước 67:** khối `content.placement` (đặt vùng banner vào trang dựng từ khối, D-236) cần sổ khối của bước 67.
  Bước 67 đi trước bước này theo thứ tự lộ trình; chưa có thì để khối lại, không chặn bước này.
- **Bước 46:** banner trong gian hàng người bán cần nút `seller` và `marketplace` (D-233). Làm sau các mục khác
  của bước này; chưa có bước 46 thì để mục đó lại, không chặn bước này.

## Phạm vi

- **`content`:**
  - bảng vị trí và banner, API quản trị và công khai, tool MCP;
  - kiểu vị trí, bộ vị trí chuẩn gieo cho khách mới, chọn trang hiện, banner dạng chữ, khối `content.placement`
    (D-236);
  - điểm cắm `ContentLinks` trong `content::api` (thêm `pub mod api` nếu bước 37 chưa thêm);
  - banner trong gian hàng người bán: vị trí `seller_store.*`, duyệt, chính sách của sàn, hạn mức (D-233).
- **`catalog`:**
  - bảng bộ sưu tập, API quản trị và công khai, tool MCP;
  - gắn bộ dựng link cho sản phẩm và bộ sưu tập vào `ContentLinks`;
  - số bán của sản phẩm cho kiểu sắp "bán chạy";
  - đăng ký `catalog.category` và `catalog.collection` vào sổ đối tượng (bước 41 mới có `catalog.product`) để banner
    chọn trang hiện (D-236).
- **`organizations`:** hồ sơ công khai của nút (thông tin cửa hàng, giờ mở cửa) và endpoint công khai.
- **`promotions`:** gắn bộ dựng link chương trình vào `ContentLinks`; phát event khi chương trình kết thúc
  (nếu bước 43 chưa có).
- **`marketplace`:** gắn bộ dựng link loại `SellerStore` (trang gian hàng) vào `ContentLinks`; role mẫu chủ
  gian hàng có thêm `content.manage` độ sâu `unit` (D-233).
- **Ngoài phạm vi:**
  - giao diện, theme, trình kéo thả trang (D-026);
  - đo lượt xem / bấm banner: thuộc bước 57 (`analytics`, module khai loại sự kiện của mình);
  - thử A/B: bước 67 (D-231);
  - quảng cáo trả phí trong sàn: bước 64 (`ads`) — banner gian hàng không phải quảng cáo (D-233).

## Thiết kế

### Kiểu và trait

Phác thảo — tra API thật bằng skill `check-library-api` và code của bước 33–43 trước khi viết.

```rust
// autoxide_content::api — điểm cắm cho module đăng ký SAU content (README, "Điểm cắm")
pub enum LinkKind { Url, Entry, Term, Product, Collection, Promotion, SellerStore /* D-233 */ }

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
    /// Nút sở hữu từng đối tượng (sản phẩm, bộ sưu tập có `org_id`; trang gian hàng = chính nút `seller`) —
    /// kiểm link của banner gian hàng (D-233). Cài mặc định trả map rỗng: loại đó không dùng được cho banner
    /// gian hàng.
    async fn owners(&self, tenant: &TenantContext, ids: &[Uuid]) -> Result<HashMap<Uuid, OrgId>, Error> {
        Ok(HashMap::new())
    }
}
pub struct ResolvedLink { pub href: String, pub label: Option<String> }

pub trait ContentLinks: Send + Sync {
    /// Gắn lúc `Module::start` của module sau. Gắn hai lần cùng một loại là lỗi khởi động.
    fn attach(&self, target: Arc<dyn LinkTarget>) -> Result<(), Error>;
}
```

`content` công bố `dyn ContentLinks` trong `start`. Bên trong `content` giữ một sổ
(`RwLock<HashMap<LinkKind, Arc<dyn LinkTarget>>>`) như `ResourceDirectory` của kernel. Loại `Url`, `Entry`,
`Term` do `content` tự lo. `SellerStore` do `marketplace` gắn (D-233).

### Kiểu vị trí, bộ vị trí chuẩn, chọn trang hiện (D-236)

Vị trí không chỉ là một cái tên: nó nói cho giao diện **vẽ thế nào** và cho người soạn **ảnh cỡ nào**, để đổi
banner không bao giờ làm vỡ giao diện.

- **Kiểu hiển thị** (`display`): `single` (một banner), `carousel` (trượt), `grid` (lưới 2–4 ô), `stack` (xếp dọc,
  hợp cột trái / phải), `strip` (dải chữ mỏng trên cùng), `popup` (khung nổi khuyến mại). Cài đặt theo kiểu
  (`display_settings`, JSON có schema, kiểm khi lưu): `carousel` — `autoplay_seconds` (0 = tắt, nếu bật thì ≥ 4);
  `grid` — `columns` (2–4); `popup` — `frequency` (`once_per_session` / `once_per_day` / `once_per_week`),
  `delay_seconds` (0–60). Giao diện vẽ theo hợp đồng ở `docs/CONTENT.md`: luôn có nút đóng popup, không popup ở
  trang giỏ hàng / thanh toán / đăng nhập / tài khoản, không tự trượt khi người xem bật giảm chuyển động.
- **Loại banner nhận** (`accepts`): `image`, `text` hay `any`. `strip` chỉ nhận `text`.
- **Kích thước ảnh** (`ratio_w`, `ratio_h`, `min_width`, `min_height`, đều tuỳ chọn): ảnh của banner (cả ảnh điện
  thoại nếu vị trí khai tỉ lệ riêng `mobile_ratio_w`, `mobile_ratio_h`) phải đủ cỡ và đúng tỉ lệ (sai lệch ≤ 2%) —
  kiểm khi lưu bằng kích thước `media` đã lưu, báo lỗi kèm cỡ cần. Banner gian hàng người bán (D-233) kiểm thêm
  chính sách của sàn, lấy mức chặt hơn.
- **Thiết bị** (`devices`): `all` / `desktop` / `mobile` — giao diện chỉ gọi vị trí hợp thiết bị.
- **Trang được hiện** (`page_kinds`, JSON): tập loại trang vị trí này dùng được — `home`, `category`, `collection`,
  `product`, `search`, `cart`, `content_entry`, `content_type` (bước 67), `landing` (trang dựng từ khối), `any`.
- **Bộ vị trí chuẩn** gieo cho khách mới (`Module::seed_tenant` của bước 33; dòng có `preset_key`, khách sửa / tắt /
  xoá được như vị trí tự tạo):

  | `key` | Kiểu | Nhận | Tỉ lệ · cỡ tối thiểu | Trang |
  |---|---|---|---|---|
  | `top_strip` | `strip` | `text` | — | `any` |
  | `home_hero` | `carousel` | `image` | 3:1 · 1800×600 (điện thoại 4:5 · 800×1000) | `home` |
  | `home_middle` | `grid` (3) | `image` | 1:1 · 600×600 | `home` |
  | `category_top` | `single` | `image` | 4:1 · 1600×400 | `category`, `collection` |
  | `sidebar_left`, `sidebar_right` | `stack` | `image` | 1:2 · 300×600 | `category`, `collection`, `search`, `content_entry` |
  | `inline` | `single` | `any` | 4:1 · 1200×300 | `content_entry`, `product`, `landing` |
  | `above_footer` | `single` | `image` | 4:1 · 1600×400 | `any` |
  | `popup` | `popup` (`once_per_day`, 5 giây) | `image` | 4:5 · 600×750 | `home`, `category`, `content_entry` |

  Vị trí `seller_store.*` (D-233) mang kiểu `carousel` / `single`, ảnh theo chính sách của sàn.
- **Banner chọn trang hiện** trong phạm vi của vị trí: `page_kinds` (tập con, trống = mọi loại của vị trí) và
  `page_targets` (JSON, tối đa 50 cặp `{kind, id}` theo sổ đối tượng — vd `catalog.category`, `catalog.collection`,
  `catalog.product`, `content.entry`; kiểm tồn tại khi lưu, `content.page_target_not_found`). Ví dụ: banner cột trái
  chỉ ở danh mục "Điện thoại".
- **Banner dạng chữ** (`kind = text`): `title` (≤ 120 ký tự), `cta_label?`, link như banner ảnh, `tone` (tên token màu
  của theme — `primary`, `secondary`, `accent`, `neutral`; không mã màu tự do, đổi theme là dải chữ đổi theo); không
  `media_id`. Hợp cho dải trên cùng: "Miễn phí giao hàng đơn từ 300.000đ — xem ngay".
- **Khác với khối của bước 67:** khối `announcement_bar` / `popup` của trang dựng từ khối là chữ **cố định** trong một
  trang (popup đó để thu email, có đồng ý); vị trí banner là nội dung **có lịch, xoay vòng, quản lý một chỗ** dùng
  ở nhiều trang. Muốn dải trên cùng có lịch thì đặt khối `content.placement {key: "top_strip"}` vào đầu trang của
  website (D-234).
- **Khối `content.placement`** (khai ở `content` khi có sổ khối của bước 67): props `{key}`; `BlockData` gom mọi vị trí
  trong một trang thành **một** lần đọc, trang dựng từ khối tính là `page_kind = landing` (hay `content_entry` /
  `content_type` theo trang), áp đúng lọc trang như endpoint công khai. Trang đích tự làm có cột phải hay slider
  dùng chung banner đã quản lý.
- **Thứ tự cố định** theo `position` khách sắp — không đảo ngẫu nhiên: đảo phá cache và người xem F5 thấy nhảy.

### Banner trong gian hàng người bán trên sàn (D-233)

Người bán trên sàn tự đặt banner trong gian hàng của mình; sàn giữ quyền duyệt và đặt luật bằng dữ liệu.

- **Chủ sở hữu:** banner gian hàng có `org_id` là nút `seller` (bước 46; `content` hỏi loại nút qua
  `OrgDirectory`, không cần `marketplace`).
- **Vị trí riêng:** `seller_store.top`, `seller_store.middle` — dòng `content_placements` có
  `audience = seller_store`, `content` tạo trong `Module::seed_tenant` và cho khách sẵn có bằng
  `Module::upgrades` (bước 33). Tiền tố `seller_store.` dành riêng: khách không tạo khoá có dấu chấm, không xoá
  được hai vị trí này (`content.placement_reserved`). Banner có `org_id` là nút `seller` chỉ vào vị trí
  `seller_store`, và vị trí `seller_store` chỉ nhận banner như vậy (`content.seller_banner_placement_mismatch`).
- **Quyền:** dùng permission banner hiện có `content.manage`, độ sâu `unit` ở nút `seller` (role mẫu chủ gian
  hàng của `marketplace`). Ghi: `principal.require_write_at(banner.org_id)`; danh sách lọc bằng
  `scope_condition`. Người bán không thấy, không sửa được banner của người bán khác hay của sàn.
- **Chính sách của sàn** (`content_seller_banner_policies`, dữ liệu theo nút sàn, lấy dòng của tổ tiên gần
  nhất có chính sách qua `OrgTree`):
  - `review_mode`: `pre_approval` (duyệt trước khi hiện) hay `post` (hiện ngay, sàn vẫn từ chối được sau);
  - `max_per_seller`: số banner chưa lưu trữ tối đa mỗi người bán; không vượt hạn mức gói
    `content.seller_banners_per_seller` của khách (bước 35, hỏi qua `EntitlementsHandle`) — lấy số nhỏ hơn;
  - kích thước ảnh: `image_min_width`, `image_min_height`, `image_max_bytes` (NULL = chỉ luật chung của
    `media`), kiểm bằng thông tin ảnh của `Media::find_many`.
  - Chưa có dòng nào: `pre_approval`, 5 banner, không ràng thêm về ảnh.
- **Duyệt:** cột `review_status` (`not_required` / `pending` / `approved` / `rejected`) bên cạnh `status`.
  - Banner của sàn và của cửa hàng: `not_required`, như trước.
  - Banner gian hàng chuyển sang `active` với `pre_approval` → `pending`; sửa chữ, ảnh, link, khung giờ, ngôn
    ngữ của banner `approved` / `rejected` → về `pending`. Với `post` → `approved` ngay.
  - Sàn duyệt bằng `POST /banners/{id}/review` `{decision: approve | reject, reason}`, `If-Match`; từ chối bắt
    buộc có lý do. Người duyệt cần `content.manage` ở nút cha của nút `seller` (nút sàn): người bán có quyền
    độ sâu `unit` không tự duyệt được banner của mình.
- **Link chỉ tới hàng của chính người bán:** `link_kind` ∈ `Product`, `Collection`, `SellerStore`, và
  `LinkTarget::owners` trả đúng `banner.org_id`. `Url`, `Entry`, `Term`, `Promotion` và đối tượng của người khác
  → `content.seller_banner_link_forbidden`. Kiểm khi lưu; khi đọc công khai vẫn bỏ banner có link không còn
  hiển thị được (như mọi banner).
- **Không dụ giao dịch ngoài sàn** (như bước 46, mẫu che liên hệ của bước 61): `title`, `subtitle`,
  `cta_label`, `alt` không được chứa URL, tên miền, email, số điện thoại (mẫu cố định trong `content`) →
  `content.seller_banner_contact_forbidden`. Chữ trong ảnh không kiểm được bằng máy: sàn kiểm khi duyệt.
- **Hạn mức:** đếm ở `content_seller_banner_counts`, tăng bằng `UPDATE … SET count = count + 1 WHERE count <
  max` (như ngân sách của bước 64) trong cùng transaction với lúc tạo / bỏ lưu trữ; giảm khi lưu trữ / xoá
  (`rows::adjust`, sàn 0). Không cập nhật dòng nào → `content.seller_banner_limit_reached`.
- **Không phải quảng cáo trong sàn (bước 64):** không trả phí, không `sponsored`, chỉ đọc được qua vị trí
  `seller_store.*` với `org_id` của chính nút `seller`; không bao giờ xuất hiện ở vị trí chung của sàn.

### Bảng dữ liệu

Mọi bảng cấp khách hàng có `tenant_id`; unique và index bắt đầu bằng nó (D-185). Kiểu chạy trên cả hai
database: UUID, text, integer, timestamp UTC. Không dùng enum của Postgres: text có kiểm ở service.
Migration mới, có `down`.

| Bảng | Cột chính | Khoá / index |
|---|---|---|
| `content_placements` | `tenant_id`, `id`, `key` (vd `home_hero`, `[a-z0-9_]{1,64}`; riêng vị trí hệ thống `seller_store.top`, `seller_store.middle` — D-233), `name`, `description`, `max_items` (mặc định 10, ≤ 50), `audience` (`site` mặc định / `seller_store`, D-233), `display`, `display_settings` (JSON), `accepts`, `ratio_w?`, `ratio_h?`, `mobile_ratio_w?`, `mobile_ratio_h?`, `min_width?`, `min_height?`, `devices`, `page_kinds` (JSON), `preset_key?`, `active` (D-236), `created_at`, `updated_at`, `version` | unique (`tenant_id`, `key`) |
| `content_banners` | `tenant_id`, `id`, `placement_id`, `org_id` (null = mọi cửa hàng của khách), `locale`, `kind` (`image` / `text`, D-236), `title`, `subtitle`, `cta_label`, `tone?` (banner chữ, D-236), `media_id` null (bắt buộc khi `image`), `mobile_media_id` null, `page_kinds` (JSON), `page_targets` (JSON) (D-236), `alt`, `link_kind`, `link_url` null, `link_target_id` null, `starts_at` null, `ends_at` null, `position`, `status` (`draft` / `active` / `archived`), `review_status` (`not_required` mặc định / `pending` / `approved` / `rejected`), `review_reason` null, `reviewed_by` null, `reviewed_at` null (bốn cột duyệt — D-233), `ai_assisted`, `created_by`, `created_at`, `updated_at`, `version` | index (`tenant_id`, `placement_id`, `locale`, `status`, `position`); index (`tenant_id`, `link_kind`, `link_target_id`); index (`tenant_id`, `review_status`, `id`) (hàng chờ duyệt, D-233) |
| `content_seller_banner_policies` (D-233) | `tenant_id`, `org_id` (nút sàn), `review_mode` (`pre_approval` / `post`), `max_per_seller`, `image_min_width` null, `image_min_height` null, `image_max_bytes` null, `updated_by`, `updated_at`, `version` | pk (`tenant_id`, `org_id`) |
| `content_seller_banner_counts` (D-233) | `tenant_id`, `org_id` (nút `seller`), `count` (banner chưa lưu trữ), `updated_at` | pk (`tenant_id`, `org_id`) |
| `catalog_collections` | `tenant_id`, `id`, `org_id` null, `kind` (`manual` / `rule`), `rules` text (JSON đã kiểm), `sort` (`manual` / `newest` / `price_asc` / `price_desc` / `best_selling`), `media_id` null, `status`, `starts_at`, `ends_at`, `version`, thời điểm | index (`tenant_id`, `status`) |
| `catalog_collection_texts` | `tenant_id`, `collection_id`, `locale`, `title`, `slug`, `description`, `meta_title`, `meta_description` | unique (`tenant_id`, `locale`, `slug`) |
| `catalog_collection_items` | `tenant_id`, `collection_id`, `product_id`, `position` | unique (`tenant_id`, `collection_id`, `product_id`); index (`tenant_id`, `collection_id`, `position`) |
| `catalog_product_stats` | `tenant_id`, `product_id`, `org_id`, `sold_30d`, `sold_total`, `updated_at` | unique (`tenant_id`, `product_id`, `org_id`) |

Thông tin cửa hàng có hai phần, mỗi thứ một chỗ (D-237):
- **Thương hiệu** — **cài đặt kế thừa của nút** do `organizations` khai qua `Module::org_settings` (bước 34), không
  phải bảng mới (chuỗi đặt, cửa hàng ghi đè): `storefront.name` (tên thương hiệu), `.logo_media_id`, `.icon_media_id`,
  `.phone`, `.email`, `.social_links`.
- **Địa điểm** — hồ sơ công khai của nút ở bước 34 (`display_name`, `Address`, giờ mở cửa, ngày đặc biệt,
  `pickup`, ảnh bìa): không lặp lại ở đây.

Giá trị theo ngôn ngữ thì khoá có hậu tố `.<locale>`.

Dùng cho khung website (D-234, bước 67 có trước): `organizations` khai khối `organizations.store_info` qua
`Module::blocks()` (chọn trường hiện: tên, liên hệ, địa chỉ, giờ mở cửa, "đang mở cửa") kèm `BlockData` theo lô, và
công bố logo đã kế thừa qua trait để theme của `content` dùng khi logo theme để trống — một chỗ đặt logo cho trang,
mail, hoá đơn.

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
| `GET /api/v1/content/banners?review_status=pending` | Hàng chờ duyệt banner gian hàng của các nút `seller` trong phạm vi người gọi (D-233) |
| `POST /api/v1/content/banners/{id}/review` | Sàn duyệt `{decision: approve \| reject, reason}`, `If-Match`; cần `content.manage` ở nút cha của nút `seller`; từ chối cần lý do (D-233) |
| `GET/PUT /api/v1/content/seller-banner-policies/{org_id}` | Chính sách banner gian hàng của nút sàn; `content.manage` ở nút đó; `GET` trả cả chính sách hiệu lực (kế thừa, mặc định) (D-233) |

Người bán dùng chính `GET/POST /api/v1/content/banners` và `PATCH/DELETE …/{id}` với `org_id` là nút `seller` của
mình (D-233).

Công khai (không cần đăng nhập, `public_cache` như `content`; khách suy từ tên miền, bước 33):

| Method + path | Trả về |
|---|---|
| `GET /api/v1/content/public/placements?keys=&locale=&org_id=&page_kind=&page_ref=` (D-236) | Nhiều vị trí của **một trang** trong một lời gọi (tối đa 10 `keys`): mỗi vị trí kèm `display`, `display_settings`, `accepts`, cỡ ảnh, `devices` và banner đã lọc theo trang (`page_ref` = `<kind>:<id>` của trang đang xem; banner không giới hạn trang luôn có). Cùng luật như dòng dưới; `max-age` là mốc gần nhất của mọi vị trí |
| `GET /api/v1/content/public/placements/{key}?locale=&org_id=&page_kind=&page_ref=` | Banner `active`, trong khung giờ lúc gọi, của `org_id` và các nút tổ tiên (dùng `OrgTree`), sắp theo `position`. Ảnh dạng `PublicMedia` (có `variants`, `webp_url`), link đã dựng (`href`); banner có link không còn hiển thị được thì bỏ. Vị trí `seller_store.*`: `org_id` bắt buộc là nút `seller`, chỉ trả banner của đúng nút đó (không lấy tổ tiên) có `review_status` `approved` (D-233) |
| `GET /api/v1/catalog/public/collections/{slug}?locale=` + phân trang sản phẩm | Bộ sưu tập và sản phẩm (cùng hình dạng thẻ sản phẩm của bước 41) |
| `GET /api/v1/organizations/public/storefront?org_id=` | Thông tin cửa hàng: thương hiệu đã kế thừa (`storefront.*`) ghép hồ sơ địa điểm của nút (bước 34), cùng "đang mở cửa" tính theo múi giờ của nút. Bước 37 đã có thì kèm thông tin chủ sở hữu website và đoạn mã logo Bộ Công Thương của tên miền |

**Cache của vị trí banner.**
- `max-age` = min(mặc định của `public_cache`, số giây tới mốc `starts_at` / `ends_at` gần nhất trong
  tương lai của các banner trong vị trí đó). Nhờ vậy CDN không giữ banner đã hết giờ.
- `ETag` từ `version` của các dòng trả về và mốc đó.
- Lọc theo trang chạy trong bộ nhớ trên các banner đang hiệu lực của vị trí (tối đa `max_items`, ≤ 50) — câu truy
  vấn không đổi, không thêm index; `page_kind` / `page_ref` nằm trong đường dẫn nên CDN tự cache riêng từng trang.

### Tool MCP · Event · Job · Permission · Mã lỗi

- **Tool MCP:**
  - `content.list_banners`, `content.save_banner`, `content.order_placement`;
  - `catalog.list_collections`, `catalog.save_collection`.
  - Cùng service, cùng permission với REST (luật kiến trúc 8).
  - Tool không tự đổi `status` sang `active`: chỉ tạo bản nháp. Con người bật lên, như luật AI của D-109.
  - Banner gian hàng (D-233): người bán dùng `content.save_banner` trong nút của mình; `content.list_banners` lọc
    được `review_status`. Không có tool duyệt / từ chối banner: quyết định của người duyệt ở sàn.
- **Event:**
  - `content.banner_changed`, `catalog.collection_changed` (outbox, cùng transaction);
  - `content` nhận `promotions.program_ended` bằng `Subscription::optional`, rồi chuyển banner trỏ tới
    chương trình đó sang `archived`. Chịu được nhận lại event.
  - `content.banner_reviewed` (`banner_id`, `org_id` của nút `seller`, `decision`, `reason`) — để báo người bán
    qua module đăng ký sau (D-233).
- **Số bán:** `catalog` nhận `orders.order_completed` và `orders.refund_completed` (tuỳ chọn) để cập nhật
  `catalog_product_stats`, cộng / trừ bằng `rows::adjust`.
- **Lịch:** `catalog.roll_sold_30d`, hằng ngày, tính lại cửa sổ 30 ngày. `unique_key` có khe ngày (README).
- **Permission:** dùng `content.manage`, `catalog.manage`, `organizations.manage`; không thêm permission mới
  trừ khi bước 34 tách sẵn. Banner gian hàng cũng dùng `content.manage`, giới hạn theo độ sâu (D-233).
- **Hạn mức** (bước 35, D-233): `content.seller_banners_per_seller` — số banner tối đa mỗi người bán theo gói của
  khách; chính sách của sàn chỉ đặt được thấp hơn hay bằng.
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
  | `content.placement_reserved` (409, D-233) | "Vị trí hệ thống `{key}` không xoá hay đổi khoá được." |
  | `content.seller_banner_placement_mismatch` (422, D-233) | "Banner của gian hàng chỉ đặt ở vị trí dành cho gian hàng, và vị trí đó chỉ nhận banner của gian hàng." |
  | `content.seller_banner_link_forbidden` (422, D-233) | "Banner của gian hàng chỉ trỏ được tới sản phẩm, bộ sưu tập hay trang gian hàng của chính người bán." |
  | `content.seller_banner_contact_forbidden` (422, D-233) | "Chữ trên banner không được chứa link, tên miền, email hay số điện thoại." |
  | `content.seller_banner_limit_reached` (409, D-233) | "Gian hàng đã có đủ {max} banner." |
  | `content.seller_banner_image_invalid` (422, D-233) | "Ảnh banner phải tối thiểu {min_width}×{min_height} px và không quá {max_bytes} byte." |
  | `content.banner_not_pending` (409, D-233) | "Banner không ở trạng thái chờ duyệt." |
  | `content.review_reason_required` (422, D-233) | "Từ chối banner cần nêu lý do." |
  | `content.seller_store_org_required` (422, D-233) | "Vị trí của gian hàng cần `org_id` của một người bán." |
  | `content.banner_image_size_invalid` (422, D-236) | "Ảnh cho vị trí `{key}` cần tỉ lệ {ratio} và tối thiểu {min_width}×{min_height} px." |
  | `content.banner_kind_not_allowed` (422, D-236) | "Vị trí `{key}` chỉ nhận banner dạng {accepts}." |
  | `content.page_kind_not_allowed` (422, D-236) | "Vị trí `{key}` không dùng ở trang loại `{page_kind}`." |
  | `content.page_target_not_found` (422, D-236) | "Không có trang `{kind}` `{id}` để gắn banner." |
  | `content.placement_settings_invalid` (422, D-236) | "Cài đặt hiển thị của vị trí chưa hợp lệ: {count} lỗi (xem `errors`)." |
  | `content.placement_keys_too_many` (422, D-236) | "Mỗi lần đọc tối đa {max} vị trí." |
  | `catalog.collection_not_found` | — |
  | `catalog.rule_invalid` | — |
  | `catalog.rule_needs_module` | — |

## Thứ tự làm

1. `content`: bảng vị trí và banner, service, API quản trị; link loại `Url` / `Entry` / `Term`. Gồm test.
   Kiểu vị trí, cỡ ảnh, trang được hiện, banner dạng chữ, bộ vị trí chuẩn gieo qua `seed_tenant` (D-236).
   → mục 1 của ROADMAP.
2. `content`: điểm cắm `ContentLinks`, endpoint công khai (cả đọc nhiều vị trí một lần, lọc theo trang — D-236), tính `max-age`, nhận event tuỳ chọn của
   `promotions`. → mục 2 và 5.
3. `organizations`: khai cài đặt `storefront.*` (thương hiệu), endpoint quản trị và công khai ghép hồ sơ địa điểm của
   bước 34, "đang mở cửa". → mục 4.
4. `catalog`: bộ sưu tập chọn tay, rồi theo quy tắc, rồi số bán và kiểu sắp `best_selling`; gắn
   `LinkTarget` cho sản phẩm và bộ sưu tập; đăng ký `catalog.category`, `catalog.collection` vào sổ đối tượng
   (D-236). → mục 3 và 5.
5. `promotions`: gắn `LinkTarget` cho chương trình. → mục 5.
6. Tool MCP, tài liệu cho người làm giao diện: thêm mục vào `docs/CONTENT.md` / `.vi.md` và tài liệu của
   `catalog`, có ví dụ trang chủ; CHANGELOG. → mục 6.
7. Banner gian hàng người bán (D-233, sau bước 46): `content` — cột `audience`, cột duyệt, hai vị trí
   `seller_store.*`, chính sách, bộ đếm hạn mức, kiểm link / chữ / ảnh, endpoint duyệt, `LinkTarget::owners`;
   `catalog` cài `owners` cho sản phẩm và bộ sưu tập; `marketplace` gắn `SellerStore` và thêm `content.manage`
   độ sâu `unit` vào role chủ gian hàng; tài liệu EN + VI. → mục 7.
8. Khối `content.placement` cho trang dựng từ khối (D-236, cần sổ khối của bước 67). → mục 1.

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
- Kiểu vị trí và chọn trang (D-236):
  - `anh_sai_ti_le_hay_qua_nho_bi_tu_choi_kem_co_can`, `anh_dien_thoai_kiem_ti_le_rieng`;
  - `dai_chu_chi_nhan_banner_chu`, `banner_chu_khong_nhan_ma_mau_tu_do`;
  - `banner_chi_hien_o_danh_muc_da_chon` (danh mục khác và trang chủ không thấy; banner không giới hạn trang vẫn
    thấy), `page_kind_ngoai_pham_vi_vi_tri_bi_tu_choi`, `page_target_khong_ton_tai_bi_tu_choi`;
  - `doc_nhieu_vi_tri_mot_lan_goi` (đếm query: không tăng theo số vị trí ngoài một lần mỗi bảng),
    `max_age_la_moc_gan_nhat_cua_moi_vi_tri`;
  - `khach_moi_co_bo_vi_tri_chuan`, `xoa_vi_tri_chuan_khong_gieo_lai`, `gieo_lai_khong_trung`;
  - `cai_dat_carousel_duoi_4_giay_bi_tu_choi`, `thu_tu_co_dinh_khong_dao`;
  - `khoi_placement_trong_trang_mot_lan_doc` (trang có 3 khối vị trí → một lần đọc).
- Banner gian hàng người bán (D-233):
  - `nguoi_ban_khong_sua_duoc_banner_cua_nguoi_ban_khac`: đọc, sửa, xoá banner của nút `seller` anh em → 403 /
    404; danh sách không lộ; tạo banner với `org_id` của nút khác hay `null` → 403.
  - `banner_gian_hang_link_ra_ngoai_bi_tu_choi`: link `Url`, `Entry`, `Promotion`, sản phẩm hay bộ sưu tập của
    người bán khác → 422 `content.seller_banner_link_forbidden`; sản phẩm của chính mình → lưu được.
  - `banner_gian_hang_chu_co_lien_he_bi_tu_choi`: số điện thoại, tên miền, email trong `title` / `cta_label`.
  - `banner_gian_hang_cho_duyet_cong_khai_khong_thay`: `pending` và `rejected` không có trong kết quả công khai;
    `approved` thì có; sửa ảnh sau khi duyệt → về `pending`, ẩn lại; `review_mode = post` thì hiện ngay.
  - `nguoi_ban_khong_tu_duyet_banner_cua_minh`: quyền `unit` ở nút `seller` gọi `…/review` → 403; từ chối không
    có lý do → 422.
  - `banner_gian_hang_vuot_han_muc_bi_chan`: tới `max_per_seller` → 409; lưu trữ một cái thì tạo được; hạn mức
    gói thấp hơn chính sách thì theo gói.
  - `tao_banner_gian_hang_dong_thoi_khong_vuot_han_muc`: 10 request tạo song song khi còn 2 chỗ → đúng 2 thành
    công, bộ đếm khớp số banner chưa lưu trữ.
  - `banner_gian_hang_khong_hien_o_vi_tri_chung_cua_san`: vị trí `site` không nhận banner của nút `seller`; đọc
    công khai vị trí của sàn với `org_id` của người bán không trả banner gian hàng; `seller_store.*` không có
    `org_id` người bán → 422.
  - `anh_banner_gian_hang_sai_kich_thuoc_bi_tu_choi`: ảnh nhỏ hơn hay nặng hơn chính sách → 422.

## Tiêu chí xong

- [ ] Mọi mục của bước 55 trong ROADMAP `[x]`.
- [ ] `./gate.sh` đầy đủ xanh.
- [ ] `docs/openapi.json` đã sinh lại; mỗi endpoint đủ mô tả.
- [ ] `locales/vi.json` có mọi mã lỗi mới.
- [ ] Tài liệu EN + VI cho người làm giao diện, cùng commit.
- [ ] CHANGELOG ở `[Unreleased]`.
- [ ] Quyết định riêng của bước (vd video trong banner, giới hạn điều kiện của quy tắc bộ sưu tập) ghi DECISIONS;
      banner gian hàng người bán đã chốt ở D-233.
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
- **Đảo banner ngẫu nhiên mỗi lần tải** (D-236): phá cache CDN, người xem thấy nhảy. Thứ tự cố định theo `position`.
- **Gọi từng vị trí một** (D-236): trang có 6 vị trí thành 6 lời gọi — dùng `GET /public/placements?keys=`.
- **Gieo lại bộ vị trí chuẩn đè lên chỉnh sửa của khách** (D-236): chỉ gieo khi tạo khách; khách đã xoá thì thôi.
- **Kiểm cỡ ảnh bằng cách tải ảnh về:** dùng kích thước `media` đã lưu lúc tải lên, không đọc lại tệp.
- **Banner trỏ tới sản phẩm bị xoá:** `resolve` trả thiếu ID đó thì bỏ banner khỏi kết quả công khai. Không
  báo lỗi cho người xem.
- **Banner gian hàng lọt ra trang chung của sàn** (D-233): truy vấn vị trí `site` lấy banner của `org_id` và tổ
  tiên — nút `seller` không bao giờ là tổ tiên của nút sàn, nhưng vẫn kiểm `audience` khi lưu và viết test riêng.
  Vị trí `seller_store.*` không lấy tổ tiên: banner của sàn không chen vào gian hàng.
- **Người bán tự duyệt** (D-233): kiểm quyền duyệt ở nút cha của nút `seller`, không ở nút của banner.
- **Sửa sau khi duyệt** (D-233): mọi trường hiển thị đổi → về `pending`; chỉ đổi `position` thì giữ `approved`.
- **Đếm hạn mức bằng `COUNT(*)` rồi mới chèn:** hai request song song cùng lọt. Dùng bộ đếm với `UPDATE` có
  điều kiện trong cùng transaction (D-233).
- **Nhầm với quảng cáo trong sàn (bước 64)** (D-233): banner gian hàng không có giá, không đấu giá, không
  `sponsored`; muốn hiện ở trang chung của sàn thì dùng `ads`.

## Căn cứ pháp lý

- **Quảng cáo phải trung thực, điều kiện khuyến mại phải minh bạch.** Luật Bảo vệ quyền lợi người tiêu dùng
  19/2023/QH15 (từ 01/7/2024); Luật Quảng cáo 16/2012/QH13 và văn bản sửa đổi — cần đối chiếu văn bản gốc.
  - Banner trỏ tới chương trình đã kết thúc phải tự ẩn.
  - Chữ trên banner do người soạn chịu trách nhiệm; core không tự sinh.
- **Nội dung do AI gợi ý mang dấu `ai_assisted`** (Luật Trí tuệ nhân tạo 134/2025/QH15, từ 01/3/2026 —
  cần đối chiếu; bước 37).
- **Chân trang website bán hàng phải có thông tin chủ sở hữu** (NĐ 52/2013 sửa bởi NĐ 85/2021 và văn bản
  hướng dẫn Luật TMĐT 2025 — cần đối chiếu). Lấy từ hồ sơ pháp lý của bước 37, không nhập lại ở đây.
- **Banner gian hàng người bán** (D-233): sàn TMĐT có trách nhiệm kiểm soát thông tin người bán đăng trên sàn và
  gỡ thông tin vi phạm (NĐ 52/2013 sửa bởi NĐ 85/2021; Luật Thương mại điện tử 122/2025 — cần đối chiếu văn bản
  gốc) → chính sách duyệt là dữ liệu của sàn. Thông tin trên banner phải trung thực (Luật 19/2023/QH15). Banner
  gian hàng là thông tin của người bán trong gian hàng, không phải dịch vụ quảng cáo của sàn; ranh giới với
  Luật Quảng cáo 16/2012/QH13 — cần đối chiếu / người có chuyên môn pháp lý.

## Agent làm bước này tự quyết

- **Banner trong gian hàng người bán của sàn:** đã chốt ở D-233, làm ở mục 7 — xem "Banner trong gian hàng người
  bán trên sàn". Tự chọn: mặc định của chính sách khi sàn chưa đặt (khuyến nghị duyệt trước, 5 banner).
- **Có cho video trong banner không:** khuyến nghị chỉ ảnh ở bước này.
- **Giới hạn số điều kiện của quy tắc bộ sưu tập:** khuyến nghị 10.
- Ghi các lựa chọn trên vào `docs/DECISIONS.md`.
