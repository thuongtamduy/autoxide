# Nháp quyết định của module `comments`

Agent chính chép các mục dưới đây vào `docs/DECISIONS.md` (đánh số tiếp theo) khi gộp, rồi xoá
file này. Nguồn: đề bài F (`docs/prompts/forms-newsletter-comments.md`), hợp đồng chung
(`docs/prompts/reader-input-contract.md`), khảo sát (`docs/research/forms-newsletter-comments.md`).

## `comments`: bảng, trạng thái, hai cấp, số đếm — agent quyết

Quyết định:

- Sáu bảng theo đề bài F: `comments_comments`, `comments_targets` (khoá chính `(target_type,
  target_id)`), `comments_bans`, `comments_reports` (unique `(comment_id, reporter_key)`),
  `comments_reply_subscriptions` (unique `(comment_id, email)`), `comments_profiles` (khoá chính
  `user_id`, unique `name_key`). Cộng các bảng của kernel theo thứ tự của hợp đồng: events
  (+ `actor_kind`), jobs, throttle, antispam nonces, tokens. Không foreign key, kể cả trong module.
- Cột ngoài bảng của đề bài: `depth` (0 / 1, cho index luồng), `published_at` (lần đầu được hiện —
  dùng cho "đã từng được duyệt"), `content_hash`, `search_key`, `reply_count`, `report_count`,
  `reports_held_at` (bình luận); `hidden` (đích); `reporter_user_id`, `note` (báo cáo);
  `name_key` (hồ sơ).
- Trạng thái đúng hợp đồng §4: `pending` / `published` / `rejected` / `spam` / `deleted`;
  `hold_reason` là lý do của `Verdict::Quarantine` nối bằng dấu phẩy, hoặc lý do riêng:
  `moderation`, `first_comment`, `reported`, `suspect_word`, `too_many_links`.
- Hai cấp như Ghost: bình luận gốc có `root_id` = chính nó; trả lời có `parent_id` = `root_id` =
  bình luận gốc và `reply_to_id` = bình luận đang được trả lời. Trả lời một trả lời gắn vào gốc.
- Số đếm (`published_count`, `pending_count` của đích; `reply_count` của bình luận gốc) đổi trong
  CÙNG transaction với trạng thái bằng `UPDATE … SET n = n + k`; đổi trạng thái dùng `UPDATE` có
  điều kiện trạng thái + `updated_at` cũ (đua thì 412). Dòng đích chèn `ON CONFLICT DO NOTHING`.
  Test đồng thời trên hai database.
- Bình luận gốc đã xoá mềm vẫn hiện (không chữ, không tên) khi còn trả lời đang hiện; trả lời đã
  xoá thì không hiện.

Đã cân nhắc: bảng `comments_revisions` của khảo sát — đề bài F không có; sửa chạy lại kiểm nội dung
là đủ chặn trò "đăng sạch rồi sửa bẩn". Đếm trả lời bằng `GROUP BY` khi đọc — bộ đếm cùng
transaction rẻ hơn và cho lọc "gốc đã xoá còn trả lời".

## `comments`: pipeline kiểm duyệt và chế độ duyệt; "đã từng được duyệt" chỉ theo tài khoản — agent quyết

Quyết định:

- Thứ tự cố định của đề bài F: đích đóng → độ dài → lọc nội dung (ký tự điều khiển / vô hình bị
  từ chối, xuống dòng chuẩn hoá) → trùng (cùng chữ, cùng đích, 24 giờ → cờ `duplicate` cho
  `SpamGuard`) → bị cấm (403 `comments.banned`) → từ cấm (422 `comments.banned_word`) / từ nghi
  ngờ (giữ `suspect_word`) → quá `AUTOXIDE_COMMENTS_MAX_LINKS` link (giữ `too_many_links`) →
  `SpamGuard` → chế độ duyệt. Lỗi đầu vào gom một lần (D-083) với mã trường `required`, `length`,
  `invalid`, `email`, `reserved`.
- `Verdict::Quarantine` → `spam` (kèm lý do; không tính vào `pending_count`); module tự giữ →
  `pending`; `Discard` → không lưu.
- Từ cấm / nghi ngờ / tên dành riêng so theo TỪ TRỌN (đã `search::fold`: không hoa thường, không
  dấu) để tránh lỗi "Scunthorpe".
- Chế độ `pre` / `first_time` / `post`, mặc định khách `pre`, người dùng `first_time`, chỉnh theo
  loại đích bằng `AUTOXIDE_COMMENTS_MODERATION_BY_TYPE` (`<loại>:guest=<chế độ>`). Người có
  `comments.moderate` bỏ qua chế độ (không bỏ qua giữ lại vì từ / link).
- "Đã từng được duyệt" = tài khoản có bình luận với `published_at` khác `NULL`. **Không bao giờ
  tính theo email**, kể cả email đã xác minh: khách gõ lại email mỗi lần, nên xác minh chỉ chứng
  minh hộp thư đó nhận được mail, không chứng minh người đang gửi là chủ hộp thư (lỗ của Isso /
  WordPress). Vì vậy `first_time` cho khách tương đương `pre` (ghi trong mô tả biến).
- Test "khách lần đầu bị giữ, lần sau (email đã xác minh) được hiện" của đề bài F mâu thuẫn với
  quy tắc trên; giải theo quy tắc: test kiểm khách lần đầu bị giữ, xác minh email, được duyệt, lần
  sau cùng email (đã xác minh) VẪN chờ duyệt, kể cả ở chế độ `first_time`; còn người dùng đăng
  nhập lần đầu chờ, được duyệt rồi thì lần sau hiện ngay.

Đã cân nhắc: tin email đã xác minh ở các lần sau (giả mạo được bằng cách gõ email người khác); cấp
cho khách đã xác minh một "khoá khách" giữ ở trình duyệt — thực chất là một kiểu tài khoản mới, để
sau nếu chủ dự án muốn.

## `comments`: endpoint công khai và câu trả lời giống nhau — agent quyết

Quyết định:

- Đường dẫn dưới `/api/v1/comments/public/…` (hợp đồng §2): `settings`, `threads`, `comments`
  (`POST`), `comments/{id}` (`GET`, `PATCH`), `comments/{id}/replies`, `comments/{id}/delete`
  (`POST`), `comments/{id}/reports`, `counts`, `targets/{type}/{id}`, `verify`, `unsubscribe`.
  Tên hiển thị: `GET` / `PUT /api/v1/comments/profile` (cần đăng nhập).
- Phạm vi token form `comments:<target_type>`; giới hạn: IP 20 / giờ, email 10 / giờ (khách), người
  dùng 60 / giờ — đều là cấu hình.
- Câu trả lời `201` chỉ nói `published` hay `pending`: giữ chờ duyệt, giữ vì spam và bị bỏ đều là
  `pending` (bỏ thì ID và token sửa giả cùng định dạng). Bài được hiện thì nói `published` — không
  lộ gì thêm, vì bot đọc luồng công khai cũng thấy. Đây là cách đọc hợp đồng §2 ("không lộ trạng
  thái thật") cho module mà người viết thật cần biết bình luận có đang chờ duyệt không.
- Gửi lại cùng token form (`Claim::Replay`) trả lại bình luận đầu, khách nhận token sửa MỚI (máy
  chủ chỉ giữ bản băm nên không trả lại được token cũ; token cũ thôi dùng được).
- Khách sửa / xoá bằng `edit_token` trong THÂN request (không có header riêng: CORS của kernel chỉ
  cho một danh sách header cố định, không được sửa kernel); vì vậy xoá của tác giả là `POST
  …/delete` thay vì `DELETE` (không đặt token vào URL). Người dùng đăng nhập gửi `{}`.
- Báo cáo không cần token form: mỗi người (user / dấu vết IP) một lần mỗi bình luận, giới hạn
  `AUTOXIDE_COMMENTS_REPORTS_PER_HOUR`, vượt thì trả lời như thường mà không lưu.
- Thân `POST /public/comments` và `PATCH /public/comments/{id}` tối đa 128 KiB (`Module::body_limits`).

## `comments`: kiểm duyệt, cấm, đích, MCP, permission — agent quyết

Quyết định:

- Quản trị: `GET /` (hàng chờ, mặc định `pending`, lọc trạng thái / đích, tìm không dấu theo
  `search_key`), `GET` / `PATCH` (`If-Match` bắt buộc) / `DELETE /{id}` (xoá hẳn; gốc thì cả luồng),
  `GET /{id}/reports`, `GET` / `POST /bans`, `DELETE /bans/{id}`, `GET` / `PUT
  /targets/{type}/{id}` (`If-Match` tuỳ chọn — đặt trạng thái kiểu bật / tắt như `pause` của
  newsletter), `GET` / `DELETE /data?email=`.
- Báo cáo đủ ngưỡng giữ lại ĐÚNG MỘT LẦN: `UPDATE` có điều kiện `status = 'published' AND
  reports_held_at IS NULL AND report_count >= ngưỡng`; duyệt lại rồi thì báo cáo sau không ẩn nữa.
- Cấm theo `user` (ID), `email` (chữ thường), `ip` (`ip_fingerprint` của bình luận — cùng khối
  /24 hay /64); `reason` bắt buộc; `until` phải ở tương lai, trống là vĩnh viễn. Event cấm không
  mang giá trị.
- Đích: `open` / `closed` + `hidden` (ẩn là đóng luôn) + `closes_at` (tự đóng;
  `AUTOXIDE_COMMENTS_AUTO_CLOSE_DAYS` đặt sẵn khi tạo dòng). Event `comments.target_opened` /
  `comments.target_closed` khi trạng thái thật đổi.
- Nghe `content.entry_archived` (đóng) và `content.entry_deleted` (đóng và ẩn) BẰNG TÊN, chỉ khi
  event có trong chương trình và `content.entry` được bật; ID lấy từ chủ thể `content.entry:<id>`.
  Không nghe `content.entry_unpublished`: bài về nháp thì sổ đối tượng báo không công khai nên bình
  luận mới bị từ chối, còn bình luận cũ ẩn theo trang của website.
- Permission đúng đề bài F (`comments.post`, `comments.read`, `comments.moderate`,
  `comments.manage`) **cộng `comments.export`** cho xuất / xoá theo email — hợp đồng §3 bắt
  `<module>.export`, và hợp đồng thắng đề bài.
- Tool MCP: `comments.queue`, `comments.get` (`comments.read`), `comments.moderate`
  (`comments.moderate`, nhận `version` như `If-Match`), `comments.set_target_state`
  (`comments.manage`). Không có tool viết bình luận hay xoá hẳn.
- Tên schema OpenAPI mang tiền tố `Comment…` / `Comments…` (D-134).

## `comments`: chữ thường, HTML an toàn tự viết — agent quyết

Quyết định: lưu chữ người viết (đã lọc); trả cả `body` lẫn `body_html` dựng khi đọc: escape `& < >
" '`, xuống dòng thành `<br>`, chỉ `http://` / `https://` thành link `rel="nofollow ugc noopener"`
(link dừng ở khoảng trắng hay `< > " '`, bỏ dấu câu cuối, giữ `)` khi có `(`). Khoảng 100 dòng tự
viết, có test XSS (`<script>`, `on…`, `javascript:`, `data:`, chèn thuộc tính qua `"`). Không thêm
thư viện; Markdown (`pulldown-cmark` + `ammonia`) để sau.

## `comments`: mail báo trả lời — agent quyết

Quyết định:

- Chọn nhận: `notify_replies: true` + `email` → mail xác minh (token `OneTimeTokens` mục đích
  `verify`, `subject` = ID bình luận, `data` = email, sống 72 giờ, tối đa
  `AUTOXIDE_COMMENTS_VERIFY_MAILS_PER_DAY` mail / địa chỉ / ngày bằng `Throttle`) → trang
  `AUTOXIDE_COMMENTS_VERIFY_URL` gửi token tới `POST /public/verify` → `email_verified` và dòng
  theo dõi. Mỗi bình luận xác minh riêng (không ai đăng ký được email người khác).
- Gửi khi trả lời được HIỆN: module nghe `comments.comment_published` của chính nó (kể cả khi được
  duyệt sau), gửi tới người theo dõi bình luận được trả lời (`reply_to_id`), trừ chính người trả
  lời. Mail không có chữ do người lạ gõ (hợp đồng §5): chỉ tiêu đề và đường dẫn của đích lấy qua sổ
  đối tượng (đích không có bộ tra thì một từ chung, không dùng `target_id` do client gửi),
  `AUTOXIDE_COMMENTS_SITE_URL` + đường dẫn, và link huỷ.
- Token huỷ RIÊNG CHO TỪNG MAIL (như D-124 của newsletter): 32 byte ngẫu nhiên, chỉ lưu băm trong
  `comments_tokens` (mục đích `unsubscribe`, `subject` = ID dòng theo dõi, sống 365 ngày) — bảng
  `comments_reply_subscriptions` vì vậy không có cột token (khác bảng của đề bài F: một token
  cho mỗi dòng mà chỉ lưu bản băm thì không đặt lại được vào mail sau). `POST /public/unsubscribe`
  luôn trả 200; `all: true` bỏ mọi dòng theo dõi của địa chỉ đó.
- `dedupe_key` = `comments.reply:<ID trả lời>:<ID dòng theo dõi>` và `comments.verify:<ID token>`.
- Thiếu `AUTOXIDE_COMMENTS_VERIFY_URL` hoặc `AUTOXIDE_COMMENTS_UNSUBSCRIBE_URL` thì tính năng tắt
  (không lỗi khởi động, `GET /public/settings` báo `reply_mails: false`).

## `comments`: dữ liệu cá nhân và thời hạn giữ — agent quyết

Quyết định:

- Xoá theo email / tài khoản bị xoá là xoá mềm giữ cấu trúc luồng (hợp đồng §3): chữ, tên, email,
  token sửa, dấu vết IP và (với tài khoản) `user_id` bị bỏ; dòng theo dõi và token bị xoá; một event
  `comments.comments_erased` cho cả lô, chỉ có ID. Hồ sơ tên bị xoá; báo cáo của người đó giữ lại
  nhưng `reporter_key` thành `deleted:<id báo cáo>`.
- Lệnh cấm theo email GIỮ LẠI khi xoá theo email (để còn chặn được) — ngoại lệ có lý do, như danh
  sách chặn của newsletter (D-132); bản xuất có nó.
- Tác giả tự xoá thì dấu vết IP giữ tới hạn thường (để còn cấm được kẻ phá).
- Job hằng ngày `comments.cleanup`: `pending` / `spam` quá `AUTOXIDE_COMMENTS_RETENTION_DAYS`
  (30) bị xoá hẳn — bình luận gốc còn bình luận khác trong luồng thì xoá mềm; dấu vết IP quá
  `AUTOXIDE_COMMENTS_IP_RETENTION_DAYS` (90) bị xoá; nonce, bộ đếm, token hết hạn bị dọn.
  `rejected` giữ lại (đề bài chỉ nói chờ duyệt / spam; quyết định của người kiểm duyệt là hồ sơ).
- `PersonalData` xuất hồ sơ, bình luận, báo cáo của tài khoản.

## `comments`: cấu hình — agent quyết

Quyết định: `AUTOXIDE_COMMENTS_ENABLED` (true), `…_TARGET_TYPES` (`content.entry`),
`…_ALLOW_GUESTS` (true), `…_SIGNED_IN_ONLY_TYPES`, `…_GUEST_MODERATION` (`pre`),
`…_USER_MODERATION` (`first_time`), `…_MODERATION_BY_TYPE`, `…_MAX_LENGTH` (4000), `…_MAX_LINKS`
(2), `…_EDIT_MINUTES` (15), `…_REPORT_THRESHOLD` (3), `…_AUTO_CLOSE_DAYS` (0), `…_BANNED_WORDS`,
`…_SUSPECT_WORDS`, `…_RESERVED_NAMES` (`admin,administrator,moderator,root,system`),
`…_PER_IP_PER_HOUR` (20), `…_PER_EMAIL_PER_HOUR` (10), `…_PER_USER_PER_HOUR` (60),
`…_REPORTS_PER_HOUR` (30), `…_RETENTION_DAYS` (30), `…_IP_RETENTION_DAYS` (90), `…_VERIFY_URL`,
`…_UNSUBSCRIBE_URL`, `…_SITE_URL`, `…_VERIFY_MAILS_PER_DAY` (3). Không biến nào bắt buộc lúc
khởi động. Bỏ `…_FLOOD_SECONDS` và `…_TRUSTED_AFTER` của khảo sát: giới hạn theo giờ của
`SpamGuard` và chế độ `first_time` đã che; `…_MAX_DEPTH` cố định 2 theo đề bài.
