# Bước 68 — Menu của website (`menus`)

> Lộ trình: `docs/ROADMAP.md` bước 68 · Quyết định: D-238 (chủ dự án quyết) · Đề bài: `docs/prompts/menus-module.md`
> · Đặc tả liên quan: `docs/steps/51-ui-contracts.md` (P6 sổ đối tượng; P1 sổ điều hướng của trang quản trị — **khác**
> module này), `docs/steps/41-catalog.md` (`catalog.category`), `docs/steps/55-storefront.md` (mặt tiền)

## Mục tiêu

Người quản trị dựng menu của website: thanh danh mục, bảng xổ lớn (mega menu), menu chân trang, menu phụ. Mục
menu trỏ tới một địa chỉ, tới bản ghi của **bất kỳ module nào** (bài viết, danh mục nội dung, danh mục sản phẩm,
khuyến mại, gian hàng, cửa hàng), hoặc là nhóm tiêu đề không có đích (một cột của bảng xổ). Mục trỏ tới danh mục
có thể tự lấy danh mục con. Website đọc một endpoint công khai trả cây đã giải sẵn: nhãn, đường dẫn, biểu tượng,
nhãn nổi bật còn hạn, ô gợi ý có ảnh.

Module **không** biết sản phẩm, danh mục hay bài viết là gì: mọi thứ đi qua sổ đối tượng của kernel (D-123).

## Ranh giới — không làm trùng

| Việc | Ở đâu | `menus` làm gì |
|---|---|---|
| Menu của trang quản trị (phân hệ, quyền, gói) | `Module::navigation`, bước 51 P1 | Không đụng tới. Hai thứ không gộp |
| Khối "Bán chạy" trong bảng xổ | Danh sách sản phẩm công khai của `catalog` (lọc theo danh mục, sắp `best_selling`, bước 41) | Không làm. Mục trả `ref` (loại, ID) để giao diện gọi `catalog` khi người dùng mở bảng xổ |
| Đầu trang, chân trang của website | `content_site_regions` (bước 67, D-234) | Khai khối `menus.menu {key}` (thay `content.menu` của D-234) để đặt menu vào đầu trang, chân trang |
| Menu cũ của `content` (bước 17) | `content_menus`, deprecated | Nhập sang bằng lệnh; không đọc bảng của `content` |

## Bắt đầu khi

- Kernel: phần sổ đối tượng của D-238 (`Resource.image`, `resolve_many`, `children`, `has_children`) — **đã làm**.
- Làm trước bước 33 theo lệnh của chủ dự án (14/11/2026). Phần theo khách và theo website làm cùng bước 33, 34
  (mục "Việc của bước sau").
- Thứ tự đăng ký: sau `media` (ảnh của ô gợi ý) và `content` (menu cũ). Loại đối tượng của module đăng ký sau
  (`catalog`, `promotions`, `marketplace`) vẫn tra được vì sổ đối tượng đọc lúc chạy.

## Thiết kế

### Bảng

`menus_menus`:

| Cột | Kiểu | Ghi chú |
|---|---|---|
| `id` | UUIDv7 | |
| `key` | chuỗi ≤ 64 | `main`, `footer`… chữ thường, số, `-`, `_` |
| `locale` | chuỗi ≤ 35 | BCP 47 chuẩn hoá, thuộc `AUTOXIDE_MENUS_LOCALES` |
| `created_at`, `updated_at` | thời điểm UTC | `updated_at` là phiên bản (`ETag`) |

Unique (`key`, `locale`). Bước 33 thêm `tenant_id`, bước 34 thêm `org_id` (website hay nút sở hữu, H1) và đổi unique
thành (`tenant_id`, `org_id`, `key`, `locale`).

`menus_items` (index (`menu_id`, `position`)):

| Cột | Ghi chú |
|---|---|
| `id`, `menu_id`, `parent_id?`, `position` | cây, thứ tự toàn menu (cha trước con) |
| `label` | ≤ 200 ký tự; rỗng khi có `ref` thì lấy tiêu đề của đối tượng theo ngôn ngữ của menu |
| `url?` | `https://`, `http://`, `mailto:`, đường dẫn bắt đầu bằng đúng một `/`; từ chối `//host` và `\` (D-149) |
| `ref_kind?`, `ref_id?` | loại và ID trong sổ đối tượng; không foreign key |
| `auto_children`, `auto_children_limit?` | chỉ với `ref` có cây; 1–50, mặc định 12; không vượt tầng 4 |
| `icon?` | bộ cố định: `bag`, `bolt`, `book`, `calendar`, `cart`, `category`, `chat`, `fire`, `gift`, `grid`, `heart`, `help`, `home`, `info`, `mail`, `map_pin`, `news`, `percent`, `phone`, `search`, `shop`, `sparkles`, `star`, `store`, `tag`, `truck`, `user` |
| `highlight?` | `accent`, `brand`, `sale`, `muted` |
| `badge_text?`, `badge_tone?`, `badge_from?`, `badge_until?` | chữ 1–20 ký tự; tone `sale`, `brand`, `new`, `neutral`; ngoài [`from`, `until`) thì ẩn |
| `feature_kind?`, `feature_id?` | ô gợi ý trong bảng xổ: một đối tượng |

Mỗi mục có **tối đa một** trong `url` và `ref`; không có cả hai là nhóm tiêu đề. Giới hạn: 4 tầng (kể cả nhóm con
tự lấy), 200 mục mỗi menu.

### Sổ đối tượng (kernel, D-038)

```rust
pub struct Resource { …, pub image: Option<Uuid> }        // ID media
pub struct ResourceRef { pub kind: String, pub id: String }
trait ResourceResolver {
    async fn resolve(&self, id) -> Result<Option<Resource>>;                      // có từ D-123
    async fn resolve_many(&self, ids, locale) -> Result<HashMap<String, Resource>>; // mặc định: gọi resolve từng cái
    fn has_children(&self) -> bool;                                               // mặc định false
    async fn children(&self, id, locale, limit) -> Result<Vec<ResourceRef>>;      // mặc định rỗng
}
```

Loại đã đăng ký: `content.entry` (thêm `image`, tra theo lô), `content.term` (danh mục, thẻ; có cây; đường dẫn
`/<AUTOXIDE_CONTENT_POST_PATH_PREFIX>/category/<slug>` hay `/…/tag/<slug>`). Module sau đăng ký trong bước của mình:
`catalog.category` (41, có `children`), `promotions.promotion` (43), `marketplace.seller` (46), cửa hàng công khai
(34).

### API quản trị `/api/v1/menus`

| Method | Đường dẫn | Permission | Ghi chú |
|---|---|---|---|
| GET | `/` | `menus.read` | `Page<SiteMenuSummary>`, `limit` / `cursor` / `order` |
| POST | `/` | `menus.manage` | 201 + `Location`; trùng (`key`, `locale`) → 409 `menus.menu_exists` |
| GET | `/{id}` | `menus.read` | cây như đã lưu, `ETag` |
| PUT | `/{id}` | `menus.manage` | thay cả cây; `If-Match` bắt buộc (428 nếu thiếu, 412 nếu cũ) |
| DELETE | `/{id}` | `menus.manage` | 204 |

Lỗi khi lưu, gom một lần (D-083); mã chung theo loại lỗi khi mọi lỗi cùng loại, không thì `validation_failed`:

| Mã | Khi |
|---|---|
| `menus.ref_unknown_kind` | `ref.kind` / `feature.kind` chưa module nào đăng ký |
| `menus.ref_not_found` | đối tượng không có (`resolve_many` không trả) |
| `menus.children_unsupported` | `auto_children` trên loại không có cây |
| `menus.menu_exists` · `menus.menu_not_found` · `menus.public_menu_not_found` · `menus.import_source_missing` | như tên |

### API công khai `GET /api/v1/menus/public/{key}?locale=`

- Không cần đăng nhập. Thiếu menu ở `locale` thì dùng `AUTOXIDE_MENUS_DEFAULT_LOCALE`; `locale` của câu trả lời
  nói ngôn ngữ thật.
- Mỗi mục: `label`, `href` (null với nhóm tiêu đề), `ref` (loại, ID), `icon`, `highlight`, `badge` (đã lọc theo
  hạn), `feature` (`ref`, tiêu đề, tóm tắt, `href`, `image` chỉ khi ảnh công khai), `children`.
- Bỏ mục trỏ tới đối tượng không còn công khai (hay loại không còn module nào cung cấp), kèm mục con; bỏ nhóm
  tiêu đề không còn mục con.
- `auto_children`: `children()` của sổ đối tượng, giới hạn tính trước khi bỏ con không công khai; nằm sau mục con
  tự thêm.
- Tra theo lô: mỗi loại một lần `resolve_many`, ảnh một lần `Media::find_many_in`.
- **Cache (D-238):** bản đã giải trong bộ nhớ của tiến trình tới `AUTOXIDE_MENUS_PUBLIC_CACHE_SECONDS` (mặc định 60;
  0 tắt), rút ngắn tới mốc bật / tắt gần nhất của nhãn; mọi lệnh ghi của module xoá cache (có số thế hệ để bản
  giải dở không cất lại). `Cache-Control: public, max-age=<giây còn lại>`; `ETag` là dấu vân tay SHA-256 của cả
  thân, kernel tự trả 304. Đối tượng đổi tên / đổi đường dẫn thì hiện sau tối đa một thời hạn cache — không nghe
  event của module khác.

### Tool MCP

`menus.list_menus`, `menus.get_menu` (kèm `version`), `menus.create_menu`, `menus.replace_menu_items` (bắt buộc
`version`, như `If-Match`), `menus.delete_menu`. Mục nhận dạng JSON thô (client MCP không đọc được `$ref`) rồi kiểm
như thân REST; hình dạng mục ghi trong mô tả tool.

### Event

`menus.menu_created`, `menus.menu_replaced`, `menus.menu_deleted`; thân `{"before", "after"}` với `key`, `locale`,
số mục. Ghi cùng transaction.

### Cấu hình

`AUTOXIDE_MENUS_LOCALES` (mặc định `en,vi`), `AUTOXIDE_MENUS_DEFAULT_LOCALE` (`en`),
`AUTOXIDE_MENUS_PUBLIC_CACHE_SECONDS` (60, 0–3600). Bước 34 chuyển ngôn ngữ sang cài đặt của website.

### Chuyển từ menu của `content` (D-059)

- Bốn endpoint `/api/v1/content/menus…` và `/api/v1/content/public/menus/{key}` khai deprecated từ 2026-11-14,
  sunset 2027-05-14, kèm endpoint thay thế. Hành vi không đổi.
- `content` công bố `autoxide_content::api::LegacyMenus`; lệnh `autoxide task menus.import_content_menus` nhập menu
  chưa có cùng (`key`, `locale`). Chạy lại không nhân đôi, không đè menu đã sửa; ngôn ngữ ngoài
  `AUTOXIDE_MENUS_LOCALES` thì báo, không dừng. Mục trỏ bài → `content.entry`, mục trỏ danh mục / thẻ →
  `content.term`. Lệnh nhập không kiểm đối tượng còn có (đích đã mất thì menu công khai tự ẩn).
- Không có `content` thì `menus` vẫn chạy; lệnh nhập báo `menus.import_source_missing`.
- Qua ngày sunset: gỡ bốn endpoint, trait `LegacyMenus`, và bảng `content_menus`, `content_menu_items` bằng
  migration **mới** của `content`.

## Việc của bước sau

| Bước | Việc |
|---|---|
| 33 | `tenant_id` cho `menus_menus`, `menus_items`; mọi truy vấn lọc theo khách; cache theo khách; thêm khách vào `resolve`, `resolve_many`, `children` của sổ đối tượng |
| 34 | `org_id` (website hay nút sở hữu); tham số `?site=` của endpoint công khai; ngôn ngữ theo website; đăng ký loại cửa hàng công khai |
| 41 | `catalog.category` có `children` (thứ tự hiển thị của danh mục), `catalog.product` có `image` |
| 43 · 46 | `promotions.promotion`, `marketplace.seller` |
| 67 | khi có sổ khối: `menus` khai khối `menus.menu {key}` (`BlockData` trả cây như endpoint công khai), thay `content.menu` |

## Thứ tự làm

1. D-238; `menus` vào danh sách module của `CLAUDE.md`.
2. Kernel: `Resource.image`, `ResourceRef`, `resolve_many`, `has_children`, `children`; test.
3. `content`: `content.term`, `content.entry` tra theo lô và có ảnh, trait `LegacyMenus`, khai deprecated; test.
4. `menus`: migration, entity, cấu hình, kiểm mục, service, phần công khai và cache, REST, tool MCP, lệnh nhập, bản
   dịch; đăng ký trong `crates/app/src/modules.rs`.
5. Sinh lại `docs/openapi.json`, `.env.example`; hướng dẫn `docs/MENUS.md` + `.vi.md`; README, CHANGELOG.
6. `./gate.sh` đầy đủ.

## Test bắt buộc (cả SQLite và Postgres)

- Kiểm mục: dàn cây, gom mọi lỗi với đúng vị trí, tầng, số mục, `url` (`//host`, `\`, `javascript:`), nhãn rỗng chỉ
  được khi có `ref`, `auto_children` chỉ với `ref`.
- Giải đối tượng: nhãn rỗng lấy tiêu đề theo ngôn ngữ; đường dẫn; `ref` trong câu trả lời.
- Bỏ đối tượng không còn công khai kèm mục con; nhóm không còn con bị bỏ; ảnh riêng tư không hiện.
- `auto_children` có giới hạn, sau mục con tự thêm; `children_unsupported` trên loại không có cây.
- Nhãn hết hạn tự ẩn; cache hết hạn đúng mốc của nhãn.
- `If-Match` đồng thời: hai người thay cùng phiên bản, đúng một người nhận 412.
- Nhập menu cũ hai lần không nhân đôi, không đè menu đã sửa; ngôn ngữ không hỗ trợ thì báo.
- Thiếu ngôn ngữ thì dùng ngôn ngữ mặc định; ngôn ngữ không hỗ trợ 422; khoá không có 404.
- REST: 201 + `Location`, 409, 403, 428, 412, `ETag` + 304 + `Cache-Control` công khai, sửa xong `ETag` đổi.
- MCP: tạo qua tool đọc được qua REST; `version` cũ bị từ chối; tool ghi cần `menus.manage`.
- `content`: `content.term` (ngôn ngữ, đường dẫn, lô, con có giới hạn), ảnh của `content.entry`, `LegacyMenus`, header
  `Deprecation` / `Sunset` / `Link` trên menu cũ.

## Tiêu chí xong

Mọi mục của bước 68 trong lộ trình `[x]` trừ mục gỡ menu cũ (làm khi qua ngày sunset); `./gate.sh` xanh trên cả hai
database.

## Chỗ dễ sai

- Gọi sổ đối tượng (có thể gọi module khác) **trong** transaction ghi: kiểm đích chạy trước `begin_write`.
- ETag của menu công khai theo `updated_at` của menu: sai, vì đổi tên danh mục không chạm menu. Dùng dấu vân tay
  của cả câu trả lời.
- Cache giữ quá mốc nhãn hết hạn: rút thời hạn tới mốc gần nhất.
- Tra từng mục một: một menu 200 mục thành hàng trăm query. Dùng `resolve_many` theo loại.
- Trùng tên schema OpenAPI với menu cũ của `content` (`MenuResponse`…): kiểu của module này mang tên `SiteMenu…`.
- Đọc bảng `content_menus` từ `menus`: cấm (luật kiến trúc 1); dùng `LegacyMenus`.

## Agent làm bước này tự quyết

Đã quyết trong D-238: bộ biểu tượng, `highlight`, cache theo thời hạn, `resolve_many` / `has_children`, đường dẫn của
`content.term`, sunset sáu tháng, lệnh nhập bỏ qua menu đã có.
