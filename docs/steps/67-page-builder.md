# Bước 67 — Trang dựng từ khối, AI tạo trang, bộ giao diện website (`content`)

> Lộ trình: `docs/ROADMAP.md` bước 67 · Luật chung: `docs/steps/README.md` · Quyết định: D-229 (thiết kế bước
> này), D-231 (thử A/B, AI tạo ảnh, khối mã tự viết), D-026 (core chỉ có API), D-102 (thân bài từ chối mặc định), D-109 (AI chỉ thành nháp), D-123 (sổ đối
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
nhiều trang, kiểm tra sức khoẻ website, bộ nháp chiến dịch tạo từ một chương trình khuyến mại, thử A/B hai phiên
bản trang, AI tạo ảnh cho banner và ảnh minh hoạ, khối mã tự viết chạy cách ly (D-231).

## Bắt đầu khi

- **Bước 51 xong:** sổ đối tượng có `image` (P6), hộp việc (P2) cho kiểm tra sức khoẻ, lệnh cần duyệt (P12).
- **Bước 34:** website là nút có tên miền (D-208), cài đặt kế thừa cho bộ giao diện.
- **Bước 37:** dấu `ai_assisted`, sổ đồng ý (popup thu email), chân trang pháp lý (thông tin chủ sở hữu, logo Bộ
  Công Thương).
- Module thương mại (41–49) đến sau và **tự khai khối của mình** ở bước của chúng; bước này không chờ chúng.
- **Thử A/B** (mục 9) cần **bước 57** (`analytics`) để đếm lượt xem và chuyển đổi; các mục khác không chờ 57.

## Phạm vi

- **Kernel:** sổ khối `Module::blocks()` (cài đặt mặc định trả rỗng — D-038), trait `BlockData` để module trả dữ
  liệu cho khối, kiểm props theo JSON Schema.
- **`content`:**
  - trang có bố cục khối (thân `layout`, cạnh thân `rich` sẵn có);
  - bộ giao diện của website, khu dùng chung, trang mẫu;
  - đọc công khai trả bố cục kèm dữ liệu khối;
  - AI tạo và sửa trang;
  - kiểm tra sức khoẻ website, bộ nháp chiến dịch;
  - thử A/B hai phiên bản trang; khối mã tự viết;
  - khối nền của `content`.
- **Kernel + `ai`:** trait `ImageModel` (tạo ảnh), adapter trong module `ai`, `FakeImageModel` trong testkit.
- **`media`:** cờ `ai_generated` trên ảnh; `catalog` từ chối ảnh `ai_generated` làm ảnh sản phẩm (bước 41).
- **`forms`, `newsletter`, `comments`, `media`:** khai khối của mình (nhúng form, ô đăng ký, bình luận, thư viện
  ảnh).

**Ngoài phạm vi:**
- giao diện kéo thả và bộ vẽ khối (repo giao diện);
- thử A/B **giá hay khuyến mại** (chỉ thử bố cục và nội dung — D-231);
- AI tạo **ảnh sản phẩm** hay ảnh người thật (D-231).

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

### 9. Thử A/B hai phiên bản trang (D-231, sau bước 57)

- **Thí nghiệm** (`content_experiments`) trên một trang đã đăng: 2–4 phiên bản, mỗi phiên bản là một revision
  của trang (A = bản đang đăng); tỉ lệ chia theo basis point (tổng 10.000); mục tiêu là một loại sự kiện của
  `analytics` (vd đặt hàng, gửi form, bấm khối X); trạng thái `draft` / `running` / `stopped`.
- **Chia cố định, không ngẫu nhiên theo lần tải:** phiên bản = `hash(experiment_id, visitor_key) mod 10000` so
  với các mốc tỉ lệ — cùng người luôn ra cùng phiên bản, **F5 không đổi**. `visitor_key`: đã đăng nhập → `user:<id>`
  (cùng phiên bản trên mọi thiết bị, mọi ngày); chưa đăng nhập → khoá phiên ẩn danh trong ngày của bước 57 (không
  cookie: cùng ngày, cùng máy và mạng thì cùng phiên bản; hôm sau hay đổi mạng có thể đổi — tài liệu ghi rõ). Chỉ
  dùng để chọn phiên bản, không lưu gắn với người.
- **Bot và trình thu thập (Google…) luôn thấy phiên bản A**, cùng một URL, `canonical` như cũ.
- **Chỉ thử bố cục và nội dung:** khối dữ liệu (giá, khuyến mại, tồn) lấy từ dữ liệu thật như mọi trang — hai người
  cùng lúc không bao giờ thấy hai giá khác nhau vì thí nghiệm. Phiên bản nào chứa chữ có số tiền khác giá đang có
  → không cho chạy (`content.experiment_price_text`).
- **Đọc công khai:** `GET /public/resolve` nhận `visitor_key` đã băm do giao diện gửi (header
  `Autoxide-Visitor`), trả `layout` của phiên bản được chia và `experiment: {id, variant}` để giao diện gửi kèm sự
  kiện cho `analytics`; ETag theo phiên bản; `Vary: Autoxide-Visitor` chỉ khi trang đang có thí nghiệm.
- **Kết quả:** đọc số gộp của `analytics` (lượt xem, chuyển đổi theo phiên bản); trả tỉ lệ chuyển đổi, số mẫu,
  độ tin cậy bằng kiểm định hai tỉ lệ (z-test), câu "chưa đủ dữ liệu" khi mẫu nhỏ. **Người quyết bản thắng**:
  `POST …/experiments/{id}/conclude {winner}` → revision thắng thành bản đăng, thí nghiệm `stopped`. Không tự
  đổi bản đăng.
- Một trang một thí nghiệm `running`; sửa trang đang thử → phải dừng thí nghiệm trước (`content.experiment_running`).

### 10. AI tạo ảnh (D-231)

- Kernel thêm trait `ImageModel` (`generate(prompt, size, count) -> Vec<GeneratedImage>`), module `ai` công bố
  adapter (OpenAI Images; nhà cung cấp tương thích khác thì thêm adapter như D-109). **Mặc định tắt**; người cài chọn
  nhà cung cấp và đặt key (secret, không vào git). `Capabilities` thêm `image_generation`.
- `POST /media/ai-images` (module `media`, `media.ai_generate`, job): mô tả + mục đích (`banner` / `background` /
  `illustration`) + tỉ lệ khung → ảnh lưu vào `media` với `ai_generated = true`, `ai_prompt` (để rà), luôn là ảnh
  của khách, có nhãn "Ảnh do AI tạo" trong dữ liệu trả về để giao diện hiện. Trình tạo trang (mục 6) gợi ý tạo ảnh
  khi thư viện không có ảnh hợp, người dùng bấm mới tạo (tốn tiền).
- **Chỉ dùng cho banner, ảnh nền, ảnh minh hoạ bài viết / trang.** `catalog` từ chối gắn ảnh `ai_generated` vào sản
  phẩm (`catalog.ai_image_not_allowed`) — tránh người mua hiểu sai về hàng thật (Luật 19/2023). `instructions` cấm
  tạo ảnh người thật có thể nhận ra, logo / thương hiệu của người khác; kết quả nhà cung cấp chặn thì báo lỗi rõ.
- Hạn mức theo gói `media.ai_images` (mỗi tháng). Không gọi trong transaction ghi.

### 11. Khối mã tự viết (D-231)

Ba mức, chọn theo cài đặt; **mức mặc định là tắt** cho tới khi chủ cửa hàng bật:

| Mức | Ai bật | Chạy thế nào |
|---|---|---|
| `off` (mặc định) | — | Không có khối `content.custom_code` |
| `sandboxed` (C) | Chủ cửa hàng bật theo website (`content.custom_code.manage`) | HTML + CSS + JavaScript trong `<iframe sandbox="allow-scripts">` **không** `allow-same-origin` (nguồn gốc rỗng: không đọc được cookie, phiên đăng nhập, giỏ hàng, dữ liệu trang chính); CSP trong khung chỉ cho script / kết nối tới host trong danh sách cho phép của website; khung đổi chiều cao qua `postMessage` có kiểm dạng tin |
| `unrestricted` (D) | **Hai lớp:** người cài máy chủ bật `AUTOXIDE_CONTENT_UNSANDBOXED_CODE=true` **và** chủ cửa hàng bật theo website, xác nhận lại danh tính (step-up, bước 54) | Mã chèn thẳng vào trang như Shopify "custom code"; **không bao giờ** trên trang thanh toán, giỏ hàng, tài khoản, đăng nhập (kể cả khi bật) |

- Áp cho mọi mức: kích thước mã ≤ `CUSTOM_CODE_MAX_BYTES` (64 KB); mỗi lần sửa ghi audit (ai, khi nào, mã băm,
  diff) và cần xác nhận lại danh tính; **AI không bao giờ tạo hay sửa khối này** (mục 6 loại nó khỏi danh sách khối
  gửi mô hình); người bán trên sàn không có mức `unrestricted` ở gian hàng; agent (API key `agent`) sửa khối này thì
  luôn chờ duyệt (P12).
- **Sổ script** `GET /custom-code/inventory?org_id`: mọi khối mã đang công khai, trang, mức, mã băm, người sửa
  cuối — để chủ cửa hàng rà (theo ý PCI DSS 4.0 yêu cầu 6.4.3 / 11.6.1 về script trên trang thanh toán; trang
  thanh toán vốn không cho khối này). `autoxide doctor` cảnh báo khi có website ở mức `unrestricted`.
- Kiểm tra sức khoẻ website (mục 8) báo khối mã gọi host ngoài danh sách cho phép, hay mức `unrestricted` đang bật.

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
| `content_experiments` | `id`, `tenant_id`, `entry_id`, `locale`, `variants` (JSON: `key`, `revision_id`, `split_bp`), `goal_event`, `goal_filter` (JSON), `status`, `started_at?`, `stopped_at?`, `winner?`, `created_by` | (`tenant_id`,`entry_id`,`status`); một `running` mỗi trang qua cột khoá `running_key` unique |
| `content_custom_code_settings` | `tenant_id`, `org_id` (website), `mode` (`off`/`sandboxed`/`unrestricted`), `allowed_hosts` (JSON), `enabled_by`, `enabled_at` | pk (`tenant_id`,`org_id`) |
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
| `GET/POST /entries/{id}/experiments` · `POST /experiments/{id}/start` · `…/stop` · `…/conclude` · `GET /experiments/{id}/results` | `content.experiments.manage` · `content.read` | thử A/B (mục 9) |
| `GET/PUT /custom-code/settings/{org_id}` · `GET /custom-code/inventory?org_id` | `content.custom_code.manage` (+ step-up khi bật / đổi mức) · `content.read` | mức của website, host cho phép, sổ script (mục 11) |
| `POST /api/v1/media/ai-images` | `media.ai_generate` | AI tạo ảnh (mục 10), job |

### Tool MCP

`content.list_blocks` (`read_only`), `content.get_page_layout` (`read_only`), `content.update_page_layout`,
`content.compose_page` (nháp), `content.edit_page` (nháp), `content.create_campaign_drafts` (nháp),
`content.site_health` (`read_only`), `content.experiment_results` (`read_only`), `media.generate_ai_image`. Không có
tool sửa khối mã tự viết cho agent ngoài đường chờ duyệt. Cùng service với REST; agent bên ngoài dựng trang được với đúng quyền như
người.

### Event

`content.layout_updated` (entry, revision, `ai_assisted`), `content.theme_updated`, `content.shared_section_updated`
(kèm số trang bị ảnh hưởng), `content.ai_draft_ready`, `content.site_health_checked`, `content.experiment_started`,
`content.experiment_concluded`, `content.custom_code_changed` (mức, mã băm — không mang mã), `media.ai_image_generated`.
Không mang nội dung trang.

### Cấu hình (`AUTOXIDE_CONTENT_`)

`LAYOUT_MAX_BLOCKS` (500), `LAYOUT_MAX_BYTES` (262144), `EDIT_LOCK_SECONDS` (120), `AI_MAX_RETRIES` (2),
`BLOCK_DATA_TIMEOUT_MS` (800), `SITE_HEALTH_NOT_FOUND_MIN_HITS` (5), `CUSTOM_CODE_MAX_BYTES` (65536),
`UNSANDBOXED_CODE` (`false` — chỉ người cài máy chủ bật mức D), `EXPERIMENT_MIN_SAMPLE` (100 lượt xem mỗi phiên bản
trước khi báo độ tin cậy). Sinh lại `.env.example`.

### Tính năng và hạn mức (bước 35)

Tính năng: `content.page_builder`, `content.ai_pages`, `content.shared_sections`, `content.experiments`,
`content.custom_code`, `media.ai_images`. Hạn mức: `content.ai_page_generations` (mỗi tháng), `content.layout_pages`,
`media.ai_images` (mỗi tháng).

### Permission

`content.pages.write`, `content.pages.ai`, `content.themes.manage`, `content.experiments.manage`,
`content.custom_code.manage` (thêm vào bộ của `content`; quyền theo độ sâu ở nút website — D-208); `media.ai_generate`
(module `media`).

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
| `content.experiment_running` | 409 | Trang đang chạy thử A/B; dừng thí nghiệm trước khi sửa. |
| `content.experiment_price_text` | 422 | Phiên bản `{variant}` ghi số tiền khác giá đang có; giá phải lấy từ khối dữ liệu. |
| `content.custom_code_disabled` | 403 | Website chưa bật khối mã tự viết. |
| `content.custom_code_unrestricted_off` | 403 | Máy chủ chưa cho phép mã chạy không cách ly. |
| `content.custom_code_forbidden_page` | 422 | Không đặt khối mã tự viết trên trang thanh toán, giỏ hàng, tài khoản hay đăng nhập. |
| `content.custom_code_host_not_allowed` | 422 | Mã gọi tới `{host}` không có trong danh sách cho phép của website. |
| `media.ai_images_unavailable` | 503 | Máy chủ chưa bật AI tạo ảnh. |
| `catalog.ai_image_not_allowed` | 422 | Ảnh do AI tạo không dùng làm ảnh sản phẩm. |

## Thứ tự làm

1. Kernel: `blocks` (kiểu, `Module::blocks`, chỗ giữ `BlockData`, kiểm schema, gate kiểm bản dịch), `GET /kernel/blocks`.
2. `content`: `body_kind`, bộ kiểm bố cục, khối nền, `PUT/PATCH layout`, khoá sửa; test.
3. Đọc công khai có `blocks_data`, ETag, khối lỗi không hỏng trang; `forms`, `newsletter`, `comments`, `media`
   khai khối; `compliance` khai `legal.footer`.
4. Theme, khu dùng chung, trang mẫu (gói mẫu dùng chung + task nạp).
5. AI: compose / edit bằng job, kiểm và thử lại, lịch sử trò chuyện; bộ nháp chiến dịch.
6. Kiểm tra sức khoẻ website, nguồn hộp việc.
7. Khối mã tự viết: cài đặt theo website, ba mức, khung cách ly, sổ script, audit + step-up.
8. AI tạo ảnh: `ImageModel`, adapter, `FakeImageModel`, cờ `ai_generated` của `media`, chặn ở `catalog`.
9. Thử A/B (khi bước 57 xong): thí nghiệm, chia cố định, đọc công khai theo phiên bản, kết quả, chọn bản thắng.
10. Tool MCP, OpenAPI, `.env.example`, `docs/PAGES.md` + `docs/PAGES.vi.md` (khai khối cho module — mục mới trong
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
- Thử A/B: `cung_nguoi_luon_cung_phien_ban` (gọi 100 lần cùng `visitor_key`), `ti_le_chia_dung_xap_xi`
  (10.000 khoá giả, sai lệch < 2%), `bot_luon_thay_ban_goc`, `phien_ban_ghi_gia_cung_bi_chan`,
  `chon_ban_thang_thanh_ban_dang`, `sua_trang_dang_thu_bi_chan`, `hai_thi_nghiem_cung_trang_chi_mot_chay`.
- Khối mã: `muc_mac_dinh_tat`, `unrestricted_can_ca_may_chu_va_chu_cua_hang`, `khong_dat_tren_trang_thanh_toan`,
  `host_ngoai_danh_sach_bi_tu_choi`, `ai_khong_bao_gio_tao_khoi_ma`, `agent_sua_khoi_ma_cho_duyet`,
  `moi_lan_sua_co_audit`.
- Ảnh AI (`FakeImageModel`): `anh_ai_luu_media_co_co_ai_generated`, `anh_ai_khong_gan_duoc_vao_san_pham`,
  `het_han_muc_anh_ai`, `tat_mac_dinh`.
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
- **Thử A/B ngẫu nhiên theo lần tải:** người xem F5 thấy trang nhảy qua lại, số liệu sai. Luôn chia bằng băm cố định.
- **Cache khi có thí nghiệm:** CDN cache một phiên bản cho mọi người nếu thiếu `Vary` — chỉ thêm `Vary` cho trang
  đang thử để không phá cache của mọi trang khác.
- **Khung cách ly bị nới:** thêm `allow-same-origin` cùng `allow-scripts` là mất cách ly hoàn toàn — test khẳng định
  thuộc tính `sandbox` trả về đúng như thiết kế.

## Căn cứ pháp lý

Đối chiếu qua nguồn tổng hợp — **cần đối chiếu văn bản gốc / người có chuyên môn pháp lý** trước khi chạy thật:

| Nội dung | Văn bản | Ghi chú |
|---|---|---|
| Thông tin chủ sở hữu, logo thông báo / đăng ký trên website bán hàng | NĐ 52/2013 sửa bởi NĐ 85/2021; Luật Thương mại điện tử 122/2025 | khối `legal.footer` tự gắn |
| Nhãn nội dung do AI tạo | Luật Trí tuệ nhân tạo 134/2025/QH15 và văn bản hướng dẫn | `ai_assisted` trên revision, API trả để giao diện gắn nhãn |
| Thông tin trung thực về giá, khuyến mại trên trang | Luật 19/2023/QH15; Luật Thương mại 2005 | giá, giảm lấy từ khối dữ liệu |
| Thu email qua popup | Luật 91/2025/QH15; NĐ 91/2020 | đồng ý trước, theo mục đích |
| Chia người xem cho thử A/B không cookie | Luật 91/2025/QH15; NĐ 356/2025 | khoá ẩn danh trong ngày của bước 57, không lưu gắn người — cần đối chiếu có cần thông báo |
| Ảnh do AI tạo, không dùng làm ảnh sản phẩm | Luật Trí tuệ nhân tạo 134/2025/QH15; Luật 19/2023/QH15 | nhãn trên ảnh, chặn ở `catalog` |
| Mã của bên thứ ba chạy trên website bán hàng | Luật An ninh mạng (văn bản hiện hành — cần đối chiếu); PCI DSS 4.0 (chuẩn ngành, không phải luật) | trách nhiệm của chủ website khi bật mức D — tài liệu cảnh báo rõ |

## Agent làm bước này tự quyết

- Thư viện kiểm JSON Schema (ưu tiên crate đã ổn định, chạy không cần mạng); ghi `STACK.md` và DECISIONS.
- Bộ khối nền cuối cùng và tên props; bộ trang mẫu ban đầu.
- Cách tính "phiên bản dữ liệu" của khối dữ liệu cho ETag.
- Tên sự kiện mục tiêu mặc định cho thử A/B; ngưỡng mẫu và mức tin cậy hiển thị.
- Nhà cung cấp ảnh thứ hai (ngoài OpenAI Images) nếu có API ổn định — tra tài liệu gốc, không đoán.
