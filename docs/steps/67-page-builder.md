# Bước 67 — Trang dựng từ khối, AI tạo trang, bộ giao diện website (`content`)

> Lộ trình: `docs/ROADMAP.md` bước 67 · Luật chung: `docs/steps/README.md` · Quyết định: D-229 (thiết kế bước
> này), D-026 (core chỉ có API), D-102 (thân bài từ chối mặc định), D-109 (AI chỉ thành nháp), D-123 (sổ đối
> tượng), D-208 (nội dung theo website và nút)

## Mục tiêu

Người không biết code tự làm trang đích (landing page) ngay trong CMS, không cần deploy, theo hai cách cùng sửa
**một** bản dữ liệu:

- **Tạo trang bằng cách trò chuyện ("vibe coding").** Người dùng mô tả trang, AI ghép trang từ các khối có sẵn.
  Người dùng nói tiếp để sửa ("đổi màu đỏ", "thêm nút mua ngay"); mỗi lần sửa là một bản nháp, hoàn tác được.
- **Kéo thả (page builder).** Giao diện kéo thả nằm ở repo giao diện (D-026, D-097); bước này cho dữ liệu và API:
  kiểm bố cục, lưu nháp và lịch sử, xem trước, hẹn giờ đăng, khoá khi hai người cùng sửa.

Trang đăng xong có link công khai ngay: tên miền của website + đường dẫn của trang (`content` đã có). Trang gắn
được chức năng thật — form, đăng ký nhận tin, giỏ hàng, lưu voucher — vì đó là **khối do module cung cấp**, không
phải code AI tự viết.

Kèm theo trong bước: bộ giao diện chung của website (màu, chữ, khoảng cách), thư viện trang mẫu, khu dùng chung
nhiều trang, kiểm tra sức khoẻ website, và bộ nháp chiến dịch tạo từ một chương trình khuyến mại.

## Bắt đầu khi

- **Bước 51 xong:** sổ đối tượng có `image` (P6), hộp việc (P2) cho kiểm tra sức khoẻ, lệnh cần duyệt (P12).
- **Bước 34:** website là nút có tên miền (D-208), cài đặt kế thừa cho bộ giao diện.
- **Bước 37:** dấu `ai_assisted`, sổ đồng ý (popup thu email), chân trang pháp lý (thông tin chủ sở hữu, logo Bộ
  Công Thương).
- Module thương mại (41–49) đến sau và **tự khai khối của mình** ở bước của chúng; bước này không chờ chúng.

## Phạm vi

- **Kernel:** sổ khối `Module::blocks()` (cài đặt mặc định trả rỗng — D-038), trait `BlockData` để module trả dữ
  liệu cho khối, kiểm props theo JSON Schema.
- **`content`:**
  - trang có bố cục khối (thân `layout`, cạnh thân `rich` sẵn có);
  - bộ giao diện của website, khu dùng chung, trang mẫu;
  - đọc công khai trả bố cục kèm dữ liệu khối;
  - AI tạo và sửa trang;
  - kiểm tra sức khoẻ website, bộ nháp chiến dịch;
  - khối nền của `content`.
- **`forms`, `newsletter`, `comments`, `media`:** khai khối của mình (nhúng form, ô đăng ký, bình luận, thư viện
  ảnh).

**Ngoài phạm vi:**
- giao diện kéo thả và bộ vẽ khối (repo giao diện);
- khối "mã tuỳ chỉnh" (HTML / CSS / JavaScript tự viết) — để sau, xem "Agent làm bước này tự quyết";
- thử A/B (sau bước 57);
- AI tạo ảnh (cần loại nhà cung cấp AI mới).

## Thiết kế

### 1. Sổ khối (kernel)

```rust
// autoxide_kernel::blocks — phác thảo, tra API thật bằng skill `check-library-api`
#[non_exhaustive]
pub struct BlockType {
    pub name: &'static str,          // `<module>.<khối>`, vd `catalog.product_grid`, `forms.embed`
    pub category: BlockCategory,     // Layout / Text / Media / Commerce / Lead / Social / Legal
    pub label_key: &'static str,     // i18n key: tên khối trong bộ chọn
    pub props_schema: &'static str,  // JSON Schema (draft 2020-12) của props — nguồn kiểm duy nhất
    pub ai_hint_key: &'static str,   // i18n key: một câu cho AI biết khi nào dùng khối
    pub has_data: bool,              // cần `BlockData` lúc đọc công khai (lưới sản phẩm…) hay không
    pub children: Children,          // None / Blocks (khối chứa khối: cột, tab)
    pub feature: Option<&'static str>, // tính năng của gói (bước 35) cần có
}
#[async_trait::async_trait]
pub trait BlockData: Send + Sync {
    /// Dữ liệu cho nhiều khối cùng loại trong MỘT lần gọi (không N+1). Chỉ trả dữ liệu công khai;
    /// tham chiếu không còn công khai → `None` (giao diện ẩn khối, không lỗi cả trang).
    async fn resolve(&self, tenant: &TenantContext, site: OrgId, locale: &str, items: &[BlockRef])
        -> Result<Vec<Option<serde_json::Value>>, Error>;
}
```

- Module khai khối qua `Module::blocks()` và gắn `BlockData` vào sổ lúc `start` (chỗ giữ của kernel như
  `ResourceDirectory`), nên `content` đăng ký sớm vẫn thấy khối của `catalog` đăng ký sau.
- Gate kiểm: tên khối duy nhất, `props_schema` là JSON Schema hợp lệ, mọi `label_key` / `ai_hint_key` có bản dịch.
- `GET /api/v1/kernel/blocks` (người có `content.pages.write`): danh sách khối của module **đang bật** và có tính
  năng trong gói, kèm schema — giao diện dựng bảng thuộc tính từ đây, AI cũng đọc đúng danh sách này.

Khối nền tối thiểu (khai ở `content`, trừ khi ghi khác):

| Nhóm | Khối |
|---|---|
| Bố cục | `section` (nền, khoảng cách, chiều rộng), `columns` (2–4 cột, xếp chồng trên điện thoại), `spacer`, `divider` |
| Chữ | `heading`, `rich_text` (dùng lại bộ kiểm ProseMirror của D-102), `button` (link nội bộ / ngoài, kiểu), `faq` (cặp hỏi–đáp, sinh JSON-LD FAQPage) |
| Media | `image` (ID `media`), `gallery`, `video` (ID `media` hay nhúng từ host cho phép như D-102), `hero` (ảnh nền + tiêu đề + nút) |
| Thu khách | `announcement_bar`, `countdown` (tới một mốc giờ UTC), `popup` (thu email: luôn kèm ô đồng ý của bước 37, tần suất hiện, không chặn đóng) |
| Nhúng | `content.embed` (bản ghi qua sổ đối tượng, P6), `content.posts` (danh sách bài theo chuyên mục) |
| Module khác | `forms.embed`, `newsletter.signup`, `comments.thread`; bước 41+: `catalog.product_grid`, `catalog.product_card`, `catalog.collection`, `orders.add_to_cart`, `orders.buy_now`, `promotions.flash_sale`, `promotions.voucher_claim`, `orders.reviews` |
| Pháp lý | `legal.footer` (khai ở `compliance`): thông tin chủ sở hữu, logo Bộ Công Thương — **tự gắn** cuối mọi trang của website bán hàng, không xoá được |

### 2. Bố cục trang

- Trang (`content` entry `kind = page`) có `body_kind` ∈ `rich` (như cũ) / `layout`. Thân `layout` là JSON:
  `{"type": "layout", "schema": 1, "sections": [ { "id", "block", "props", "children": [...] } ]}`.
- Kiểm **từ chối mặc định** như D-102: khối phải có trong sổ và đang bật; props đúng schema của khối (kiểm trong
  Rust bằng thư viện JSON Schema — tra `check-library-api`, ghi `STACK.md`); không HTML, không `style` tự do, không
  URL `javascript:`; ảnh chỉ ID `media`; link theo luật link của D-102; `id` của khối duy nhất trong trang; giới hạn
  `MAX_BLOCKS` (500), độ sâu 6, kích thước JSON 256 KB. Mọi lỗi báo một lần, kèm đường dẫn JSON (D-083).
- **Theo cỡ màn hình:** props nào schema đánh dấu `responsive` thì nhận `{base, md?, lg?}`; ẩn khối theo cỡ màn
  hình bằng `visibility`.
- **Ngôn ngữ:** mỗi bản dịch của trang có bố cục riêng (như thân `rich`); "sao bố cục sang ngôn ngữ khác" giữ
  khung, đổi chữ bằng bộ dịch AI sẵn có (tách mẩu, giữ chỗ).
- Lịch sử, xem trước bằng token, hẹn giờ đăng, đường dẫn, chuyển hướng: dùng nguyên cơ chế của `content`.
- **Sửa từng phần:** `PATCH /entries/{id}/layout` nhận JSON Patch (RFC 6902) kèm `If-Match`; mỗi patch kiểm lại cả
  trang; giao diện kéo thả và AI cùng dùng.
- **Khoá sửa:** `POST /entries/{id}/edit-lock` (giữ 2 phút, gia hạn bằng gọi lại; người khác thấy ai đang sửa);
  `If-Match` vẫn là chốt cuối — khoá chỉ để báo, không thay kiểm phiên bản.

### 3. Bộ giao diện của website (theme tokens)

- `content_themes` theo website (nút, kế thừa — chuỗi đặt, cửa hàng ghi đè): màu (primary, secondary, nền, chữ,
  thành công / lỗi), font (chọn trong danh sách cho phép, không tải font lạ), cỡ chữ, bo góc, khoảng cách,
  logo (ID `media`). Kiểm độ tương phản chữ / nền đạt WCAG AA, không đạt thì cảnh báo (không chặn).
- Khối chỉ tham chiếu token (`color: "primary"`), không mã màu tự do — trang do AI tạo vẫn đúng thương hiệu; đổi
  token là mọi trang đổi theo.
- `GET /public/theme` (ETag) cho giao diện.

### 4. Khu dùng chung và trang mẫu

- **Khu dùng chung** (`content_shared_sections`): một khu dùng ở nhiều trang (vd chân trang khuyến mại). Trang giữ
  khối `content.shared_section {id}`; sửa khu là mọi trang đổi; xoá khu đang dùng → 409 kèm danh sách trang.
- **Trang mẫu** (`content_page_templates`): bố cục mẫu theo ngành (bán lẻ, F&B, dịch vụ, sự kiện, tuyển dụng).
  Mẫu dùng chung đi kèm bản phát hành (`modules/content/data/templates/*.json`, nạp bằng task); khách lưu trang của
  mình thành mẫu riêng. Tạo trang từ mẫu = chép bố cục, bỏ khối của module chưa bật.

### 5. Đọc công khai

- `GET /api/v1/content/public/resolve?path&locale` (đã có) trả thêm `layout` và `blocks_data`: kernel gom khối có
  `has_data` theo loại, gọi `BlockData::resolve` **một lần mỗi loại** (song song, có hạn thời gian), ghép kết quả.
  Một khối lỗi / hết giờ → `null` cho khối đó, trang vẫn trả.
- Cache: `ETag` theo (phiên bản trang, phiên bản theme, "phiên bản dữ liệu" mà mỗi `BlockData` báo — vd
  `catalog` báo số phiên bản giá); `Cache-Control` ngắn cho trang có khối dữ liệu.
- Trang không còn công khai → 404; khối của module đã tắt → bỏ khỏi kết quả công khai, quản trị thấy cảnh báo.

### 6. AI tạo và sửa trang (D-109, D-229)

- `POST /entries/{id}/ai/compose` `{instruction, locale}` (tạo mới từ mô tả hay từ mẫu) và
  `POST /entries/{id}/ai/edit` `{instruction, selection?: [block_id]}` (sửa tiếp). Chạy bằng job (gọi
  `TextModel` ngoài transaction), trả `202` + job id; kết quả là **bản nháp** (revision mới, `ai_assisted`), không
  bao giờ tự đăng.
- Mô hình nhận trong `instructions`: danh sách khối đang bật (tên, `ai_hint`, schema props), token của theme, luật
  "chỉ dùng khối trong danh sách"; trong `input`: mô tả của người dùng, bố cục hiện tại (khi sửa), dữ liệu tham
  khảo người dùng chỉ định (sản phẩm, chương trình — qua tool chỉ đọc). Kết quả theo JSON Schema: `create` trả
  bố cục, `edit` trả JSON Patch. Kết quả **luôn qua bộ kiểm của mục 2**; sai thì gọi lại tối đa 2 lần kèm lỗi, vẫn
  sai thì báo người dùng.
- Ảnh: chỉ dùng ảnh có trong `media` của khách (AI chọn theo mô tả / alt) hay để khối ảnh trống chờ người dùng
  chọn. Không bịa số liệu: giá, tồn, ngày khuyến mại luôn lấy từ khối dữ liệu, không viết cứng vào chữ.
- Lịch sử trò chuyện của trang (`content_ai_threads`): lời yêu cầu, revision kết quả, ai yêu cầu — để hoàn tác
  theo từng bước và để người khác xem trang được làm thế nào. Không gửi dữ liệu cá nhân của người mua cho mô hình.
- Hạn mức AI theo gói (bước 35): `content.ai_page_generations` mỗi tháng.

### 7. Bộ nháp chiến dịch

`POST /campaign-drafts` `{promotion_id | collection_id, locale, channels: [page, banner, newsletter]}` (job): từ
một chương trình khuyến mại hay bộ sưu tập, AI tạo **nháp** trang đích (khối `promotions.flash_sale`,
`catalog.product_grid`…), banner (bước 55) và chiến dịch `newsletter` — mỗi thứ là bản nháp ở module của nó, gọi
qua tool MCP với quyền người yêu cầu. Người duyệt từng thứ rồi mới đăng / gửi.

### 8. Kiểm tra sức khoẻ website

Job `content.site_health` hằng ngày theo website, kết quả vào hộp việc (bước 51 P2) qua `WorkSource`:
- trang / bài thiếu tiêu đề hay mô tả SEO, tiêu đề trùng, ảnh thiếu alt theo ngôn ngữ đang bật;
- link nội bộ gãy (đường dẫn không còn công khai), khối tham chiếu bản ghi đã ẩn / xoá, khối của module đã tắt;
- đường dẫn 404 nhiều lần (đếm từ `/public/resolve` không thấy, gộp theo ngày, không lưu IP) → gợi ý tạo chuyển
  hướng tới trang gần nhất (theo slug tương tự), người bấm tạo;
- trang đã hết hạn chiến dịch (khối `countdown` đã qua mốc) còn đang công khai.
Mỗi mục có `fix` gợi ý (endpoint sẵn có để sửa); AI chỉ gợi ý nội dung SEO / alt bằng endpoint sẵn có.

### Bảng dữ liệu (mọi bảng có `tenant_id`, index bắt đầu bằng nó)

| Bảng | Cột chính | Khoá / index |
|---|---|---|
| `content_entries` (thêm cột) | `body_kind` (`rich`/`layout`, mặc định `rich`) | — |
| `content_themes` | `id`, `tenant_id`, `org_id`, `tokens` (JSON trong `text`), `version`, `updated_by`, `updated_at` | unique (`tenant_id`,`org_id`) |
| `content_shared_sections` | `id`, `tenant_id`, `org_id`, `name`, `layout` (JSON), `version`, `updated_at` | (`tenant_id`,`org_id`) |
| `content_shared_section_uses` | `section_id`, `entry_id` (cập nhật khi lưu trang) | unique (`tenant_id`,`section_id`,`entry_id`); (`tenant_id`,`entry_id`) |
| `content_page_templates` | `id`, `tenant_id?` (NULL = mẫu dùng chung), `org_id?`, `industry`, `name_key`/`name`, `layout`, `preview_media_id?`, `pack_version?` | (`tenant_id`,`industry`) |
| `content_edit_locks` | `entry_id`, `tenant_id`, `user_id`, `expires_at` | pk (`tenant_id`,`entry_id`) — chiếm bằng `INSERT … ON CONFLICT` có điều kiện hết hạn |
| `content_ai_threads` | `id`, `tenant_id`, `entry_id`, `locale`, `turns` (JSON: yêu cầu, revision_id, job_id, người, lúc) | (`tenant_id`,`entry_id`) |
| `content_not_found_paths` | `tenant_id`, `org_id`, `path_hash`, `path`, `day`, `hits` | unique (`tenant_id`,`org_id`,`path_hash`,`day`) — cộng bằng `rows::adjust`, giữ 30 ngày |

`content_page_templates` dòng `tenant_id` NULL là dữ liệu dùng chung (mặt điều khiển, như D-198) — ghi tên bảng
vào danh sách ở `docs/steps/README.md`.

### API REST (dưới `/api/v1/content`, trừ khi ghi khác)

| Method + path | Permission | Ghi chú |
|---|---|---|
| `GET /api/v1/kernel/blocks` | `content.pages.write` | khối đang dùng được, kèm schema |
| `PUT /entries/{id}/layout` · `PATCH /entries/{id}/layout` | `content.pages.write` | thay cả / JSON Patch; `If-Match` |
| `POST/DELETE /entries/{id}/edit-lock` | `content.pages.write` | giữ / nhả khoá sửa |
| `POST /entries/{id}/ai/compose` · `POST /entries/{id}/ai/edit` · `GET /entries/{id}/ai/thread` | `content.pages.ai` | 202 + job; kết quả là revision nháp |
| `POST /entries/{id}/layout/copy-to/{locale}` | `content.pages.write` | sao khung sang bản dịch, tuỳ chọn dịch bằng AI |
| `GET/PUT /themes/{org_id}` · `GET /public/theme` | `content.themes.manage` · công khai | token kế thừa; ETag |
| `GET/POST /shared-sections` · `GET/PUT/DELETE /shared-sections/{id}` | `content.pages.write` | xoá đang dùng → 409 |
| `GET /page-templates?industry` · `POST /page-templates` (lưu trang thành mẫu) · `POST /entries/from-template` | `content.pages.write` | |
| `POST /campaign-drafts` · `GET /campaign-drafts/{job_id}` | `content.pages.ai` | job, kết quả là danh sách bản nháp ở từng module |
| `GET /site-health?org_id` · `POST /site-health/{org_id}/run` | `content.read` · `content.pages.write` | kết quả gần nhất; chạy ngay (giới hạn 1 lần / giờ) |

### Tool MCP

`content.list_blocks` (`read_only`), `content.get_page_layout` (`read_only`), `content.update_page_layout`,
`content.compose_page` (nháp), `content.edit_page` (nháp), `content.create_campaign_drafts` (nháp),
`content.site_health` (`read_only`). Cùng service với REST; agent bên ngoài dựng trang được với đúng quyền như
người.

### Event

`content.layout_updated` (entry, revision, `ai_assisted`), `content.theme_updated`, `content.shared_section_updated`
(kèm số trang bị ảnh hưởng), `content.ai_draft_ready`, `content.site_health_checked`. Không mang nội dung trang.

### Cấu hình (`AUTOXIDE_CONTENT_`)

`LAYOUT_MAX_BLOCKS` (500), `LAYOUT_MAX_BYTES` (262144), `EDIT_LOCK_SECONDS` (120), `AI_MAX_RETRIES` (2),
`BLOCK_DATA_TIMEOUT_MS` (800), `SITE_HEALTH_NOT_FOUND_MIN_HITS` (5). Sinh lại `.env.example`.

### Tính năng và hạn mức (bước 35)

Tính năng: `content.page_builder`, `content.ai_pages`, `content.shared_sections`. Hạn mức:
`content.ai_page_generations` (mỗi tháng), `content.layout_pages`.

### Permission

`content.pages.write`, `content.pages.ai`, `content.themes.manage` (thêm vào bộ của `content`; quyền theo độ sâu ở
nút website — D-208).

### Mã lỗi (`locales/vi.json`)

| Mã | HTTP | Tiếng Việt |
|---|---|---|
| `content.layout_invalid` | 422 | Bố cục chưa hợp lệ: {count} lỗi (xem `errors`). |
| `content.block_unknown` | 422 | Khối `{block}` không có hoặc module của nó đang tắt. |
| `content.block_not_in_plan` | 403 | Gói hiện tại chưa có khối `{block}`. |
| `content.layout_too_large` | 413 | Trang vượt giới hạn {max} khối hoặc {bytes} byte. |
| `content.edit_locked` | 409 | {user} đang sửa trang này tới {until}. |
| `content.shared_section_in_use` | 409 | Khu dùng chung đang được {count} trang dùng. |
| `content.ai_unavailable` | 503 | Chưa bật AI cho máy chủ này. |
| `content.ai_result_invalid` | 422 | AI chưa tạo được bố cục hợp lệ sau {tries} lần; hãy mô tả cụ thể hơn. |
| `content.legal_footer_required` | 422 | Trang của website bán hàng phải giữ chân trang pháp lý. |

## Thứ tự làm

1. Kernel: `blocks` (kiểu, `Module::blocks`, chỗ giữ `BlockData`, kiểm schema, gate kiểm bản dịch), `GET /kernel/blocks`.
2. `content`: `body_kind`, bộ kiểm bố cục, khối nền, `PUT/PATCH layout`, khoá sửa; test.
3. Đọc công khai có `blocks_data`, ETag, khối lỗi không hỏng trang; `forms`, `newsletter`, `comments`, `media`
   khai khối; `compliance` khai `legal.footer`.
4. Theme, khu dùng chung, trang mẫu (gói mẫu dùng chung + task nạp).
5. AI: compose / edit bằng job, kiểm và thử lại, lịch sử trò chuyện; bộ nháp chiến dịch.
6. Kiểm tra sức khoẻ website, nguồn hộp việc.
7. Tool MCP, OpenAPI, `.env.example`, `docs/PAGES.md` + `docs/PAGES.vi.md` (khai khối cho module — mục mới trong
   MODULE_GUIDE EN + VI), CHANGELOG, ROADMAP.

## Test bắt buộc (cả SQLite và Postgres)

- Kiểm: `khoi_la_bi_tu_choi`, `props_sai_schema_bao_du_loi_mot_lan`, `khong_nhan_html_va_javascript_url`,
  `qua_gioi_han_khoi`, `khoi_cua_module_tat_bi_tu_choi_khi_luu`, `json_patch_kiem_lai_ca_trang`,
  `if_match_cu_bi_tu_choi`.
- Đồng thời: `hai_nguoi_cung_giu_khoa_chi_mot_duoc`, `hai_patch_cung_phien_ban_chi_mot_thang`.
- Công khai: `blocks_data_mot_lan_goi_moi_loai` (đếm lời gọi `BlockData` giả), `khoi_loi_tra_null_trang_van_tra`,
  `ban_ghi_het_cong_khai_khong_lo`, `etag_doi_khi_theme_doi`, `chan_trang_phap_ly_tu_gan_khong_xoa_duoc`.
- AI (`FakeTextModel`): `ai_tao_trang_chi_la_ban_nhap`, `ai_dung_khoi_la_thi_thu_lai_roi_bao_loi`,
  `ai_sua_tra_json_patch`, `khong_gui_du_lieu_nguoi_mua_cho_mo_hinh`, `het_han_muc_ai_theo_goi`.
- Khác: `xoa_khu_dung_chung_dang_dung_409`, `tao_trang_tu_mau_bo_khoi_module_tat`, `suc_khoe_tim_link_gay_va_404`,
  `ro_ri_giua_hai_website_cung_khach`, `tool_mcp_cung_permission_nhu_rest`.

## Tiêu chí xong

- [ ] Mọi mục của bước 67 trong ROADMAP được phủ; khối của `content`, `forms`, `newsletter`, `comments`, `media`,
      `compliance` có mặt và có bản dịch.
- [ ] Đo: `GET /public/resolve` của trang 50 khối với 5 loại khối dữ liệu (bộ giả) — p99 ghi vào
      `docs/BENCHMARKS.md`, trên cả hai database.
- [ ] `docs/PAGES.md` + `.vi.md`, MODULE_GUIDE (EN + VI), OpenAPI, `.env.example`, CHANGELOG.
- [ ] DECISIONS: thư viện JSON Schema; giới hạn khối; cách tính ETag có dữ liệu.
- [ ] `./gate.sh` đầy đủ xanh; ROADMAP; commit; phát hành theo `docs/RELEASING.md`.

## Chỗ dễ sai

- **AI viết code / HTML:** không có đường nào để chữ của mô hình thành HTML hay script. Kết quả AI là bố cục khối,
  qua cùng bộ kiểm với người. Chữ trong khối `rich_text` đi qua bộ kiểm ProseMirror của D-102.
- **Số liệu trong chữ:** AI không được viết giá, phần trăm giảm, số suất vào chữ tĩnh — dữ liệu đổi mà chữ không
  đổi là thông tin sai cho người tiêu dùng (Luật 19/2023). Hướng dẫn trong `instructions` và kiểm: khối chữ chứa
  số tiền khớp giá đang có thì cảnh báo trong bản nháp.
- **N+1 lúc đọc công khai:** gom theo loại khối; `BlockData` nhận cả danh sách.
- **Gọi AI trong transaction / trong request:** compose / edit chạy bằng job; ghi revision trong transaction ngắn
  sau khi có kết quả.
- **Khoá sửa không phải chốt:** khoá hết hạn hay bị bỏ qua thì `If-Match` vẫn chặn ghi đè.
- **Schema khối đổi:** khối thêm props thì mặc định cho props mới; bỏ hay đổi nghĩa props là khối mới
  (`…_v2`), khối cũ vẫn đọc được (như luật API, D-059).
- **Theme kế thừa:** đọc token hiệu lực theo cây (cài đặt kế thừa của bước 34), ETag theo phiên bản hiệu lực.
- **Popup thu email:** luôn có ô đồng ý và liên kết chính sách; không gửi gì khi chưa đồng ý (bước 37).

## Căn cứ pháp lý

Đối chiếu qua nguồn tổng hợp — **cần đối chiếu văn bản gốc / người có chuyên môn pháp lý** trước khi chạy thật:

| Nội dung | Văn bản | Ghi chú |
|---|---|---|
| Thông tin chủ sở hữu, logo thông báo / đăng ký trên website bán hàng | NĐ 52/2013 sửa bởi NĐ 85/2021; Luật Thương mại điện tử 122/2025 | khối `legal.footer` tự gắn |
| Nhãn nội dung do AI tạo | Luật Trí tuệ nhân tạo 134/2025/QH15 và văn bản hướng dẫn | `ai_assisted` trên revision, API trả để giao diện gắn nhãn |
| Thông tin trung thực về giá, khuyến mại trên trang | Luật 19/2023/QH15; Luật Thương mại 2005 | giá, giảm lấy từ khối dữ liệu |
| Thu email qua popup | Luật 91/2025/QH15; NĐ 91/2020 | đồng ý trước, theo mục đích |

## Agent làm bước này tự quyết

- Thư viện kiểm JSON Schema (ưu tiên crate đã ổn định, chạy không cần mạng); ghi `STACK.md` và DECISIONS.
- Bộ khối nền cuối cùng và tên props; bộ trang mẫu ban đầu.
- Cách tính "phiên bản dữ liệu" của khối dữ liệu cho ETag.
- Khối "mã tuỳ chỉnh": **không làm ở bước này**. Muốn mở sau thì cần quyết định riêng: tắt mặc định, chỉ HTML /
  CSS đã lọc, hiển thị trong iframe `sandbox` với CSP chặt, permission riêng.
