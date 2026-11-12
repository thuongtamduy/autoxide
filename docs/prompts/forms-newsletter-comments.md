# Đề bài: nền chung cho dữ liệu người đọc gửi lên + ba module `forms`, `newsletter`, `comments`

File này là đề bài giao cho một phiên Claude Code. Cách dùng: mở phiên mới ở gốc repo, trên
nhánh dựa trên `main` mới nhất, và gửi:

> Làm theo `docs/prompts/forms-newsletter-comments.md`.

Khảo sát, nguồn và chi tiết khuyến nghị (cấu hình, mã lỗi, test, 12 quyết định cần ghi):
`docs/research/forms-newsletter-comments.md` — đọc mục "Khuyến nghị cho Autoxide" trước khi làm. Đề bài mô tả **cái cần có và các
ràng buộc**; tên hàm, cách chia file, chi tiết migration do agent quyết. Chỗ nào đề bài mâu thuẫn
với code thật hoặc với `CLAUDE.md` thì `CLAUDE.md` thắng — dừng lại, ghi quyết định mới, không
lặng lẽ làm khác. Đề bài viết khi `main` ở commit `1e78589` (quyết định mới nhất lúc đó là D-095). Từ đó
`main` đã có thêm module `content` (bước 17, quyết định D-096 … D-105; số D-0xx trong đề bài này
đã khớp với `main`), nên tên và số cụ thể có thể đã đổi tiếp — đọc code thật trước.

---

## A. Chủ dự án đã quyết (ngày 12/11/2026)

Ghi thành **một** mục "chủ dự án quyết" trong `docs/DECISIONS.md` (số kế tiếp lúc làm) trước khi
viết code, rồi cập nhật `CLAUDE.md` (danh sách module nghiệp vụ đã giao) và `docs/ROADMAP.md`
(thêm các bước dưới đây):

1. Ba module nghiệp vụ mới, **mỗi module một crate riêng**: `forms` (biểu mẫu), `newsletter`
   (bản tin email), `comments` (bình luận). Không gộp. Thứ tự ưu tiên khi phải chọn: `forms` →
   `newsletter` → `comments`.
2. Thứ dùng chung cho cả ba (chống spam, token dùng một lần, IP người gọi…) làm **trong core,
   ở dạng trung lập**, trước khi làm ba module — không module nào tự chế cách riêng.
3. Làm theo ba giai đoạn (mục C): giai đoạn 0 một agent làm tuần tự; giai đoạn 1 chia **ba
   subagent chạy song song**, mỗi module một subagent; giai đoạn 2 agent chính gộp.
4. Các yêu cầu pháp lý chưa chắc chắn (Nghị định 91/2020 về thư rác, Nghị định 147/2024 về bình
   luận, Luật Bảo vệ dữ liệu cá nhân 2025) **làm thành cấu hình**, không cài cứng; tài liệu nói
   rõ người cài phải tự kiểm văn bản gốc cho trường hợp của mình.
5. Người lạ cài Autoxide là chạy được, **không cần tài khoản dịch vụ bên thứ ba** (CAPTCHA,
   Akismet chỉ là tuỳ chọn cắm thêm).

## B. Phạm vi chung

**Không làm** ở đợt này: tải file kèm bài gửi form; email tự trả lời người gửi form; form nhiều
bước và logic điều kiện; thanh toán; đo lượt mở và lượt bấm link trong email; trình soạn email
kéo thả; reaction / vote cho bình luận; đăng nhập bình luận bằng tài khoản mạng xã hội; phát hiện
nội dung độc hại bằng AI; kết nối riêng từng nhà cung cấp email (SES, Mailgun…) — có một
endpoint bounce chung là đủ.

Cả ba module: trung lập với nghiệp vụ; không foreign key xuyên module; ID UUIDv7; cursor
pagination; envelope `Data` / `Page` / `Problem`; `If-Match` theo D-084/D-085 (bắt buộc với
endpoint sửa mới); `Idempotency-Key` cho lệnh ghi có xác thực (endpoint công khai chống gửi trùng
bằng nonce của token form — giai đoạn 0 mục 6); event qua outbox cùng transaction và chịu
được nhận lại; REST và MCP chung service; gọi mạng (CAPTCHA, Akismet, gửi mail) **ngoài**
transaction ghi; test chạy trên cả SQLite và Postgres.

---

## C. Cách làm: ba giai đoạn

### Giai đoạn 0 — nền chung trong core (agent chính, tuần tự)

Chi tiết từng mục, cấu hình, mã lỗi và test: phần **(0)** của mục "Khuyến nghị cho Autoxide" trong
`docs/research/forms-newsletter-comments.md`. Mẫu xuyên suốt giống `jobs_migration` /
`outbox_migration`: **kernel cung cấp code và hàm sinh migration, module tự thêm bảng
`<module>_…`** — luật "module sở hữu bảng của mình" vẫn giữ. Mỗi mục một commit, gate xanh trên cả
hai database sau mỗi mục.

1. **Khoá ký của kernel.** Cấu hình `AUTOXIDE_SECRET_KEY`; hàm dẫn khoá theo mục đích (HMAC với
   nhãn). Không cấu hình thì **sinh một lần và lưu trong database** (đi theo backup) thay vì sinh
   ngẫu nhiên mỗi lần chạy — để token form và đề chống spam vẫn đúng khi khởi động lại và khi chạy
   nhiều máy chủ, người lạ không phải cấu hình gì. (Báo cáo đề xuất sinh ngẫu nhiên lúc khởi động
   như `AUTOXIDE_AUTH_TOKEN_SECRET`; đề bài chọn lưu database vì lý do trên — ghi quyết định.)
   Không thứ gì sống lâu phụ thuộc khoá này: link huỷ đăng ký dùng token ngẫu nhiên lưu băm.
2. **`ClientInfo`** — extractor dùng chung (IP đã giải, user agent, `Origin`, `Referer`), cùng một
   hàm với rate limiter (hiện là hàm private). Thêm `AUTOXIDE_HTTP_TRUSTED_PROXY_COUNT` (mặc định
   1, giữ đúng D-079) cho chuỗi CDN → Next.js → API. Tài liệu cài đặt có sơ đồ.
3. **Băm IP**: gộp tiền tố (IPv4 /24, IPv6 /64) rồi HMAC; dạng `bucket` (khoá đổi theo ngày, cho bộ
   đếm) và `fingerprint` (lưu cùng bản ghi, có hạn xoá). Không lưu IP thô.
4. **`Throttle` lưu trong database**, tổng quát từ `LoginThrottle` của `auth`: đếm theo (khoá, cửa
   sổ) bằng một câu `UPDATE` có điều kiện; `throttle_migration` → `<module>_throttle`; đúng khi nhiều
   máy chủ và gọi đồng thời.
5. **Token dùng một lần** chuyển từ `auth` (`token.rs`, `reset.rs`, `verify.rs`) vào kernel:
   phát (mục đích, đối tượng, hạn, thời gian chờ gửi lại) / tiêu thụ bằng
   `UPDATE … WHERE token_hash = ? AND used_at IS NULL AND expires_at > now`;
   `tokens_migration` → `<module>_tokens`. `auth` chuyển sang bản chung ở commit sau, hành vi không
   đổi.
6. **Chống spam** — trait (`SubmissionGuard` hoặc tên khác) trả một trong ba: **nhận**, **cách ly**
   (kèm lý do, người gửi không biết), **từ chối** (kèm `code` ổn định). Các lớp: trường bẫy; token
   form do máy chủ ký (tuổi tối thiểu / tối đa, nonce); giới hạn theo IP băm và email băm; phát hiện
   trùng; chỗ cắm `ChallengeVerifier` (`none` / `altcha`) và `ContentClassifier` (`none`). Chạy
   **hai pha**: kiểm và gọi mạng **trước** transaction; **trong** transaction của module thì chèn
   nonce (`<module>_antispam_nonces`, unique) — đụng unique nghĩa là gửi lại, trả kết quả cũ. Đây là
   idempotency cho người lạ: **core hiện bỏ qua `Idempotency-Key` khi request không xác thực**.
   Request có `Principal` (website gọi bằng API key, MCP) bỏ qua bẫy, token và đề, nhưng vẫn bị
   giới hạn theo người dùng. Endpoint công khai trung lập để lấy token form và đề.
7. **ALTCHA v2 tự viết** (mặc định tắt): kiểm bằng `sha2` + `hmac` đang có — không thêm crate
   `altcha` vì nó kéo theo bản cũ của `hmac`/`sha2`; vector test sinh một lần bằng thư viện chính
   thức rồi commit làm fixture. Ghim phiên bản widget trong tài liệu.
8. **Mở rộng `Mail` / `Mailer`**: `reply_to`; header theo danh sách cho phép (`List-Unsubscribe`,
   `List-Unsubscribe-Post`, `List-Id`, `Auto-Submitted`), chặn CR/LF; lỗi chia vĩnh viễn (5xx) /
   tạm thời; khoá chống trùng ánh xạ vào `unique_key` của job `mail.send`; thất bại vĩnh viễn phát
   event trung lập kèm địa chỉ nhận. Sửa mọi chỗ gọi cũ trong cùng commit. Tra cách gắn header của
   `lettre` bằng skill `check-library-api`.
9. **`Actor` cho người lạ** trên outbox (khác `system`, không có user ID) để audit log ghi đúng.
10. **`ResourceDirectory`** — registry trung lập (giống sổ `PersonalData`): module đăng ký
    `target_type` và một resolver trả `{exists, public, title, summary, url, locale}`. `comments`
    dùng để kiểm đích; `newsletter` dùng để lấy tiêu đề và link bài mới. Không ai phụ thuộc thẳng
    `content`; `content` đăng ký resolver của mình (báo người làm `content` hoặc làm ở giai đoạn 2).
11. **`AUTOXIDE_HTTP_PUBLIC_URL`** — URL công khai của API, cần cho link huỷ một chạm.

**Để sau:** adapter Turnstile / hCaptcha / reCAPTCHA / Akismet (trait đã chừa chỗ); bỏ dấu tiếng
Việt trong `search::normalize` (làm cùng bước tìm kiếm, phải tính lại các cột `*_key`);
`PersonalData` theo email (bản đầu mỗi module tự có endpoint xuất / xoá theo email).

Xong giai đoạn 0: ghi các quyết định, viết **`docs/prompts/reader-input-contract.md`** — hợp đồng
chung ngắn cho ba subagent (cách gọi `SpamGuard`, hình dạng endpoint công khai, cách lưu bằng
chứng đồng ý, đặt tên trạng thái, cách xoá và xuất dữ liệu theo email) — rồi mới sang giai đoạn 1.

### Giai đoạn 1 — ba module song song (ba subagent)

- Chạy skill `split-work` trước: đo CPU và RAM còn rảnh; máy không đủ thì chạy hai hoặc một
  subagent một lúc, không ép ba.
- Mỗi subagent: một git worktree riêng tạo từ commit cuối của giai đoạn 0; **chỉ sửa trong
  `modules/<tên>/`**, cộng ba chỗ tối thiểu để chạy được: thêm vào `members`/dependency ở
  `Cargo.toml` gốc, đăng ký ở `crates/app/src/modules.rs`, và file sinh tự động (`docs/openapi.json`,
  `.env.example`) — ba chỗ này agent chính sẽ làm lại khi gộp.
- **Không sửa** `docs/DECISIONS.md`, `docs/ROADMAP.md`, `CLAUDE.md`, `STACK.md`, kernel. Quyết
  định viết nháp vào `modules/<tên>/DECISIONS-DRAFT.md`; thứ cần thêm vào kernel thì **dừng và
  báo agent chính**, không tự thêm.
- Đề bài cho từng subagent = mục A, B, hợp đồng chung, và mục D/E/F tương ứng của file này.
- Subagent chạy `./gate.sh` trong worktree của mình (gate tự dựng Postgres riêng, chạy song song
  được), báo lại: đã làm gì, quyết định nháp, chỗ chưa chắc.

### Giai đoạn 2 — gộp (agent chính)

- **Tự kiểm lại** báo cáo của từng subagent bằng cách đọc code (không tin báo cáo suông).
- Gộp lần lượt từng module; mỗi lần: chuyển quyết định nháp vào `docs/DECISIONS.md` với số thật
  (xoá file nháp), thêm thư viện mới vào `STACK.md`, sinh lại `docs/openapi.json` và
  `.env.example`, chạy `./gate.sh` trên cả hai database, đánh `[x]` trong lộ trình, commit.
- Chỉ push khi chủ dự án yêu cầu.

---

## D. Module `forms`

**Làm:** định nghĩa form; nhận bài gửi công khai; kiểm dữ liệu ở máy chủ; hộp thư quản trị (đọc,
đánh dấu, spam / không phải spam, lưu trữ); báo email cho quản trị viên; webhook qua event; xuất
CSV; xoá và xuất theo email (quyền của chủ dữ liệu); tự xoá theo thời hạn.

**Dữ liệu** (bảng `forms_…`):

| Bảng | Nội dung chính |
|---|---|
| `forms_forms` | `key` (slug, unique, dùng trong URL công khai), `name`, `status` (`open`/`closed`), `fields` (JSON), `version`, `allowed_origins`, `success_message` theo ngôn ngữ, `notify_emails` (**không bao giờ** trả ra endpoint công khai), `reply_to_field`, thiết lập chống spam, `retention_days` |
| `forms_form_versions` | ảnh chụp `fields` mỗi khi đổi trường — bài gửi cũ luôn đọc được |
| `forms_submissions` | `form_id`, `form_version`, `data` (JSON `{key: value}`), `status` (`new`/`read`/`archived`/`spam`), `spam_reason`, `email` (trường email nếu có, đã chuẩn hoá — để xoá/xuất theo email), `locale`, `page_url`, `referrer`, `user_agent`, `ip_fingerprint`, `consent` (văn bản đã hiện, phiên bản, lúc nào), thời gian |

**Trường** — danh sách đóng: `text`, `email`, `phone`, `textarea`, `number`, `select`, `radio`,
`checkbox` (nhiều lựa chọn), `date`, `hidden`, `consent` (ô đồng ý, lưu kèm văn bản). Mỗi trường:
`key` ổn định, nhãn / gợi ý / lựa chọn theo ngôn ngữ (vi, en), `required`, giới hạn độ dài hoặc
khoảng số. Máy chủ kiểm **mọi** trường theo kiểu, báo mọi lỗi một lần theo `key` (D-083) — sản
phẩm khác (Payload) bỏ qua bước này; ta không. Trường lạ bị từ chối.

**API:**

- Quản trị: `GET/POST /forms`, `GET/PATCH/DELETE /forms/{id}`; `GET /forms/{id}/submissions`
  (lọc theo trạng thái, tìm theo chữ), `GET/PATCH/DELETE /submissions/{id}`;
  `GET /forms/{id}/submissions/export?format=csv` (chặn CSV injection: ô bắt đầu bằng `=`, `+`,
  `-`, `@` thì thêm `'`); `GET /submissions?email=` và `DELETE /submissions?email=` (yêu cầu của
  chủ dữ liệu).
- Công khai: `GET /public/{key}` (định nghĩa form, bỏ phần riêng tư), `POST /public/{key}/submissions`.
  Trả **cùng một** câu trả lời cho nhận, cách ly và dính bẫy (không lộ cho bot). Có `Origin` mà
  không nằm trong `allowed_origins` thì từ chối. Form `closed` trả lỗi rõ.
- MCP: liệt kê form, xem form, liệt kê / xem bài gửi, đổi trạng thái. Không có tool gửi form
  (MCP luôn có người gọi đăng nhập).

**Permission:** `forms.read`, `forms.manage` (tạo, sửa form), `forms.review` (đổi trạng thái bài
gửi), `forms.export` (xuất CSV, xuất / xoá theo email).

**Event:** form tạo / sửa / xoá; `forms.submission_received` (chỉ bài được nhận, không phát cho
spam), bài gửi đổi trạng thái, bị xoá. Event **không mang câu trả lời** (chỉ ID bài gửi và `key`
của form): module `webhooks` tự chuyển mọi event ra ngoài, nên dữ liệu cá nhân không được nằm
trong payload — hệ thống ngoài lấy chi tiết qua API có permission. Email báo quản trị viên đi qua
job queue.

**Cấu hình** `AUTOXIDE_FORMS_…`: kích thước tối đa một bài gửi, thời hạn giữ bài spam (mặc định
30 ngày), thời hạn giữ mặc định của bài gửi (mặc định: giữ mãi, mỗi form chỉnh được).

**Test riêng:** kiểm từng kiểu trường và báo đủ lỗi; đổi trường form xong bài cũ vẫn đọc đúng;
bẫy / quá nhanh / quá nhiều lần trả cùng câu trả lời; gửi trùng với cùng `Idempotency-Key`; CSV
injection; xoá theo email; `notify_emails` không lộ ra endpoint công khai.

## E. Module `newsletter`

**Làm:** danh sách bản tin; người đăng ký; xác nhận hai bước; huỷ đăng ký một chạm (RFC 8058) và
trang chọn danh sách; chiến dịch (nháp, hẹn giờ, gửi, tạm dừng, tiếp tục, huỷ); gửi theo lô có
giới hạn tốc độ, không gửi trùng khi chạy lại; danh sách chặn (bounce, khiếu nại, thủ công);
endpoint bounce chung; tự tạo bản tin khi `content` đăng bài (tuỳ chọn theo danh sách).
**Không** đo lượt mở / lượt bấm.

**Dữ liệu** (bảng `newsletter_…`):

| Bảng | Nội dung chính |
|---|---|
| `newsletter_lists` | `key`, tên và mô tả theo ngôn ngữ, `double_opt_in` (mặc định bật), `public`, chế độ "bài mới" (`off`/`draft`/`send`, mặc định `off`) |
| `newsletter_subscribers` | `email` chuẩn hoá (unique), `name`, `locale`, `status` (`active`/`blocked`), `blocked_reason`, `attributes` (JSON), thời gian |
| `newsletter_subscriptions` | (`subscriber_id`, `list_id`) khoá chính, `status` (`unconfirmed`/`confirmed`/`unsubscribed`), `consent` (nguồn, văn bản đã hiện, lúc nào, dấu vết IP), `confirmed_at`, `unsubscribed_at` |
| `newsletter_campaigns` | danh sách nhận, `subject`, `preheader`, `body_text`, `body_html`, `advertising` (có phải thư quảng cáo), `status` (`draft`/`scheduled`/`sending`/`paused`/`sent`/`cancelled`), `scheduled_at`, `source` + `source_ref` (unique — một bài chỉ sinh một bản tin dù event bị giao lại), số đếm |
| `newsletter_deliveries` | (`campaign_id`, `subscriber_id`) unique, `status` (`pending`/`sent`/`failed`/`skipped`), `attempts`, `sent_at`, `error` |
| `newsletter_suppressions` | `email` unique, `reason` (`hard_bounce`/`complaint`/`manual`), thời gian |

**Luồng:**

- **Đăng ký** (công khai, qua `SpamGuard`): tạo hoặc cập nhật người đăng ký — **trả cùng một câu
  trả lời** dù email đã có hay chưa. Danh sách có `double_opt_in` thì gửi email xác nhận (token
  dùng một lần của kernel, có hạn); giới hạn số email xác nhận gửi tới một địa chỉ; email xác nhận
  không chứa chữ do người đăng ký gõ (tránh bị dùng làm công cụ gửi spam).
- **Xác nhận:** link trong email mở trang của website; website gọi `POST` để xác nhận — `GET`
  không đổi dữ liệu (máy quét link của hộp thư sẽ tự mở link).
- **Huỷ đăng ký:** mỗi email có header `List-Unsubscribe` (URL API, cần URL công khai ở giai đoạn
  0 mục 9) và `List-Unsubscribe-Post: List-Unsubscribe=One-Click`; link mang **token ngẫu nhiên riêng của
  từng đăng ký, lưu băm**, không hết hạn (không phụ thuộc khoá ký). Endpoint một chạm nhận thân
  `application/x-www-form-urlencoded` theo RFC 8058 — ngoại lệ so với `ValidJson`, ghi quyết định. Trang chọn danh sách (website) đọc / sửa qua token. Huỷ có hiệu lực ngay.
- **Gửi chiến dịch:** chụp danh sách người nhận vào `newsletter_deliveries` theo lô (mỗi lô một
  transaction ngắn — SQLite chỉ có một người ghi), bỏ người bị chặn hoặc chưa xác nhận; job gửi
  từng lô: nhận một nhóm `pending`, gửi qua `Mailer` với khoá chống trùng = ID delivery, cập nhật
  trạng thái. Giới hạn tốc độ bằng cấu hình; mỗi lô là một job có `unique_key` và `run_at`, cỡ lô
  phải gửi xong trong khoảng 240 giây (job queue giữ job 300 giây rồi coi là chết). Tạm dừng / tiếp tục / huỷ an toàn giữa chừng.
- **Bounce:** event thất bại vĩnh viễn từ `mail` (giai đoạn 0 mục 7) và endpoint chung
  `POST /bounces` (cần permission, cho dịch vụ ngoài gọi) → hard bounce 1 lần hoặc khiếu nại 1 lần
  là chặn; soft bounce theo ngưỡng cấu hình.
- **Bài mới:** nghe event "đã đăng" của `content` **bằng tên event** (không phụ thuộc crate
  `content`), lấy tiêu đề, tóm tắt, link qua `ResourceDirectory` (giai đoạn 0 mục 10). Chế độ
  `draft` (mặc định khi bật): tạo chiến dịch nháp chờ người duyệt; `send`: gửi luôn. Phần này làm
  **sau cùng**, khi `content` đã gộp và đã đăng ký resolver.
- **Thư quảng cáo** (`advertising = true`): thêm nhãn đầu tiêu đề và giới hạn số thư quảng cáo
  gửi tới một địa chỉ mỗi 24 giờ — cả hai lấy từ cấu hình (giá trị gợi ý theo Nghị định 91/2020:
  nhãn `[QC]`, tối đa 3 thư; tài liệu ghi rõ chưa đối chiếu văn bản gốc). Chân thư có link huỷ và
  địa chỉ người gửi (cấu hình).

**API:**

- Quản trị: danh sách (`/lists`), người đăng ký (`/subscribers`, tìm theo email, nhập nhiều người
  có kèm nguồn đồng ý), chiến dịch (`/campaigns`, `…/{id}/send`, `/schedule`, `/pause`, `/resume`,
  `/cancel`, `/test` — gửi thử tới một địa chỉ), danh sách chặn (`/suppressions`), `POST /bounces`,
  xuất / xoá theo email.
- Công khai: đăng ký, xác nhận, huỷ một chạm, đọc / sửa lựa chọn danh sách bằng token.
- MCP: liệt kê danh sách, tìm người đăng ký, tạo chiến dịch nháp, hẹn giờ / gửi (cần permission
  gửi), xem tiến độ.

**Permission:** `newsletter.read`, `newsletter.manage` (danh sách, người đăng ký, danh sách chặn),
`newsletter.send` (gửi, hẹn giờ, huỷ chiến dịch), `newsletter.export`.

**Cấu hình** `AUTOXIDE_NEWSLETTER_…`: URL trang xác nhận và trang lựa chọn của website, hạn token
xác nhận, tốc độ gửi mỗi phút, cỡ lô, ngưỡng soft bounce, nhãn và giới hạn thư quảng cáo, địa chỉ
người gửi ở chân thư.

**Test riêng:** đăng ký cùng email đồng thời; job gửi chạy lại hoặc chạy song song không gửi trùng
cho ai; event "đã đăng" giao lại hai lần chỉ sinh một chiến dịch; huỷ một chạm; người bị chặn
không nhận; giới hạn thư quảng cáo; tạm dừng rồi tiếp tục gửi đúng phần còn lại; header
`List-Unsubscribe` có mặt.

## F. Module `comments`

**Làm:** bình luận gắn vào **đích bất kỳ** theo (`target_type`, `target_id`); hai cấp (bình luận
gốc + trả lời, có `reply_to_id` để biết đang trả lời ai); người dùng đăng nhập hoặc khách;
kiểm duyệt; báo cáo vi phạm; cấm theo người dùng / email / dấu vết IP; sửa trong thời hạn; xoá mềm
giữ cấu trúc luồng; đếm số bình luận; mở / đóng bình luận theo đích; báo email khi có trả lời
(chọn nhận).

**Dữ liệu** (bảng `comments_…`):

| Bảng | Nội dung chính |
|---|---|
| `comments_comments` | `target_type`, `target_id`, `root_id`, `parent_id`, `reply_to_id`, `author_kind` (`user`/`guest`), `user_id`, `author_name`, `author_email` (khách, để báo trả lời), `email_verified`, `body` (chữ thường), `status` (`pending`/`published`/`rejected`/`spam`/`deleted`), `hold_reason`, `edit_token_hash`, `edited_at`, `ip_fingerprint`, `locale`, thời gian |
| `comments_targets` | (`target_type`, `target_id`) khoá chính, `state` (`open`/`closed`), `closes_at`, số bình luận đã hiện / đang chờ |
| `comments_bans` | `kind` (`user`/`email`/`ip`), `value`, `reason` (bắt buộc), `until` (trống = vĩnh viễn), người cấm |
| `comments_reports` | `comment_id`, người báo (user hoặc dấu vết IP), `reason`; unique (bình luận, người báo) |
| `comments_reply_subscriptions` | email đã xác minh, bình luận theo dõi, token huỷ |
| `comments_profiles` | `user_id` (khoá chính, không foreign key), `display_name` — `Principal` không có tên, và `comments` không đọc `auth` |

**Quy tắc:**

- `target_type` phải nằm trong danh sách cấu hình (ví dụ `content.entry`). Kiểm đích có tồn tại và
  công khai qua `ResourceDirectory` khi loại đích có resolver; không có resolver thì chấp nhận theo
  cấu hình. Không đọc bảng module khác. Dòng `comments_targets` tạo khi có bình luận đầu tiên. Có thể nghe event xoá / lưu trữ của module sở hữu đích (theo
  tên event) để đóng và ẩn bình luận.
- **Kiểm duyệt** theo thứ tự cố định: đích đóng → độ dài → lọc nội dung → trùng → bị cấm → từ cấm
  (từ chối) / từ nghi ngờ (giữ lại) → quá nhiều link (giữ lại) → `SpamGuard` → chế độ duyệt. Chế
  độ (cấu hình, chỉnh được theo loại đích, riêng cho khách và người dùng đăng nhập): `pre` (duyệt
  mọi bình luận), `first_time` (duyệt bình luận đầu tiên), `post` (hiện ngay). Mặc định: **khách
  `pre`**, người dùng đăng nhập `first_time`. "Đã từng được duyệt" chỉ tính theo tài khoản — không
  theo email khách tự khai (giả mạo được).
- **Khách** bật/tắt bằng cấu hình (`AUTOXIDE_COMMENTS_ALLOW_GUESTS`); có thể bắt buộc đăng nhập
  cho loại đích nhất định. Tên dành riêng (cấu hình) không cho khách dùng. Khách nhận một token sửa
  khi tạo bình luận (lưu băm) thay cho cookie.
- Người dùng đăng nhập cần permission `comments.post`; tên hiển thị lấy từ `comments_profiles`
  (người dùng tự đặt); bình luận có cờ "người dùng đã đăng nhập".
- **Nội dung bản đầu: chữ thường**, xuống dòng giữ nguyên, link tự nhận ra (chỉ `http`/`https`, có
  `rel="nofollow ugc noopener"`); máy chủ trả cả bản gốc lẫn bản HTML đã escape. Markdown
  (`pulldown-cmark` + `ammonia`) để sau. Không ảnh, không Gravatar.
- **Sửa / xoá:** tác giả sửa trong `AUTOXIDE_COMMENTS_EDIT_MINUTES` (mặc định 15); xoá là xoá mềm
  (xoá nội dung, giữ chỗ trong luồng); quản trị viên xoá hẳn được.
- **Báo cáo:** đủ số báo cáo (cấu hình) thì tự giữ lại chờ duyệt.
- **Gỡ theo yêu cầu** và mọi thao tác kiểm duyệt ghi audit log (phục vụ yêu cầu gỡ trong 24 giờ
  của Nghị định 147/2024 nếu áp dụng — tài liệu nói rõ chưa đối chiếu văn bản gốc).
- **Giữ dữ liệu:** bình luận chờ duyệt / spam tự xoá sau thời hạn (mặc định 30 ngày); dấu vết IP
  xoá sau thời hạn (mặc định 90 ngày).

**API:**

- Công khai: luồng theo đích (bình luận gốc đã hiện, mới nhất / cũ nhất, mỗi bình luận kèm vài
  trả lời đầu và tổng số trả lời; cursor), trả lời của một bình luận (cursor), đếm theo nhiều đích
  một lần (tối đa 100), tạo bình luận (qua `SpamGuard`), sửa / xoá bằng token sửa hoặc đăng nhập,
  báo cáo, xác minh email khách và huỷ theo dõi trả lời.
- Quản trị: hàng chờ duyệt (lọc trạng thái, đích, tìm theo chữ), đổi trạng thái (duyệt, từ chối,
  spam), xoá hẳn, cấm / bỏ cấm, mở / đóng đích, xuất / xoá theo email.
- MCP: xem hàng chờ, xem bình luận, duyệt / từ chối / đánh spam, mở / đóng đích.

**Permission:** `comments.post`, `comments.read` (xem cả bình luận chưa hiện), `comments.moderate`,
`comments.manage` (cấm, mở / đóng đích, cấu hình).

**Event:** bình luận tạo (kèm trạng thái), được hiện, đổi trạng thái, sửa, xoá; đích đóng / mở —
đủ để website làm mới cache qua webhook.

**Dữ liệu cá nhân:** nhận `privacy::USER_DELETED` → xoá mềm bình luận của người đó (giữ cấu trúc
luồng), bỏ trống `user_id`; `PersonalData` xuất bình luận của người dùng; khách thì xoá / xuất theo
email (quản trị viên).

**Test riêng:** trả lời đồng thời vào một luồng giữ số đếm đúng; báo cáo đồng thời tới ngưỡng chỉ
giữ lại một lần; XSS (`javascript:`, thẻ HTML, thuộc tính `on…`); khách lần đầu bị giữ, lần sau
(email đã xác minh) được hiện; email tự khai không vượt được duyệt; hết thời hạn sửa; bị cấm có
`until` hết hạn thì gửi lại được; cursor của luồng.

---

## G. Xong khi

- Giai đoạn 0, 1, 2 đều xong; các bước mới trong `docs/ROADMAP.md` đánh `[x]`; `./gate.sh` xanh
  trên cả hai database.
- Mỗi module có hướng dẫn dùng `docs/<MODULE>.md` + `docs/<MODULE>.vi.md` (cùng nội dung), trong
  đó có phần "pháp lý": các cấu hình liên quan và lời nhắc người cài tự kiểm văn bản gốc.
- `docs/openapi.json`, `.env.example`, bản dịch `vi` đã sinh lại và khớp.
- Báo cáo cho chủ dự án bằng tiếng Việt, lời thường: đã làm gì, quyết định nào đã ghi, việc họ
  cần làm (ví dụ cấu hình SMTP có ký DKIM trước khi gửi bản tin thật, URL công khai), việc để lại
  cho repo giao diện (form hiển thị trường bẫy và token, trang xác nhận và lựa chọn bản tin, khung
  bình luận).
