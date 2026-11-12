# Quyết định nháp của module `forms` (giai đoạn 1)

Agent chính chuyển các mục này vào `docs/DECISIONS.md` với số thật khi gộp, rồi xoá file này.
Mọi mục: 2026-11-12 — agent quyết.

## Module `forms`: bảng, trường, phiên bản, API

Quyết định:

- Ba bảng: `forms_forms` (định nghĩa hiện hành; cột JSON `fields`, `allowed_origins`,
  `success_message`, `notify_emails` lưu dạng chữ), `forms_form_versions` (ảnh chụp `fields`, unique
  `(form_id, version)`), `forms_submissions`. Cộng các bảng sinh từ kernel: `forms_events` (+ cột
  `actor_kind`), `forms_jobs`, `forms_throttle`, `forms_antispam_nonces`. Không foreign key, kể cả
  trong module: xoá form thì service xoá phiên bản và bài gửi cùng transaction.
- `forms_forms.version` bắt đầu từ 1 và chỉ tăng khi danh sách trường thật sự đổi (so sau khi chuẩn
  hoá); mỗi lần tăng chụp một dòng `forms_form_versions`. Bài gửi giữ `form_version`, đọc ra với
  trường và nhãn của đúng phiên bản đó (`answers[]` trong API). Vì có ảnh chụp nên KHÔNG khoá `key`
  của trường khi đã có bài gửi (báo cáo nghiên cứu đề xuất khoá) — bài cũ đọc theo ảnh chụp của nó;
  CSV lấy cột theo trường hiện hành rồi tới trường chỉ có ở phiên bản cũ.
- Trạng thái form `open` / `closed` (theo đề bài D, không theo báo cáo `draft/published/archived`);
  không có bước "publish". Form đóng: `GET /public/{key}` vẫn trả (kèm `status: closed`) để website
  báo đã đóng; gửi bài thì 409 `forms.closed`.
- Kiểu trường đóng: `text`, `email`, `phone`, `textarea`, `number`, `select`, `radio`, `checkbox`,
  `date`, `hidden`, `consent`. KHÔNG làm `message` (khối chỉ hiển thị) của báo cáo — đề bài D không có.
  Nhãn / gợi ý / placeholder / nhãn lựa chọn là `{en, vi}`, thiếu ngôn ngữ nào thì dùng ngôn ngữ kia.
  Mỗi form nhiều nhất một trường `consent`.
- Lỗi câu trả lời dùng mã chung `validation_failed`, mỗi lỗi một `FieldError` đường dẫn
  `data.<key>`, mã trường: `required`, `consent_required`, `type`, `length`, `email`, `phone`,
  `option`, `range`, `date`, `invalid` (ký tự điều khiển), `unknown` (trường lạ). Lỗi định nghĩa
  cũng `validation_failed`, đường dẫn `fields[i].…`. Không thêm mã `forms.validation_failed` riêng
  như báo cáo đề xuất: kernel đã có một cách báo lỗi trường.
- `number` nhận số JSON hoặc chuỗi số; `min` / `max` của trường là `f64` (không phải tiền — luật
  "tiền là số nguyên" không áp). Số nguyên được lưu dạng nguyên.
- Đường dẫn: quản trị `GET/POST /forms`, `GET/PATCH/DELETE /forms/{id}`, `GET /forms/{id}/submissions`,
  `GET /forms/{id}/submissions/export?format=csv`, `GET/PATCH/DELETE /forms/submissions/{id}`;
  xuất / xoá theo email `GET/DELETE /forms/data?email=` (theo hợp đồng chung §3, thay cho
  `/submissions?email=` của đề bài D); công khai `GET /forms/public/{key}` và
  `POST /forms/public/{key}/submissions`.
- `If-Match` bắt buộc với `PATCH` / `DELETE` form và `PATCH` bài gửi (endpoint sửa mới, D-084);
  `DELETE` một bài gửi không cần (xoá là xoá, không có gì để ghi đè).
- Xoá form xoá luôn mọi bài gửi của nó (event `forms.form_deleted` có `submissions_deleted`);
  muốn giữ bài thì đóng form. Đã cân nhắc: chặn xoá khi còn bài (409) — buộc quản trị viên xoá từng
  bài, không giúp gì cho quyền riêng tư.
- Permission theo đề bài D: `forms.read` (form và bài gửi), `forms.manage`, `forms.review` (đổi trạng
  thái, xoá một bài), `forms.export` (CSV, xuất / xoá theo email). Không tách
  `forms.submissions.*` như báo cáo.
- Danh sách bài gửi mặc định bỏ `spam` (hộp thư); `status=spam` mới thấy spam.
- Tool MCP: `forms.list`, `forms.get`, `forms.list_submissions`, `forms.get_submission`,
  `forms.set_submission_status` (nhận `version` như `If-Match`). Không có tool tạo / sửa form ở bản
  đầu (đề bài D chỉ yêu cầu năm tool này) và không có tool gửi form.

## Bài gửi công khai: `Origin`, email người gửi, bằng chứng đồng ý, trùng

Quyết định:

- `allowed_origins` rỗng = mọi nơi được gửi (người lạ cài xong chạy được ngay). Có danh sách thì
  request có `Origin` không nằm trong đó bị 403 `forms.origin_not_allowed`; request không có
  `Origin` (máy chủ gọi) không bị kiểm. Origin lưu dạng `scheme://host[:port]` viết thường.
- Email người gửi (cột `email`, trim + chữ thường, có index): trường `reply_to_field` nếu có trả
  lời, không thì trường `email` đầu tiên có trả lời. `reply_to_field` phải trỏ tới một trường
  `email` của form.
- Bằng chứng đồng ý: máy chủ tự lấy chữ của ô `consent` theo ngôn ngữ đã gửi (`locale`) từ định
  nghĩa — không nhận chữ do client gửi (hợp đồng chung §3 nói "client gửi kèm"; với form máy chủ biết
  chính xác chữ đã phát ra nên đáng tin hơn). Cột `consent` =
  `{text, version: "<phiên bản form>", at, source: "forms:<key>", field, ip_fingerprint}`; chỉ có khi
  ô được tích.
- Dò trùng: cùng form, cùng `content_hash` của câu trả lời đã chuẩn hoá, trong 24 giờ → báo
  `duplicate` cho `SpamGuard` (bài bị giữ ở `spam`).
- `page_url` lấy từ thân; không gửi thì lấy header `Referer` nếu là địa chỉ `http(s)` (header lạ
  không thành lỗi 422). `referrer` chỉ từ thân (`document.referrer`).
- Câu trả lời công khai: 201 `Data{id, message}` + `Location: /api/v1/forms/submissions/{id}` cho
  nhận, giữ lại và bỏ đi (bỏ đi thì ID giả UUIDv7). Theo D-113, token thiếu / sai / hết hạn vẫn là lỗi
  400 của `SpamGuard` (hợp đồng chung §6 liệt "token thiếu / sai" vào nhóm trả lời giống bài nhận,
  nhưng §2 và kernel coi đó là lỗi — giữ theo kernel; test chỉ kiểm bẫy / quá nhanh / quá nhiều).
- Giới hạn mặc định (cấu hình, form ghi đè được hai cái đầu): 10 bài / giờ / khối IP / form, 5 bài
  / giờ / email / form, 1000 bài / giờ / người gọi đã đăng nhập (máy chủ website gửi thay mọi khách
  bằng một API key nên ngưỡng cao). Đếm theo email nghĩa là gửi lại cùng token sau khi đã vượt
  ngưỡng nhận ID giả khác ID lần đầu — chấp nhận được (bot mới vượt ngưỡng).
- Kích thước thân của `POST /public/{key}/submissions` giới hạn bằng `Module::body_limits` theo
  `AUTOXIDE_FORMS_MAX_SUBMISSION_BYTES` (mặc định 64 KiB); service kiểm thêm cỡ JSON của câu trả lời.

## Mail báo quản trị viên, event, thời hạn giữ

Quyết định:

- Mail báo đi theo event: module tự nghe `forms.submission_received` của mình và gọi `dyn Mailer`
  (mail vào job queue của module `mail`). Không có job riêng trong `forms_jobs`: `Mailer::send` đã là
  hàng đợi bền, và event ghi cùng transaction với bài nên không sót. `dedupe_key` =
  `forms.notify:<id bài>:<sha256(địa chỉ nhận)>` (băm để vừa cột `unique_key` 200 ký tự) — event giao
  lại không sinh mail thứ hai. `Reply-To` = email người gửi. Ngôn ngữ mail theo
  `AUTOXIDE_FORMS_NOTIFY_LANGUAGE` (mặc định `en`). Mail chứa câu trả lời (chỉ gửi quản trị viên).
- Chuyển bài từ `spam` ra trạng thái khác phát thêm `forms.submission_received` (bài lọt bộ lọc
  thì quản trị viên vẫn được báo); bài spam không bao giờ phát event này.
- Event: `forms.form_created`, `forms.form_updated`, `forms.form_deleted`,
  `forms.submission_received`, `forms.submission_status_changed`, `forms.submission_deleted`,
  `forms.submissions_erased` (xoá theo email: chỉ ID và số lượng, không email),
  `forms.submissions_expired` (job dọn: form, số lượng, lý do `spam` / `retention`). Hình dạng
  `{before, after}`; tóm tắt form không có `notify_emails`, tóm tắt bài chỉ có `id`, `form_id`,
  `form_key`, `status`.
- Job `forms.cleanup` mỗi ngày: xoá spam quá `AUTOXIDE_FORMS_SPAM_RETENTION_DAYS` (30), xoá bài quá
  `retention_days` của form (`null` → `AUTOXIDE_FORMS_RETENTION_DAYS`, mặc định 0 = giữ mãi — an toàn
  cho dữ liệu; người cài tự đặt theo luật áp dụng), xoá `ip_fingerprint` quá
  `AUTOXIDE_FORMS_IP_RETENTION_DAYS` (90), dọn nonce và bộ đếm hết hạn. `ip_fingerprint` nằm trong
  bằng chứng đồng ý thì giữ cùng bằng chứng (mục đích khác: chứng minh đồng ý).
- Thêm hai cấu hình ngoài đề bài: `AUTOXIDE_FORMS_MAX_FIELDS` (50) và
  `AUTOXIDE_FORMS_IP_RETENTION_DAYS` (hợp đồng chung §3 bắt xoá dấu vết IP theo thời hạn), cùng ba
  giới hạn số lần và `AUTOXIDE_FORMS_NOTIFY_LANGUAGE`. Bỏ `AUTOXIDE_FORMS_STORE_IP` của báo cáo:
  chỉ lưu dấu vết băm, có hạn xoá.
- CSV tự viết (`csv.rs`, vài chục dòng): mọi ô trong ngoặc kép, ô bắt đầu bằng `=`, `+`, `-`, `@`,
  tab, CR thêm `'`, BOM UTF-8, CRLF. Không thêm thư viện. Xuất đồng bộ (đọc theo trang 200 dòng);
  xuất bằng job ghi vào `Storage` để sau (báo cáo cũng để sau).

Không thêm thư viện bên thứ ba nào.
