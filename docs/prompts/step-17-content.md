# Đề bài bước 17 — module `content`

File này là đề bài giao cho một phiên Claude Code làm bước 17 của `docs/ROADMAP.md`. Cách dùng:
mở phiên mới ở gốc repo và gửi:

> Làm bước 17 theo `docs/prompts/step-17-content.md`.

Phần dưới là đề bài. Quyết định nền: D-096, D-098, D-100, D-101, D-102. Khảo sát và nguồn:
`docs/research/content.md`. Đề bài này mô tả **cái cần có và các ràng buộc**; tên hàm, cách chia
file, chi tiết migration do agent làm quyết. Chỗ nào đề bài mâu thuẫn với code thật hoặc với
`CLAUDE.md` thì `CLAUDE.md` thắng — dừng lại, ghi quyết định mới, không lặng lẽ làm khác.

---

## 0. Điều kiện trước

1. Bước 15 (nền chung) và 16 (`media`) phải đã `[x]` trong `docs/ROADMAP.md`. Chưa thì làm
   chúng trước theo thứ tự (skill `next-step`), rồi mới vào bước 17.
2. Đọc trước: `CLAUDE.md`; `docs/ROADMAP.md` bước 15–17; `docs/DECISIONS.md` từ D-080 trở đi;
   mục "Khuyến nghị cho Autoxide `content`" của `docs/research/content.md`;
   `docs/MODULE_GUIDE.vi.md`; module `notes` (khuôn mẫu đầy đủ) và `files` (dọn và xuất dữ liệu
   cá nhân, nhận event).
3. Đọc code thật của bước 15 (lọc, sắp xếp, tìm kiếm, endpoint công khai, cache) và `api` của
   `media` trước khi thiết kế route và query — đề bài này viết trước khi hai bước đó xong, nên
   tên cụ thể có thể khác.
4. API thư viện: skill `check-library-api` trước khi dùng thứ chưa dùng trong repo (crate
   `slug`, phần mới của SeaORM, `hmac`).

## 1. Phạm vi

**Có:** bài viết, trang (có cấp cha con — D-101), danh mục, thẻ, tác giả, menu, phiên bản, đa
ngôn ngữ, SEO, hẹn giờ, xem trước, chuyển hướng, dữ liệu cho sitemap và feed, endpoint công khai,
event, tool MCP.

**Không:** loại nội dung tự định nghĩa; bình luận, form, bản tin email, thành viên trả phí; quy
trình duyệt nhiều bước; đăng nhiều bài cùng lúc (release); sinh HTML từ JSON ở máy chủ; lưu HTML
thô hay mã nhúng; khoá bài khi đang sửa; thống kê lượt xem; quyền theo từng bài. Mọi khái niệm
của `content` chỉ nằm trong `modules/content` — không đưa vào kernel. Thứ trung lập mà thiếu
trong kernel (ví dụ `ErrorKind` cho 428) thì thêm vào kernel ở dạng trung lập.

## 2. Mô hình dữ liệu

Mọi bảng tên `content_…`, ID là UUIDv7 sinh trong ứng dụng, thời gian UTC, JSON lưu dạng `TEXT`.
Không foreign key xuyên module: `user_id`, `media_id` chỉ là UUID. Foreign key trong module thì
dùng bình thường. Migration có `down`.

| Bảng | Nội dung chính | Ràng buộc và index |
|---|---|---|
| `content_entries` — bản nháp làm việc | `kind` (`post`/`page`), `locale`, `translation_group_id`, `parent_id` (chỉ trang), `status` (`draft`/`published`/`archived`), `has_unpublished_changes`, `title`, `slug`, `excerpt`, `body` (JSON), `body_schema_version`, `plain_text`, `word_count`, `reading_time_minutes`, `featured_media_id`, `featured_media_alt`, `meta_title`, `meta_description`, `og_media_id`, `canonical_url`, `noindex`, `created_by`, `created_at`, `updated_at` (nguồn của `ETag`) | unique (`translation_group_id`, `locale`); unique (`locale`, `kind`, `parent_key`, `slug`); index cho danh sách quản trị |
| `content_published` — bản công khai | `entry_id` (PK), `kind`, `locale`, `translation_group_id`, `parent_id`, `revision_id`, `path`, các trường nội dung và SEO như trên, `published_at` (ngày hiển thị, sửa được), `first_published_at`, `content_modified_at`, `updated_at` | unique (`locale`, `path`); index (`locale`, `kind`, `published_at`, `entry_id`) cho cursor |
| `content_revisions` | `entry_id`, `snapshot` (JSON mọi trường sửa được), `reason` (`initial`/`explicit_save`/`background_save`/`published`/`unpublished`/`restored`/`scheduled`), `created_by`, `created_at` | index (`entry_id`, `created_at`, `id`) |
| `content_schedules` | `entry_id`, `action` (`publish`/`unpublish`), `revision_id`, `run_at`, `status` (`scheduled`/`running`/`succeeded`/`failed`/`canceled`), `error_code`, `created_by`, thời gian | index (`status`, `run_at`), (`entry_id`, `status`) |
| `content_redirects` | `locale`, `from_path`, `to_entry_id` hoặc `to_url`, `status_code` (301/302/307/308/410), `source` (`auto`/`manual`/`gone`), `created_by`, `created_at` | unique (`locale`, `from_path`) |
| `content_tree_locks` | một dòng cho mỗi `locale`, cột `version` | xem mục 4 |
| `content_terms` + `content_term_translations` | danh mục (có `parent_id`) và thẻ (phẳng); tên, slug, mô tả, meta theo ngôn ngữ | unique (`taxonomy`, `locale`, `slug`) |
| `content_entry_terms` | `entry_id`, `term_id`, `sort_order` (term đầu là chính) | PK (`entry_id`, `term_id`) |
| `content_authors` | `user_id` (để trống được, không foreign key), `display_name`, `slug`, `bio`, `avatar_media_id`, `website_url` | unique (`slug`) |
| `content_entry_authors` | `entry_id`, `author_id`, `sort_order` (tác giả đầu là chính) | PK (`entry_id`, `author_id`) |
| `content_menus` + `content_menu_items` | menu theo (`key`, `locale`); item có `parent_id`, `sort_order`, `label`, đích là mục nội dung, term hoặc URL | unique (`key`, `locale`) |

**Bẫy NULL trong unique:** cả SQLite và Postgres coi hai `NULL` là khác nhau, nên unique có
`parent_id` để trống sẽ **không** chặn hai trang gốc trùng slug. Dùng cột `parent_key` không
NULL (ID cha, hoặc UUID toàn số 0 cho trang gốc và mọi bài viết), hoặc cách khác chạy được trên cả
hai database — và viết test cho đúng trường hợp này.

Term và tác giả gắn vào mục **có hiệu lực ngay**, không đi qua nháp (như Ghost, WordPress); ghi
rõ trong tài liệu API.

## 3. Vòng đời, phiên bản, hẹn giờ

- **Tạo** mục mới: `status = draft`, sinh revision `initial`. Slug để trống thì sinh từ tiêu
  đề bằng crate `slug`; kết quả rỗng thì dùng một đoạn của ID; trùng thì thêm `-2`, `-3`; dài
  tối đa 100 ký tự. Slug người dùng gửi lên phải đúng dạng `[a-z0-9]+(-[a-z0-9]+)*`.
- **Sửa** nháp (`PATCH`): chỉ đổi `content_entries`, bật `has_unpublished_changes` nếu đang
  công khai. Không sinh revision mỗi lần sửa; sinh khi client yêu cầu lưu tường minh, hoặc lần
  sửa cách revision gần nhất quá 10 phút (`background_save`).
- **Đăng** (`publish`): trong **một** transaction — kiểm ảnh tồn tại và công khai (qua `api` của
  `media`, gọi **trước** transaction), tính `path`, kiểm `path` chưa bị dùng, chép sang
  `content_published`, sinh revision `published`, ghi chuyển hướng nếu `path` đổi (mục 4), xoá
  chuyển hướng `auto`/`gone` đang chiếm `path` mới, ghi event qua outbox.
- **Gỡ** (`unpublish`): xoá khỏi `content_published`, về `draft` (đường dẫn trả 404 — có thể đăng
  lại). **Lưu trữ** (`archive`): gỡ và ghi `gone` (410). **Xoá**: chỉ khi không còn trang con;
  nếu đang công khai thì ghi `gone`.
- **Khôi phục** revision: chép snapshot vào nháp (sinh revision `restored`), không tự đăng.
- **Giữ revision** tối đa `AUTOXIDE_CONTENT_MAX_REVISIONS` mỗi mục; job định kỳ dọn bớt, không
  bao giờ xoá revision đang được bản công khai hay một lịch đang chờ trỏ tới.
- **Hẹn giờ:** tạo lịch đăng thì sinh revision `scheduled` từ nháp hiện tại và ghim vào lịch; lịch
  chạy qua job queue của kernel đúng `run_at`. Mỗi mục tối đa một lịch đăng và một lịch gỡ đang
  chờ. Lúc chạy: kiểm trong transaction rằng lịch vẫn `scheduled` (chạy lại không đăng hai lần);
  nếu bản công khai đã được đăng **sau** lúc đặt lịch thì lịch `failed` với
  `content.schedule_outdated`, không ghi đè. Permission kiểm lúc đặt lịch; người đặt lịch bị xoá
  tài khoản thì lịch của họ bị huỷ.

## 4. Đường dẫn, cây trang, chuyển hướng

- `path` do backend tính: trang = `/` + slug các trang tổ tiên + slug; bài =
  `/<AUTOXIDE_CONTENT_POST_PATH_PREFIX>/` + slug (mặc định `blog`). Không chứa tên miền và
  tiền tố ngôn ngữ — website tự ghép. Bài và trang dùng chung không gian đường dẫn (unique
  (`locale`, `path`)), nên trang tên `blog` có trang con trùng đường dẫn với bài sẽ bị chặn bằng
  `content.path_taken`.
- **Cây trang** (D-101): cha phải là trang cùng ngôn ngữ; sâu tối đa 5 cấp; chặn vòng lặp. Đổi
  cha là thao tác riêng `move` (cần `content.publish` nếu mục đang công khai), có hiệu lực ngay
  cho cả nháp lẫn bản công khai. Mọi thay đổi cây của một ngôn ngữ (`move`, đăng trang có slug
  đổi) bắt đầu transaction bằng việc cập nhật dòng của ngôn ngữ đó trong `content_tree_locks` —
  trên Postgres câu đó khoá dòng nên hai lệnh `move` chéo nhau không thể cùng qua bước kiểm vòng
  lặp; SQLite vốn chỉ có một người ghi. Có test đồng thời cho đúng tình huống A vào B và B vào A.
- Không cho đăng trang khi trang cha chưa công khai (`content.parent_not_published`); không cho gỡ
  trang khi còn trang con đang công khai (`content.page_has_published_children`).
- **Bản dịch của trang:** tạo bản dịch thì cha của bản dịch là bản dịch của cha trong ngôn ngữ
  đích; cha chưa có bản dịch thì báo `content.parent_not_translated`. Sau đó cây mỗi ngôn ngữ đi
  riêng.
- **Chuyển hướng tự động:** khi `path` công khai của một mục đổi (đổi slug rồi đăng, `move`), ghi
  chuyển hướng 301 từ đường dẫn cũ tới **ID** mục — cho mục đó và mọi trang con đang công khai,
  trong cùng transaction. Chuyển hướng trỏ ID nên không thành chuỗi. Chuyển hướng làm tay: chặn
  vòng lặp (`content.redirect_loop`).
- **`resolve`** (công khai): nhận (`locale`, `path`); thứ tự tìm: bản công khai → chuyển hướng →
  404. Trả `Data` với một trong: mục nội dung đầy đủ; `{"type": "redirect", "status": 301, "location": …}`;
  `{"type": "gone"}` (410). Website gọi một lần là biết render, chuyển hướng hay báo lỗi.

## 5. Thân bài

- JSON theo cấu trúc ProseMirror/Tiptap (`type`, `attrs`, `content`, `marks`, `text`). Máy chủ
  **từ chối mặc định**: chỉ nhận node, mark, thuộc tính trong danh sách. Bộ đầu tiên: `doc`,
  `paragraph`, `heading` (cấp 2–4), `text`, `hardBreak`, `bulletList`, `orderedList`,
  `listItem`, `blockquote`, `codeBlock` (`language`), `horizontalRule`, `image` (`mediaId`,
  `alt`, `caption` — không có `src`), `table`/`tableRow`/`tableHeader`/`tableCell`, `embed`
  (`url`, host phải nằm trong `AUTOXIDE_CONTENT_EMBED_HOSTS`); mark `bold`, `italic`,
  `underline`, `strike`, `code`, `link` (`href`: `http`, `https`, `mailto` hoặc đường dẫn bắt
  đầu bằng `/`). Danh sách cuối cùng chốt cùng repo giao diện; công bố JSON Schema của thân bài
  trong OpenAPI. `body_schema_version` = 1; đổi danh sách về sau phải migrate cả revision.
- Giới hạn: kích thước (`AUTOXIDE_CONTENT_MAX_BODY_BYTES`), độ sâu, số node. Mọi lỗi báo **một
  lần**, mỗi lỗi kèm vị trí trong JSON (D-083).
- Lúc lưu: trích `plain_text` (cho tìm kiếm và tóm tắt), `word_count` (đếm theo khoảng trắng —
  với tiếng Việt là đếm âm tiết), `reading_time_minutes` (theo
  `AUTOXIDE_CONTENT_WORDS_PER_MINUTE`, mặc định 238 — con số cho tiếng Anh, tài liệu phải nói rõ
  chưa có số đo cho tiếng Việt), danh sách `mediaId` để kiểm.

## 6. API

Route của module dưới `/api/v1/content`; tên và cách khai endpoint công khai theo cơ chế thật của
bước 15. Danh sách dùng `fetch_page` (cursor), lọc và sắp xếp theo bước 15. Tạo mới trả
`Created`. Mọi lệnh ghi nhận `Idempotency-Key`.

**Quản trị** (cần đăng nhập):

- `GET/POST /entries`, `GET/PATCH/DELETE /entries/{id}` — lọc theo `kind`, `locale`, `status`,
  term, tác giả, cha; tìm theo chữ.
- `POST /entries/{id}/publish`, `/unpublish`, `/archive`, `/move`, `/translations` (tạo bản dịch
  ở `locale` khác), `/preview-token`.
- `GET /entries/{id}/revisions`, `GET /entries/{id}/revisions/{revision_id}`,
  `POST /entries/{id}/revisions/{revision_id}/restore`.
- `GET/POST /entries/{id}/schedules`, `DELETE /schedules/{id}` (huỷ).
- `GET/POST /terms`, `GET/PATCH/DELETE /terms/{id}`; `GET/POST /authors`,
  `GET/PATCH/DELETE /authors/{id}`; `GET /menus`, `GET/PUT /menus/{key}?locale=` (`PUT` thay cả
  cây item); `GET/POST /redirects`, `GET/PATCH/DELETE /redirects/{id}`.

**`If-Match` bắt buộc** (D-102) cho `PATCH`, `DELETE` và mọi thao tác đổi trạng thái của mục
(`publish`, `unpublish`, `archive`, `move`, `restore`, tạo lịch): thiếu thì 428
`precondition_required`, sai thì 412 (D-084). Thêm `ErrorKind` cho 428 vào kernel nếu chưa có.
Menu `PUT` cũng bắt buộc.

**Công khai** (không cần đăng nhập, chỉ đọc `content_published`, có cache của bước 15):

- `resolve` (mục 4).
- Danh sách bài đã đăng: lọc `locale`, term, tác giả; tìm theo chữ (trait của bước 15); sắp theo
  `published_at` mới nhất.
- Menu theo `key` + `locale`: item đã tính sẵn `path` và tiêu đề; item trỏ tới mục chưa công khai
  thì ẩn.
- Term và tác giả theo slug.
- Dữ liệu sitemap: mọi mục công khai (`path`, `locale`, `content_modified_at`, `noindex`, và các
  bản dịch cùng nhóm để website sinh hreflang), phân trang bằng cursor.
- Dữ liệu feed: N bài mới nhất của một ngôn ngữ, kèm thân bài JSON, tóm tắt, tác giả, ảnh — đủ
  cho RSS 2.0, Atom và JSON Feed (ID ổn định là UUID của mục, không phải URL).
- Xem trước: nhận preview token, trả bản nháp, `Cache-Control: no-store`.

**Preview token:** HMAC-SHA256 trên (ID mục, hạn dùng) bằng
`AUTOXIDE_CONTENT_PREVIEW_SECRET`, hạn mặc định `AUTOXIDE_CONTENT_PREVIEW_TTL_SECONDS` = 600.
Chưa cấu hình secret thì tạo token trả `content.preview_disabled`. Không lưu bảng.

Mọi handler có `///` (tiếng Anh) và `description` cho từng response; tài liệu nêu permission
cần có. Sinh lại `docs/openapi.json` và `.env.example`.

## 7. Permission, event, MCP, dữ liệu cá nhân

- **Permission:** `content.read` (đọc nháp, revision, lịch), `content.write` (tạo, sửa nháp, tạo
  bản dịch, khôi phục, preview token), `content.publish` (đăng, gỡ, lưu trữ, xoá, `move` mục đang
  công khai, hẹn giờ), `content.manage` (term, tác giả, menu, chuyển hướng).
- **Event** cho mọi thay đổi (tạo, sửa, đăng, gỡ, lưu trữ, xoá, chuyển, khôi phục, lịch; term,
  tác giả, menu, chuyển hướng), ghi qua outbox cùng transaction; audit log tự nhận. Event đổi bản
  công khai mang đủ để website làm mới cache: ID, `kind`, `locale`, `translation_group_id`,
  `path` mới, mọi `path` cũ bị ảnh hưởng (kể cả trang con). Website nhận qua module `webhooks`.
- **Tool MCP** (cùng service, cùng permission với REST): liệt kê, đọc, tạo nháp, sửa, đăng, gỡ,
  xem revision, khôi phục, `resolve`. Tool sửa và đổi trạng thái nhận tham số `version` (giá trị
  `ETag`) và kiểm như `If-Match`. Mô tả tool bằng tiếng Anh.
- **Dữ liệu cá nhân:** nhận `privacy::USER_DELETED` — xoá hồ sơ tác giả gắn với người đó (và liên
  kết của nó với bài), bỏ trống `created_by`, huỷ lịch họ đặt; bài vẫn giữ. Cài `PersonalData`:
  xuất hồ sơ tác giả và danh sách mục họ tạo.

## 8. Cấu hình

Khai bằng `Setting`, mặc định hợp lý, người lạ không cấu hình gì vẫn chạy được (trừ preview):

`AUTOXIDE_CONTENT_LOCALES` (mặc định `en,vi`, mã BCP 47), `AUTOXIDE_CONTENT_DEFAULT_LOCALE`
(`en`), `AUTOXIDE_CONTENT_POST_PATH_PREFIX` (`blog`), `AUTOXIDE_CONTENT_MAX_REVISIONS` (50),
`AUTOXIDE_CONTENT_MAX_BODY_BYTES`, `AUTOXIDE_CONTENT_WORDS_PER_MINUTE` (238),
`AUTOXIDE_CONTENT_EMBED_HOSTS` (YouTube, Vimeo), `AUTOXIDE_CONTENT_PREVIEW_SECRET` (không mặc
định), `AUTOXIDE_CONTENT_PREVIEW_TTL_SECONDS` (600).

## 9. Mã lỗi

Tiền tố `content.`, có bản dịch trong `modules/content/locales/vi.json`. Ít nhất: `slug_taken`,
`path_taken`, `invalid_body`, `media_not_found`, `media_not_public`, `locale_not_supported`,
`translation_exists`, `parent_not_found`, `parent_not_translated`, `parent_not_published`,
`page_has_children`, `page_has_published_children`, `tree_too_deep`, `tree_cycle`,
`schedule_conflict`, `schedule_outdated`, `preview_disabled`, `preview_token_invalid`,
`redirect_loop`, `not_found`.

## 10. Test bắt buộc

Mọi test chạy trên **cả SQLite và Postgres** qua `TestDatabase::run`. Ngoài test thường cho từng
endpoint và từng mã lỗi:

- Đồng thời: hai lệnh đăng cùng một mục; job hẹn giờ chạy cùng lúc với đăng tay; job chạy lại
  hai lần; hai mục cùng lúc lấy một slug; hai lệnh `move` chéo nhau (không được tạo vòng lặp);
  hai lệnh sửa cùng `If-Match` (chỉ một thắng).
- Hai trang gốc cùng slug bị chặn (bẫy NULL).
- Đổi slug trang cha rồi đăng: mọi trang con có `path` mới và chuyển hướng từ `path` cũ;
  `resolve` đường dẫn cũ trả chuyển hướng tới đúng mục.
- Endpoint công khai không bao giờ trả nháp; bản dịch thiếu trả 404, không fallback.
- Kiểm thân bài: link `javascript:`, node lạ, thuộc tính lạ, `src` ảnh, quá sâu, quá lớn — báo đủ
  lỗi trong một lần.
- Slug tiếng Việt: "Đường đến Đà Lạt" → `duong-den-da-lat`.
- Xoá tài khoản và xuất dữ liệu cá nhân.
- Tool MCP qua một MCP client thật, kể cả tham số `version`.

## 11. Cách làm

- Chạy skill `split-work` trước khi chia việc. Thứ tự gợi ý, mỗi phần gate xanh rồi commit:
  1. khung module, migration, mục nháp (tạo, sửa, đọc, danh sách), kiểm thân bài, slug;
  2. đăng, gỡ, lưu trữ, bản công khai, cây trang, `path`, chuyển hướng, `resolve`;
  3. revision, khôi phục, hẹn giờ;
  4. bản dịch, term, tác giả, menu;
  5. endpoint công khai, sitemap, feed, preview, tìm kiếm, cache;
  6. event, tool MCP, dữ liệu cá nhân, tài liệu.
- Ghi quyết định mới (skill `record-decision`) khi chọn khác đề bài hoặc gặp chỗ đề bài không
  nói; thêm crate `slug` vào `STACK.md`.
- Tài liệu cho người dùng: `docs/CONTENT.md` + `docs/CONTENT.vi.md` (cùng nội dung) — mô hình,
  vòng đời, đường dẫn và chuyển hướng, cách website dùng `resolve`, preview, webhook làm mới
  cache, sitemap, feed.

## 12. Xong khi

- Mọi mục bước 17 trong `docs/ROADMAP.md` đánh `[x]`; `./gate.sh` xanh trên cả hai database.
- `docs/openapi.json`, `.env.example`, bản dịch đã sinh lại và khớp.
- Báo cáo cho chủ dự án bằng tiếng Việt, lời thường: đã làm gì, quyết định nào đã ghi, việc gì
  họ cần làm (ví dụ cấp `AUTOXIDE_CONTENT_PREVIEW_SECRET` khi deploy), việc gì để lại cho repo
  giao diện (danh sách node của trình soạn thảo, route xem trước, nhận webhook).
