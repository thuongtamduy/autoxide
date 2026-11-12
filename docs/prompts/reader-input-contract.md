# Hợp đồng chung: dữ liệu người đọc gửi lên (`forms`, `newsletter`, `comments`)

Viết sau giai đoạn 0 của [`forms-newsletter-comments.md`](forms-newsletter-comments.md). Ba module
phải làm GIỐNG NHAU ở mọi điểm dưới đây; chỗ nào hợp đồng không nói thì theo `CLAUDE.md` và đề bài
của module. Phần nền nằm trong kernel (D-107 … D-118) — đọc doc comment của từng mục trước khi
dùng, đừng tự chế cách khác.

## 1. Đồ nghề của kernel

| Cần | Dùng | Quyết định |
|---|---|---|
| Khoá HMAC cho việc của module | `context.require::<SecretKey>()?` → `derive("autoxide/<module>/<việc>/v1")` | D-107 |
| IP, user agent, `Origin`, `Referer` | extractor `autoxide_kernel::http::ClientInfo` | D-108 |
| Lưu dấu vết IP | `ip_hash::fingerprint(&secret, ip)` (không bao giờ lưu IP thô) | D-109 |
| Giới hạn số lần | `Throttle::new(db, "<module>", limit, window)`, bảng `throttle_migration` | D-110 |
| Token dùng một lần (xác nhận email…) | `OneTimeTokens::new("<module>")`, bảng `tokens_migration` | D-111 |
| Chống spam bài gửi công khai | `context.require::<SpamGuard>()?`, bảng `antispam_migration` | D-113, D-114 |
| Gửi mail | `dyn Mailer`, `Mail { …, reply_to, headers, dedupe_key, ..Mail::default() }` | D-115 |
| Mail bị từ chối hẳn | nghe event `autoxide_kernel::mail::DELIVERY_FAILED` | D-115 |
| Event do người lạ gây ra | `Actor::public()`; bảng event thêm `outbox_actor_migration` | D-116 |
| Tra đối tượng của module khác | `context.resources()` → `resolve("content.entry", id)` | D-117 |
| Link tuyệt đối về API | `context.require::<PublicUrl>()?` → `link("/api/v1/…")` | D-118 |

Thứ tự migration của module mới (sau các bảng của module):

```rust
autoxide_kernel::event::outbox_migration("m00a_create_events", "forms"),
autoxide_kernel::event::outbox_actor_migration("m00b_add_event_actor", "forms"),
autoxide_kernel::job::jobs_migration("m00c_create_jobs", "forms"),
autoxide_kernel::throttle::throttle_migration("m00d_create_throttle", "forms"),
autoxide_kernel::antispam::antispam_migration("m00e_create_antispam_nonces", "forms"),
// chỉ module cần token dùng một lần:
autoxide_kernel::tokens::tokens_migration("m00f_create_tokens", "newsletter"),
```

## 2. Endpoint công khai nhận bài gửi

- Đường dẫn dưới `/api/v1/<module>/public/…` (như `content`, D-105). Handler nhận
  `principal: Option<Principal>` và `client: ClientInfo`; OpenAPI khai `security((), ("bearer" =
  []))`. Thân JSON nhận bằng `ValidJson<T>`.
- Thân có ba trường chống spam, cùng tên ở cả ba module:
  - `form_token` (chuỗi): lấy từ `GET /api/v1/antispam/token?scope=<phạm vi>`;
  - `trap` (chuỗi, tuỳ chọn): giá trị ô bẫy mà website ẩn với người dùng — người thật để trống;
  - `challenge` (chuỗi, tuỳ chọn): giá trị ô `altcha` của widget khi máy chủ bật đề.
- Phạm vi (`scope`): `forms:<key của form>`, `newsletter:subscribe`, `comments:<target_type>`.
- Hai pha:
  1. Kiểm dữ liệu của module (báo mọi lỗi một lần, D-083) → dò trùng nếu cần (`content_hash`) →
     `guard.check(&Submission { … }, &limits).await?`. Mọi việc gọi mạng ở pha này.
  2. `Verdict::Discard` → không ghi gì, trả lời như thành công. `Accept` / `Quarantine` → mở
     transaction, tạo bản ghi (`Quarantine` thì lưu ở trạng thái chờ / spam, kèm lý do), ghi event
     bằng `Actor::public()` (hoặc `Actor::from(&principal)`), rồi
     `checked.claim(&tx, "<module>", Some(&id.to_string())).await?`. `Claim::Replay { result }` →
     rollback và trả lại câu trả lời của bản ghi `result` (gửi lại cùng token = cùng kết quả).
- `Limits`: theo IP (`ip`), theo email (`email`, khi có) cho người lạ; theo người dùng (`user`)
  cho người gọi đã đăng nhập. Giá trị mặc định do module chọn và khai thành cấu hình.
- **Câu trả lời giống hệt** cho nhận, giữ lại và bỏ đi — bot không được phân biệt: cùng mã HTTP,
  cùng hình dạng, không lộ trạng thái thật. Bản ghi chưa có (`Discard`) thì trả ID / token giả
  cùng định dạng. Lỗi chỉ cho: dữ liệu sai (422), token / đề hỏng (lỗi của `SpamGuard`), đích
  không nhận (vd form đóng, bình luận đã đóng).
- Có `Origin` mà không nằm trong danh sách cho phép (khi module có danh sách đó) → từ chối 403.

## 3. Dữ liệu cá nhân

- **Email** lưu dạng chuẩn hoá: `trim()` + chữ thường; cột có index để xoá / xuất theo email.
- **IP**: chỉ lưu `ip_hash::fingerprint`, cột `ip_fingerprint`, xoá theo thời hạn cấu hình của
  module bằng job hằng ngày. Không lưu IP thô ở đâu, kể cả log.
- **Bằng chứng đồng ý** (ô đồng ý của form, đăng ký bản tin): cột JSON `consent` =
  `{"text": "<đúng chữ đã hiện>", "version": "<phiên bản>", "at": "<thời điểm UTC>", "source":
  "<nơi đăng ký>", "ip_fingerprint": "<…>"}`. Chữ đồng ý do client gửi kèm, máy chủ lưu nguyên
  văn; không có ô đồng ý thì `null`.
- **Xuất / xoá theo email** (quyền của chủ dữ liệu, người không có tài khoản):
  `GET /api/v1/<module>/data?email=` và `DELETE /api/v1/<module>/data?email=`, permission
  `<module>.export`, trả `Data`. Xoá là xoá hẳn (hoặc xoá mềm giữ cấu trúc, với bình luận); ghi
  event (không mang email). Dữ liệu theo tài khoản: nhận `privacy::USER_DELETED` và cài
  `PersonalData` như `content`.
- **Event không mang dữ liệu cá nhân**: chỉ ID và khoá (vd `submission_id`, `form_key`). Module
  `webhooks` chuyển mọi event ra ngoài; hệ thống ngoài lấy chi tiết qua API có permission.

## 4. Đặt tên

- Trạng thái: chữ thường, `snake_case`, lưu dạng chữ, kiểm trong ứng dụng.
  - `forms`: bài gửi `new` / `read` / `archived` / `spam`; form `open` / `closed`.
  - `newsletter`: đăng ký `unconfirmed` / `confirmed` / `unsubscribed`; người đăng ký `active` /
    `blocked`; chiến dịch `draft` / `scheduled` / `sending` / `paused` / `sent` / `cancelled`.
  - `comments`: `pending` / `published` / `rejected` / `spam` / `deleted`.
- Lý do giữ lại: cột `spam_reason` (forms) / `hold_reason` (comments), giá trị là các lý do của
  `Verdict::Quarantine` nối bằng dấu phẩy (`too_fast,duplicate`) hoặc lý do riêng của module.
- Mã lỗi `<module>.<việc>`, có bản dịch `locales/vi.json`. Permission `<module>.<việc>`.
- Bảng `<module>_…`; event `<module>.<việc_đã_xảy_ra>`.

## 5. Mail

- Mẫu mail bằng `MailTemplate` (khoá `<module>.mail.<tên>`, bản dịch vi). Mail gửi cho người lạ
  **không chứa chữ do người lạ gõ** (trừ mail gửi quản trị viên), để không bị dùng làm công cụ
  gửi spam.
- Gửi lặp lại được (job chạy lại) thì đặt `dedupe_key` duy nhất (vd ID lượt gửi).
- Mail gửi quản trị viên về một bài gửi: `reply_to` = email người gửi (nếu có).
- Giới hạn số mail xác nhận gửi tới một địa chỉ bằng `Throttle` theo email.

## 6. Test bắt buộc chung

Mọi test chạm database qua `TestDatabase::run`, chạy trên cả SQLite và Postgres:

- bẫy, token thiếu / sai, gửi quá nhanh, quá nhiều lần → câu trả lời công khai giống hệt bài nhận;
- gửi lại cùng token → cùng một bản ghi, kể cả gửi đồng thời;
- xoá / xuất theo email; `USER_DELETED`; event không có email trong payload;
- endpoint công khai qua router thật (HTTP), tool MCP qua client thật.
