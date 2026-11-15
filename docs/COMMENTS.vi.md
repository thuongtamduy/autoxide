# Bình luận

*English: [COMMENTS.md](COMMENTS.md)*

Module `comments` cho người đọc bình luận vào bất cứ thứ gì website hiển thị — bài viết, trang,
ảnh — theo loại và id của nó (`target_type`, `target_id`). Luồng có hai cấp: bình luận gốc và các
trả lời (trả lời một trả lời thì vào cùng luồng và nhớ đang trả lời ai). Người dùng đăng nhập và
khách đều bình luận được; mọi bình luận đi qua một chuỗi kiểm tra cố định và, mặc định, bình luận
của khách chờ duyệt. Người đọc báo cáo được bình luận; người kiểm duyệt duyệt, từ chối, đánh dấu
spam, cấm và đóng đích; tác giả sửa được bình luận trong một lúc ngắn và xoá được bất cứ lúc nào.
Module chỉ có API: website hiển thị khung bình luận; màn hình quản trị là một dự án riêng. Đường
dẫn: `/api/v1/comments`. Hợp đồng đầy đủ nằm trong [`openapi.json`](openapi.json).

Không cần tài khoản dịch vụ bên thứ ba: spam bị chặn bởi lớp chống spam dùng chung của core (ô bẫy
ẩn, token form có chữ ký, giới hạn số lần, dò trùng; đề proof-of-work tự host tuỳ chọn) cộng danh
sách từ và giới hạn số link của module.

## Permission

| Permission          | Cho phép                                                                         |
| ------------------- | -------------------------------------------------------------------------------- |
| `comments.post`     | Bình luận khi đăng nhập, sửa và xoá bình luận của mình, đặt tên hiển thị           |
| `comments.read`     | Đọc mọi bình luận ở mọi trạng thái, kèm email tác giả, dấu vết mạng và báo cáo     |
| `comments.moderate` | Duyệt, từ chối, đánh dấu spam, giữ lại; xoá hẳn                                   |
| `comments.manage`   | Cấm và bỏ cấm; mở, đóng và ẩn đích                                                |
| `comments.export`   | Xuất và xoá mọi bình luận viết bằng một địa chỉ email                             |

Khách không cần permission nào: mọi thứ dưới `/api/v1/comments/public/` đều mở.

## Đích

`AUTOXIDE_COMMENTS_TARGET_TYPES` (mặc định `content.entry`) liệt kê các loại nhận bình luận. Khi
một module khác tra được loại đó (module `content` tra `content.entry`), bình luận chỉ được nhận
khi đích có thật và đang công khai; loại khác được nhận theo cấu hình. Đích có dòng riêng từ bình
luận đầu tiên, giữ trạng thái (`open` / `closed`, `hidden`, `closes_at`) và số đếm. Module sở hữu
loại đích khai event nào của nó đóng hay gỡ đích: khi `content` lưu trữ một bài, bình luận của bài
đóng lại; khi xoá bài, bình luận còn bị ẩn. `AUTOXIDE_COMMENTS_AUTO_CLOSE_DAYS` đóng đích sau một số
ngày kể từ bình luận đầu tiên.

## Hỏi đáp

Cùng các luồng đó làm hỏi đáp sản phẩm: câu hỏi là bình luận gốc, câu trả lời là trả lời của nó. Module
sở hữu đích có thể gắn **nhãn** cho tác giả, tính lúc viết và trả về dạng mã ổn định trong `badges` (vd
`seller` cho người quản lý sản phẩm, `verified_buyer` cho người đã nhận hàng); giao diện tự hiện nhãn
cho từng mã.

- Tìm câu hỏi của một đích: `GET /api/v1/comments/public/threads?target_type=…&target_id=…&q=bảo hành`
  — không phân biệt hoa thường và dấu; khớp đúng dấu xếp trước.
- Số trên tab: `GET /api/v1/comments/public/counts` trả `thread_count` (câu hỏi đang hiện) bên cạnh
  `count` (mọi bình luận đang hiện); đích cũng trả `thread_count`, người kiểm duyệt có thêm
  `pending_thread_count`.
- Câu hỏi chưa trả lời (chưa có trả lời đang hiện): `GET /api/v1/comments?status=published&unanswered=true`.
- `AUTOXIDE_COMMENTS_VERIFIED_ONLY_TYPES` liệt kê loại đích chỉ tài khoản đã xác thực được hỏi hay trả
  lời: khách bị từ chối, người dùng đăng nhập được hỏi qua module giữ trạng thái xác thực tài khoản
  (`comments.verification_required`, 403). Chưa cài module đó thì mọi tài khoản đăng nhập tính là đã
  xác thực.

## Website

1. `GET /api/v1/comments/public/settings` — giới hạn và công tắc (có nhận khách không, số phút
   được sửa, độ dài tối đa, mail báo trả lời có bật không, Markdown có bật không).
2. `GET /api/v1/comments/public/threads?target_type=content.entry&target_id=…&order=desc&limit=20&replies_limit=3`
   — bình luận gốc đang hiện kèm vài trả lời đầu và `reply_count`; đi tiếp bằng
   `pagination.next_cursor`. Thêm trả lời: `GET /api/v1/comments/public/comments/{id}/replies`.
   Số đếm cho trang danh sách: `GET /api/v1/comments/public/counts?target_type=content.entry&target_ids=a,b,c`
   (tối đa 100). Khung bình luận có mở không: `GET /api/v1/comments/public/targets/{type}/{id}`.
3. Hiển thị `body_html` (mọi thứ đã escape, xuống dòng thành `<br>`, địa chỉ `http(s)` thành link
   có `rel="nofollow ugc noopener"`); không bao giờ chèn `body` như HTML. Ghi rõ khách là khách
   (`author.kind`). Xem "Chữ thường và Markdown" bên dưới.
4. Mỗi lần hiện khung (không bao giờ trong HTML đã cache): `GET /api/v1/antispam/token?scope=comments:content.entry`.
   Hiện một ô bẫy ẩn mà người thật để trống.
5. `POST /api/v1/comments/public/comments`:

```json
{"target_type": "content.entry", "target_id": "0190…", "parent_id": null,
 "body": "Bài hay!\nCảm ơn.", "author_name": "Ada", "email": "ada@example.com",
 "notify_replies": true, "locale": "vi", "form_token": "…", "trap": ""}
```

- Câu trả lời là `201` kèm bình luận, `status` (`published`, hoặc `pending`: chờ duyệt) và, với
  khách, một `edit_token` chỉ hiện ở đây. Bình luận chờ duyệt, bị giữ vì spam hay bị bộ chống spam
  bỏ đi đều trả `pending`: bot không biết gì thêm.
- Dữ liệu sai: `422` kèm mọi lỗi trong `errors` (chữ rỗng, quá dài, tên dành riêng, email sai…).
  Từ cấm: `422 comments.banned_word`. Token thiếu hay hết hạn: `400 antispam.token_*` — tải lại
  form. Đích đã đóng: `409 comments.target_closed`. Tác giả bị cấm: `403 comments.banned`.
- Gửi lại cùng `form_token` thì nhận lại đúng bình luận đó (với `edit_token` mới; token cũ thôi
  dùng được), không lưu thêm gì.
- Người dùng đăng nhập gửi access token thay cho token form, cần `comments.post`, và trước hết chọn
  tên hiển thị bằng `PUT /api/v1/comments/profile {"display_name": "…"}`.
- Tên (`author_name` của khách, tên hiển thị) dài 1 tới 50 ký tự, không có ký tự điều khiển, ký tự
  định dạng vô hình (gạch nối mềm, dấu hướng chữ, ký tự thẻ…), chữ trông như khoảng trống (chữ đệm
  Hangul và tương tự), hay một từ trộn chữ Latin, Hy Lạp và Kirin (`аdmin` với `а` Kirin). Chữ
  trông giống nhau giữa các bảng chữ trong cả một từ (tên viết trọn bằng chữ Kirin) thì không bắt
  được.
- Sửa: `PATCH /api/v1/comments/public/comments/{id} {"body": "…", "edit_token": "…"}` trong
  `AUTOXIDE_COMMENTS_EDIT_MINUTES`. Chữ được kiểm lại, và `status` của câu trả lời cho biết bình
  luận còn hiện không: bình luận đang hiện chờ duyệt lại khi giờ có từ nghi ngờ hay quá nhiều link,
  hoặc khi một bình luận mới của cùng tác giả trên đích đó sẽ phải chờ (chế độ `pre`, hay
  `first_time` khi tài khoản chưa có bình luận nào đang hiện) — nên mặc định bản sửa của khách luôn
  chờ duyệt lại. Xoá: `POST /api/v1/comments/public/comments/{id}/delete {"edit_token": "…"}` (tác
  giả đăng nhập gửi `{}`) — chữ, tên, email mất, chỗ trong luồng giữ lại. ID không tồn tại cũng trả
  `200`, nên không thể xoá thử để phân biệt bình luận bị bỏ với bình luận bị giữ. Sửa thử thì được:
  `PATCH` với ID giả của bình luận bị bỏ trả `404 comments.not_found` (giới hạn đã biết; bot chỉ
  biết đúng một bình luận đó bị bỏ).
- Báo cáo: `POST /api/v1/comments/public/comments/{id}/reports {"reason": "spam" | "abuse" |
  "illegal" | "other", "note": "…"}`. Mỗi tài khoản báo một bình luận một lần; khách được đếm theo
  khối mạng (`/24` với IPv4, `/48` với IPv6).

## Chữ thường và Markdown

Bình luận gửi kèm `format`: `text` (mặc định) hay `markdown`. `markdown` chỉ dùng được khi
`GET /api/v1/comments/public/settings` trả `markdown: true` (`AUTOXIDE_COMMENTS_MARKDOWN`, mặc định
bật; tắt thì `markdown` bị từ chối ở trường `format`). Sửa mà không gửi `format` thì giữ định dạng
của bình luận. Bình luận cũ là `text` và hiện như trước.

- `text`: chữ thường. Dấu `*` vẫn là `*`; xuống dòng được giữ; địa chỉ `http(s)` thành link.
- `markdown`: một phần nhỏ của Markdown — `**đậm**`, `*nghiêng*`, `[link](https://…)` và
  `<https://…>`, `> trích dẫn`, `` `code` `` và khối code. Tiêu đề và mục danh sách hiện như đoạn
  văn; ảnh hiện thành chữ của nó (không bao giờ tải ảnh); HTML thô hiện thành chữ. Chỉ link
  `http(s)` dùng được, đều có `rel="nofollow ugc noopener"`.

Máy chủ dựng `body_html` từ một danh sách thẻ cố định (`p`, `br`, `strong`, `em`, `a`,
`blockquote`, `code`, `pre`), nên website chèn nguyên `body_html` ở cả hai định dạng. Link Markdown
được tính vào `AUTOXIDE_COMMENTS_MAX_LINKS` như địa chỉ trần. Từ cấm được so trên chữ người viết.

## Kiểm duyệt

Thứ tự kiểm: đích đóng → độ dài → nội dung (ký tự điều khiển và ký tự vô hình bị từ chối) → trùng
→ bị cấm → từ cấm (từ chối) / từ nghi ngờ (giữ lại) → quá nhiều link (giữ lại) → chống spam → chế
độ duyệt. Chế độ: `pre` (mọi bình luận chờ duyệt), `first_time` (tới khi **tài
khoản** có một bình luận đang hiện — bình luận được duyệt rồi bị đánh spam hay bị từ chối không
tính) và `post` (hiện ngay); mặc định `pre` cho khách và `first_time` cho người dùng,
chỉnh được theo loại đích (`AUTOXIDE_COMMENTS_MODERATION_BY_TYPE`). Email của khách không bao giờ
là bằng chứng danh tính — ai cũng gõ được địa chỉ bất kỳ — nên nó không bao giờ giúp bình luận
vượt duyệt, kể cả khi đã xác nhận.

- `GET /api/v1/comments?status=pending&target_type=&target_id=&q=` — hàng chờ (`q` tìm trong chữ
  và tên, không phân biệt hoa thường, dấu), kèm email, dấu vết mạng, lý do giữ lại và số báo cáo.
  Mỗi bình luận có `target_title` và `target_url` khi module sở hữu đích mô tả được nó và đích đang
  công khai.
- `GET /api/v1/comments/targets?target_type=&state=open|closed&pending=true|false` — các đích có
  bình luận, mới trước, kèm số đếm và (khi công khai) `title`, `url`.
- `PATCH /api/v1/comments/{id} {"status": "published" | "rejected" | "spam" | "pending"}` kèm
  `If-Match`. `DELETE /api/v1/comments/{id}` xoá hẳn (bình luận gốc thì xoá cả luồng).
- Đủ `AUTOXIDE_COMMENTS_REPORT_THRESHOLD` báo cáo thì bình luận đang hiện bị giữ lại một lần; người
  kiểm duyệt duyệt lại rồi thì báo cáo sau không ẩn nó nữa. Bình luận bị giữ như vậy nằm trong hàng
  chờ tới khi người kiểm duyệt quyết: job dọn không bao giờ xoá bình luận đã từng hiện.
  `GET /api/v1/comments/{id}/reports`.
- `POST /api/v1/comments/bans {"kind": "user" | "email" | "ip", "value": "…", "reason": "…",
  "until": null}` (`ip` nhận `ip_fingerprint` của bình luận), `GET`, `DELETE
  /api/v1/comments/bans/{id}`.
- `PUT /api/v1/comments/targets/{type}/{id} {"state": "closed", "hidden": false, "closes_at":
  null}`.

Mọi quyết định là một event (`comments.comment_status_changed`, `comments.comment_deleted`,
`comments.ban_created`, `comments.target_closed`…) mà module `audit` ghi lại kèm ai làm, lúc nào.

## Mail báo trả lời

Tắt cho tới khi đặt cả `AUTOXIDE_COMMENTS_VERIFY_URL` lẫn `AUTOXIDE_COMMENTS_UNSUBSCRIBE_URL`.
Gửi `notify_replies: true` kèm `email` thì một mail xác nhận tới địa chỉ đó (nhiều nhất
`AUTOXIDE_COMMENTS_VERIFY_MAILS_PER_DAY`); trang của website gửi token trong đó tới `POST
/api/v1/comments/public/verify`. Từ đó, mỗi trả lời được hiện cho bình luận ấy gửi một mail tới
địa chỉ đó, chỉ có tiêu đề và link của đích — không bao giờ có chữ của trả lời hay tên người trả
lời, để tính năng này không thành công cụ gửi spam — và một link mà token của nó gửi tới `POST
/api/v1/comments/public/unsubscribe` (`"all": true` thôi nhận mail về mọi bình luận theo dõi bằng
địa chỉ đó).

Trên máy chủ có nhiều khách, token có thể bắt đầu bằng `t<id khách>.`: token tự nêu khách của nó, nên
lệnh xác nhận địa chỉ và thôi nhận mail chạy được trên mọi tên miền của API mà không cần header
`Autoxide-Tenant`. Gửi lại token đúng nguyên văn (token có thể chứa dấu `.`).

## Dữ liệu cá nhân

- `GET /api/v1/comments/data?email=…` liệt kê, và `DELETE /api/v1/comments/data?email=…` xoá, mọi
  bình luận viết bằng một địa chỉ (permission `comments.export`): chữ, tên, email và dấu vết mạng
  mất, chỗ trong luồng giữ lại, mail báo dừng. Lệnh cấm địa chỉ đó được giữ (nó có để chặn địa chỉ
  đó); bỏ bằng tay nếu được yêu cầu.
- Địa chỉ mạng không bao giờ được lưu: chỉ lưu bản băm có khoá của khối mạng, xoá sau
  `AUTOXIDE_COMMENTS_IP_RETENTION_DAYS` — ở bình luận và ở báo cáo của khách (báo cáo giữ lại,
  `reporter_key` thành `expired:<id báo cáo>`).
- Bình luận chờ duyệt hay spam **chưa từng hiện** bị xoá sau `AUTOXIDE_COMMENTS_RETENTION_DAYS` kể
  từ lúc gửi. Bình luận đã từng hiện rồi quay về chờ (bị giữ vì báo cáo, tác giả sửa, người kiểm
  duyệt giữ lại hay đánh spam) không bao giờ tự bị xoá: người kiểm duyệt duyệt, từ chối hay xoá nó.
- Event (và vì thế webhook) chỉ mang id, đích và trạng thái, không bao giờ có chữ, tên hay địa chỉ.
- Khi một tài khoản bị xoá, bình luận của nó được xoá theo cùng cách, tên hiển thị bị xoá và báo
  cáo của nó không còn chỉ về nó. Bản xuất dữ liệu của tài khoản có bình luận, tên hiển thị và báo
  cáo của nó.

## Pháp lý

Luật đòi gì ở bình luận tuỳ vào nơi bạn và người đọc ở. Module cho bạn cấu hình, không cài luật
cứng trong code; **các điều dưới đây chưa được đối chiếu với văn bản luật gốc — hãy tự kiểm cho
trường hợp của bạn trước khi dựa vào.**

- **Việt Nam, Nghị định 147/2024/NĐ-CP (dịch vụ internet):** theo các nguồn phụ, mạng xã hội chỉ
  được cho tài khoản đã xác thực bằng số điện thoại Việt Nam bình luận; trang thông tin điện tử
  tổng hợp có giấy phép có thể không được cho người đọc bình luận tin bài; nội dung vi phạm phải gỡ
  trong 24 giờ khi cơ quan có thẩm quyền yêu cầu, và một số dịch vụ phải lưu nhật ký xử lý (có
  nguồn nói 2 năm). Module có `AUTOXIDE_COMMENTS_ENABLED`, `AUTOXIDE_COMMENTS_ALLOW_GUESTS`,
  `AUTOXIDE_COMMENTS_SIGNED_IN_ONLY_TYPES`, đóng hay ẩn đích, xoá hẳn (`DELETE
  /api/v1/comments/{id}`, ghi vào audit log kèm ai làm, lúc nào) và lý do báo cáo `illegal`.
  `AUTOXIDE_COMMENTS_VERIFIED_ONLY_TYPES` hỏi module giữ trạng thái xác thực số điện thoại hay định
  danh (không phải module này) trước khi nhận bình luận. Thời gian giữ audit log do module
  `audit` quyết.
- **Việt Nam, Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15 và Nghị định 356/2025:** người dùng có quyền
  xin dữ liệu và xin xoá; module xuất và xoá theo email và theo tài khoản, chỉ giữ dữ liệu mạng ở
  dạng băm và trong thời hạn cấu hình. Hãy kiểm thời hạn phải trả lời yêu cầu, và việc giữ nhật ký
  kiểm duyệt sau khi xoá (chỉ id, hành động, thời điểm) đã đủ với bạn chưa.
- **Nơi khác:** ví dụ GDPR của EU (quyền của chủ dữ liệu) và Digital Services Act (thông báo và xử
  lý nội dung bất hợp pháp).

## AI agent (MCP)

Tool `comments.queue` và `comments.get` (`comments.read`), `comments.moderate`
(`comments.moderate`; gửi `version` đã đọc, như `If-Match`) và `comments.set_target_state`
(`comments.manage`), cùng permission và cùng kiểm tra như REST. Cho hỏi đáp: `comments.unanswered`
(`comments.read`) liệt kê câu hỏi đang hiện chưa có trả lời đang hiện, và `comments.answer`
(`comments.post`) trả lời một câu bằng tài khoản của người gọi — như người dùng đăng nhập trên website,
cùng nhãn, cùng kiểm xác thực và kiểm duyệt (đặt tên hiển thị trước bằng `PUT
/api/v1/comments/profile`). Không có tool mở luồng mới (đó là việc của người đọc trên website) và không
có tool xoá hẳn (dùng REST).

## Cấu hình

| Biến                                       | Mặc định                                    |
| ------------------------------------------ | ------------------------------------------- |
| `AUTOXIDE_COMMENTS_ENABLED`                | `true`                                      |
| `AUTOXIDE_COMMENTS_TARGET_TYPES`           | `content.entry`                             |
| `AUTOXIDE_COMMENTS_ALLOW_GUESTS`           | `true`                                      |
| `AUTOXIDE_COMMENTS_SIGNED_IN_ONLY_TYPES`   | —                                           |
| `AUTOXIDE_COMMENTS_VERIFIED_ONLY_TYPES`    | —                                           |
| `AUTOXIDE_COMMENTS_GUEST_MODERATION`       | `pre`                                       |
| `AUTOXIDE_COMMENTS_USER_MODERATION`        | `first_time`                                |
| `AUTOXIDE_COMMENTS_MODERATION_BY_TYPE`     | —                                           |
| `AUTOXIDE_COMMENTS_MAX_LENGTH`             | `4000`                                      |
| `AUTOXIDE_COMMENTS_MAX_LINKS`              | `2`                                         |
| `AUTOXIDE_COMMENTS_MARKDOWN`               | `true`                                      |
| `AUTOXIDE_COMMENTS_EDIT_MINUTES`           | `15`                                        |
| `AUTOXIDE_COMMENTS_REPORT_THRESHOLD`       | `3`                                         |
| `AUTOXIDE_COMMENTS_AUTO_CLOSE_DAYS`        | `0` (không bao giờ)                         |
| `AUTOXIDE_COMMENTS_BANNED_WORDS`           | —                                           |
| `AUTOXIDE_COMMENTS_SUSPECT_WORDS`          | —                                           |
| `AUTOXIDE_COMMENTS_RESERVED_NAMES`         | `admin,administrator,moderator,root,system` |
| `AUTOXIDE_COMMENTS_PER_IP_PER_HOUR`        | `20`                                        |
| `AUTOXIDE_COMMENTS_PER_EMAIL_PER_HOUR`     | `10`                                        |
| `AUTOXIDE_COMMENTS_PER_USER_PER_HOUR`      | `60`                                        |
| `AUTOXIDE_COMMENTS_REPORTS_PER_HOUR`       | `30`                                        |
| `AUTOXIDE_COMMENTS_RETENTION_DAYS`         | `30`                                        |
| `AUTOXIDE_COMMENTS_IP_RETENTION_DAYS`      | `90`                                        |
| `AUTOXIDE_COMMENTS_VERIFY_URL`             | — (cần cho mail báo trả lời)                |
| `AUTOXIDE_COMMENTS_UNSUBSCRIBE_URL`        | — (cần cho mail báo trả lời)                |
| `AUTOXIDE_COMMENTS_SITE_URL`               | — (link trang trong mail báo trả lời)       |
| `AUTOXIDE_COMMENTS_VERIFY_MAILS_PER_DAY`   | `3`                                         |

Lớp chống spam (`AUTOXIDE_ANTISPAM_*`) dùng chung cho mọi module. Mọi biến được mô tả trong
[`.env.example`](../.env.example).
