# Bản tin

*English: [NEWSLETTER.md](NEWSLETTER.md)*

Module `newsletter` giữ các danh sách bản tin và người đăng ký, nhận đăng ký từ người lạ kèm mail
xác nhận (double opt-in — xác nhận hai bước), huỷ đăng ký bằng một chạm, và gửi chiến dịch theo
lô với tốc độ bạn chọn, không bao giờ gửi cùng một chiến dịch hai lần cho một người. Module giữ
danh sách chặn (suppression list) lấy từ bounce và khiếu nại, có thể biến mỗi bài mới của module
`content` thành một chiến dịch, và xuất hay xoá mọi thứ về một địa chỉ email. Module **không** đo
lượt mở hay lượt bấm. Chỉ có API: website hiển thị ô đăng ký, trang xác nhận và trang lựa chọn;
màn hình quản trị là dự án riêng. Đường dẫn gốc `/api/v1/newsletter`. Hợp đồng đầy đủ ở
[`openapi.json`](openapi.json).

Không cần tài khoản bên thứ ba: spam bị chặn bằng lớp chống spam dùng chung của core, mail đi qua
module `mail` (máy chủ SMTP của bạn).

## Trước khi gửi

1. **Máy chủ SMTP ký DKIM.** Gmail và Yahoo từ chối thư hàng loạt không được xác thực, và header
   huỷ một chạm (RFC 8058) chỉ có tác dụng khi chữ ký DKIM bao cả chúng. Đặt
   `AUTOXIDE_MAIL_SMTP_URL` và `AUTOXIDE_MAIL_FROM` tới một máy chủ ký cho tên miền của bạn (kèm
   SPF và DMARC).
2. **`AUTOXIDE_HTTP_PUBLIC_URL`** — địa chỉ của API này, vd `https://api.example.com`. Mỗi mail
   chiến dịch mang `List-Unsubscribe: <https://api.example.com/api/v1/newsletter/public/unsubscribe?token=…>`.
3. **`AUTOXIDE_NEWSLETTER_CONFIRM_URL`** — trang xác nhận của website, có `{token}`. Hãy đặt
   kể cả khi không danh sách nào bật double opt-in: đăng ký đáng ngờ và người vào lại danh sách
   đã rời đều được xác nhận qua mail ở mọi danh sách.
4. **`AUTOXIDE_NEWSLETTER_PREFERENCES_URL`** — trang lựa chọn của website, có `{token}`; chân mỗi
   mail chiến dịch có link tới đó.
5. **`AUTOXIDE_NEWSLETTER_POSTAL_ADDRESS`** — tên và địa chỉ bưu chính của người gửi, in ở chân
   thư; bắt buộc với chiến dịch quảng cáo.

Không biến nào ở trên cần để khởi động máy chủ: chức năng nào cần một biến còn thiếu thì trả
`503` với mã nêu tên nó (`public_url_missing`, `newsletter.confirm_url_missing`,
`newsletter.preferences_url_missing`, `newsletter.postal_address_missing`).

## Permission

| Permission          | Cho phép                                                                               |
| ------------------- | -------------------------------------------------------------------------------------- |
| `newsletter.read`   | Đọc danh sách, người đăng ký (dữ liệu cá nhân), chiến dịch kèm tiến độ, danh sách chặn |
| `newsletter.manage` | Danh sách, nhập và sửa người đăng ký, soạn chiến dịch nháp, danh sách chặn, bounce     |
| `newsletter.send`   | Gửi, hẹn giờ, tạm dừng, tiếp tục, huỷ chiến dịch; gửi thử; `new_posts: send` cho danh sách |
| `newsletter.export` | Xuất hoặc xoá mọi thứ về một địa chỉ email                                             |
| `newsletter.subscribe` | Thêm đăng ký mà không qua lớp chống spam dành cho người lạ (API key riêng của máy chủ website) |

Agent có `newsletter.manage` mà không có `newsletter.send` soạn được chiến dịch để một người duyệt
rồi gửi: sửa chiến dịch đang hẹn giờ thì nó về nháp, và cho danh sách tự gửi bài mới cần
`newsletter.send`, nên không thư nào đi mà không có người giữ `newsletter.send` đồng ý. Người đọc
website không cần gì: mọi thứ dưới `/api/v1/newsletter/public/` là công khai. Chỉ cấp
`newsletter.subscribe` cho API key của máy chủ website; đăng nhập thôi chưa đủ để bỏ qua lớp
chống spam.

## Danh sách

```bash
curl -X POST http://localhost:8080/api/v1/newsletter/lists \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -H "Idempotency-Key: $(uuidgen)" \
  -d '{"key": "weekly", "name": {"en": "Weekly news", "vi": "Tin hằng tuần"},
       "description": {"en": "Every Monday."}, "public": true}'
```

- `double_opt_in` (mặc định `true`): người đăng ký công khai nhận mail xác nhận và không nhận gì
  cho tới khi mở link trong đó.
- `public` (mặc định `false`): hiện ở `GET /public/lists` và cho người lạ đăng ký.
- `new_posts`: `off` (mặc định), `draft` hoặc `send` — xem mục "Bài mới". Đặt `send` cần
  `newsletter.send`.
- `PATCH` và `DELETE` cần `If-Match`. Danh sách còn chiến dịch chưa xong thì không xoá được
  (`409 newsletter.list_in_use`); xoá danh sách thì xoá luôn các đăng ký của nó.

## Website: ô đăng ký

1. `GET /api/v1/newsletter/public/lists?locale=vi` — các danh sách công khai và `token_scope`
   (`newsletter:subscribe`).
2. Mỗi lần hiện form (không bao giờ nằm trong HTML được cache):
   `GET /api/v1/antispam/token?scope=newsletter:subscribe` — một token form. Hiện một ô bẫy ẩn mà
   người thật để trống, và một ô đồng ý **không được tích sẵn**.
3. `POST /api/v1/newsletter/public/subscribe`:

```json
{"email": "ada@example.com", "lists": ["weekly"], "locale": "vi",
 "consent_text": "Gửi tôi bản tin hằng tuần.", "source": "footer",
 "form_token": "…", "trap": ""}
```

- Câu trả lời luôn là `202 {"data": {"accepted": true}}` — địa chỉ mới, đang chờ xác nhận, đã
  đăng ký, bị khoá, dính bẫy, gửi quá nhiều: không ai dò được địa chỉ nào đã có trong danh sách.
  Hãy nhắc người đọc xem hộp thư.
- Request chỉ kiểm dữ liệu và token chống spam rồi xếp yêu cầu vào hàng đợi; một job chạy nền
  lưu nó và gửi mail xác nhận ngay sau đó. Vì vậy thời gian trả lời cũng không phụ thuộc địa
  chỉ. Worker chạy job phải đang chạy (mặc định có, ở vai trò tiến trình `all`).
- Dữ liệu sai: `422` với mọi lỗi trong `errors` (key không phải danh sách công khai là
  `lists[0]`). Token thiếu hay hết hạn: `400 antispam.token_*` — tải lại form.
- Chữ đồng ý, phiên bản của nó, thời điểm, nguồn và dấu vết băm của khối địa chỉ mạng được lưu
  cùng đăng ký làm bằng chứng.
- Mail xác nhận chỉ có tên danh sách và link — không bao giờ có chữ người đọc gõ, để form không
  bị dùng gửi thư rác cho người khác. Mỗi địa chỉ nhận tối đa
  `AUTOXIDE_NEWSLETTER_CONFIRM_MAILS_PER_DAY` mail xác nhận mỗi ngày.
- Luôn cần mail xác nhận, kể cả ở danh sách không bật double opt-in, khi đăng ký bị bộ chống
  spam nghi ngờ (gửi quá nhanh…) và khi địa chỉ đã huỷ đăng ký danh sách đó hay vẫn đang chờ xác
  nhận: không ai đăng ký lại hộ người khác được.
- Mọi yêu cầu đăng ký đã lưu đều được đếm vào `AUTOXIDE_NEWSLETTER_CONFIRM_MAILS_PER_DAY` của địa
  chỉ đó, dù có cần mail hay không, để thời gian trả lời gần như nhau. Không xếp hàng được mail
  (vd chưa đặt `AUTOXIDE_NEWSLETTER_CONFIRM_URL` và danh sách không bật double opt-in) thì câu
  trả lời vẫn là `202`, lý do ghi vào log của máy chủ.
- Máy chủ website có thể gọi bằng API key của nó, nếu key đó có permission
  `newsletter.subscribe`: không cần token, giới hạn
  `AUTOXIDE_NEWSLETTER_SUBSCRIBE_PER_USER_PER_HOUR`. Người gọi đã đăng nhập khác (vd tài khoản tự
  đăng ký) đi qua đúng các bước kiểm như người lạ: không có token form thì nhận `400
  antispam.token_missing`.

## Website: trang xác nhận và trang lựa chọn

- **Xác nhận.** Mail có link tới `AUTOXIDE_NEWSLETTER_CONFIRM_URL`. Trang đó gửi
  `POST /api/v1/newsletter/public/confirm {"token": "…"}` (chỉ mở link — như máy quét link của
  hộp thư — không đổi gì). `200` liệt kê các danh sách đã xác nhận; `400
  newsletter.confirmation_invalid` nghĩa là link đã hết hạn (sau
  `AUTOXIDE_NEWSLETTER_CONFIRM_HOURS`) hoặc đã dùng.
- **Lựa chọn.** Mỗi mail chiến dịch có link tới `AUTOXIDE_NEWSLETTER_PREFERENCES_URL`. Trang đó
  đọc `GET /api/v1/newsletter/public/preferences?token=…` (địa chỉ, tên, ngôn ngữ, mọi danh sách
  công khai và danh sách người đó đang có) và lưu bằng
  `PUT /api/v1/newsletter/public/preferences?token=…`
  `{"lists": [{"key": "weekly", "subscribed": false}], "unsubscribe_all": false}`. Huỷ có hiệu
  lực ngay; vào một danh sách công khai từ đây được xác nhận ngay (token đến từ hộp thư của
  chính người đó).
- **Một chạm.** Ứng dụng mail gọi `POST /api/v1/newsletter/public/unsubscribe?token=…` với thân
  form `List-Unsubscribe=One-Click`; người đó rời danh sách của mail đó ngay. Câu trả lời như nhau
  với mọi token.

Link huỷ đăng ký không bao giờ hết hạn. Mỗi mail có token ngẫu nhiên riêng; chỉ bản băm được lưu.

## Chiến dịch

```bash
# Nháp (newsletter.manage)
curl -X POST http://localhost:8080/api/v1/newsletter/campaigns \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"list_id": "…", "subject": "This week", "body_text": "Hello…",
       "body_html": "<p>Hello…</p>", "locale": "en"}'
# Đọc, rồi gửi đúng phiên bản đó (newsletter.send)
curl -X POST http://localhost:8080/api/v1/newsletter/campaigns/$ID/send \
  -H "Authorization: Bearer $TOKEN" -H 'If-Match: "1762934400000000"'
```

- `POST /campaigns/{id}/test {"email": …}` gửi một mail thử. `GET /campaigns/{id}/preview` cho
  thư đúng như khi gửi (nhãn tiêu đề, chân thư), không gửi gì.
- `GET /settings` cho cấu hình đang dùng (nhãn quảng cáo và giới hạn mỗi ngày, địa chỉ bưu chính)
  và, trong `missing`, các cấu hình còn phải đặt trước khi gửi.
- `send` lấy mọi người đã xác nhận của danh sách ở thời điểm đó (ảnh chụp), rồi đưa mail vào hàng
  đợi mail theo lô `AUTOXIDE_NEWSLETTER_BATCH_SIZE`, tối đa `AUTOXIDE_NEWSLETTER_SEND_RATE_PER_MINUTE`
  mail mỗi phút. Ngay trước mỗi mail, người bị chặn, bị khoá hay vừa huỷ đăng ký bị bỏ qua.
- `schedule {"at": "2026-12-01T08:00:00Z"}`, `pause`, `resume`, `cancel`. Tạm dừng dừng sau lô
  đang gửi; tiếp tục gửi cho những người còn lại — không ai nhận một chiến dịch hai lần, kể cả khi
  job chạy lại hay chạy trên nhiều máy chủ. Huỷ thì bỏ qua các mail chưa gửi.
- `send` và `schedule` cần `If-Match`, để bản bạn đã duyệt là bản được gửi. Chỉ sửa được chiến
  dịch nháp hay đang hẹn giờ; chỉ xoá được chiến dịch chưa từng gửi.
- Sửa chiến dịch đang hẹn giờ (`PATCH`, có đổi thật) thì nó về `draft` và bỏ giờ hẹn; các job
  đang chờ theo giờ hẹn cũ không làm gì. Người có `newsletter.send` phải hẹn giờ hay gửi lại.
- `body_html` được lưu và gửi nguyên vẹn — **không được làm sạch**. Giao diện quản trị không bao
  giờ được chèn nó vào trang của mình: chỉ xem trước trong `<iframe sandbox>` (không chạy script,
  không cùng origin).
- `GET /campaigns/{id}` có `progress`: `total`, `pending`, `sent` (đã giao cho hàng đợi mail),
  `failed`, `skipped`, thêm `bounced`
  (bounce cứng, cũng tính trong `failed`) và `unsubscribed` (người nhận huỷ đăng ký bằng link của thư
  chiến dịch này).
- Mỗi mail có chân thư (theo `locale` của chiến dịch) với link trang lựa chọn và địa chỉ bưu
  chính, cùng header `List-Unsubscribe` và `List-Unsubscribe-Post: List-Unsubscribe=One-Click`.
- Một khe hở không đóng được: máy chủ chết ngay sau khi đưa mail vào hàng đợi, trước khi ghi lại,
  thì người nhận đó bị đánh `failed` (`interrupted`) thay vì liều gửi mail thứ hai.

## Danh sách chặn và bounce

- Địa chỉ bị chặn không nhận mail chiến dịch nào, dù đăng ký thế nào.
- Mail do máy chủ này gửi bị từ chối hẳn vì chính địa chỉ người nhận (`mail.delivery_failed`: mã
  mở rộng `5.1.x` hay `5.2.x`, như "không có hộp thư này") thì địa chỉ bị chặn (`hard_bounce`) và
  mail đó thành `failed`. Lỗi vĩnh viễn khác (sai mật khẩu SMTP, địa chỉ gửi không được phép, hết
  hạn mức gửi) là lỗi của bên gửi: mail được thử lại rồi vào dead-letter queue
  (`autoxide jobs failed`), không chặn người đăng ký nào.
- Dịch vụ gửi mail có báo bounce gọi `POST /api/v1/newsletter/bounces`
  `{"email": …, "type": "hard" | "soft" | "complaint", "reason": …}` bằng API key có
  `newsletter.manage`. `hard` và `complaint` chặn ngay; `soft` chặn sau
  `AUTOXIDE_NEWSLETTER_SOFT_BOUNCE_LIMIT` lần.
- `GET`/`POST /suppressions`, `DELETE /suppressions/{id}` để quản lý bằng tay.

## Người đăng ký

- `GET /subscribers?email=&list_id=&status=&blocked=` — tìm theo một phần địa chỉ, danh sách,
  trạng thái đăng ký, bị chặn hay không.
- `POST /subscribers/import` — tối đa 1000 người vào các danh sách ở trạng thái `confirmed`, bắt
  buộc `consent_source` (họ đã đồng ý ở đâu). Người đã huỷ một danh sách thì vẫn là đã huỷ.
- `PATCH /subscribers/{id}` (`If-Match`) — tên, ngôn ngữ, thuộc tính, `blocked`,
  `unsubscribe_list_ids`. `DELETE /subscribers/{id}` xoá hẳn.

## Bài mới

Danh sách có `new_posts` là `draft` nhận một chiến dịch nháp (tiêu đề, tóm tắt và link của bài)
mỗi khi module `content` đăng một **bài viết** (trang thì không); `send` thì gửi luôn (đặt
`send` cần `newsletter.send`).
`new_post_locale` chỉ lấy bài bằng một ngôn ngữ. Link là `AUTOXIDE_NEWSLETTER_SITE_URL` nối với
đường dẫn của bài; thiếu nó (hay thiếu cấu hình cần để gửi) thì chiến dịch chỉ là nháp. Một bài
sinh nhiều nhất một chiến dịch cho mỗi danh sách, mãi mãi — đăng lại không sinh gì mới. Nhập nhiều
bài cũ là đăng tất cả: để `new_posts` ở `off` hoặc `draft` trong lúc đó.

## Dữ liệu cá nhân

- `GET /api/v1/newsletter/data?email=…` xuất, `DELETE /api/v1/newsletter/data?email=…` xoá hẳn,
  mọi thứ về một địa chỉ: người đăng ký, đăng ký kèm bằng chứng đồng ý, mail đã gửi, link xác nhận
  (permission `newsletter.export`). **Dòng của danh sách chặn được giữ**, để địa chỉ đã bounce,
  khiếu nại hay xin không bao giờ nhận mail thì không bị gửi lại; xoá nó bằng
  `DELETE /suppressions/{id}` nếu người đó yêu cầu.
- Người đăng ký không gắn với tài khoản: yêu cầu đi theo email. Khi một tài khoản bị xoá, danh
  sách và chiến dịch nó tạo vẫn giữ; `created_by` thành `null`.
- Địa chỉ mạng không bao giờ được lưu: chỉ dấu vết băm có khoá của khối địa chỉ, nằm trong bằng
  chứng đồng ý.
- Event (và vì thế webhook) chỉ mang ID và key của danh sách, không bao giờ có địa chỉ hay tên.
- Một job hằng ngày xoá đăng ký chưa xác nhận sau `AUTOXIDE_NEWSLETTER_UNCONFIRMED_DAYS`.

## Pháp lý

Luật đòi gì ở một bản tin tuỳ bạn và người đọc ở đâu. Module cho bạn cấu hình, không cài cứng
luật vào code; **các nội dung dưới đây chưa được đối chiếu với văn bản pháp luật gốc — hãy tự
kiểm cho trường hợp của bạn trước khi dựa vào.**

- **Việt Nam, Nghị định 91/2020/NĐ-CP (chống thư rác):** chỉ gửi thư quảng cáo cho người đã đồng
  ý trước; có nhãn ở đầu tiêu đề thư quảng cáo; tối đa 3 thư quảng cáo tới một địa chỉ trong 24
  giờ trừ khi người nhận đồng ý khác; có thông tin người gửi và cách từ chối nhận tiếp. Giá trị
  mặc định của module theo cách hiểu này: đánh dấu chiến dịch `advertising: true`, và đặt
  `AUTOXIDE_NEWSLETTER_ADVERTISING_LABEL` (mặc định `[QC]`; có nguồn ghi `[ADV]`),
  `AUTOXIDE_NEWSLETTER_ADVERTISING_MAX_PER_DAY` (mặc định `3`) và
  `AUTOXIDE_NEWSLETTER_POSTAL_ADDRESS`. Bản tin biên tập có tính là quảng cáo hay không là do bạn
  quyết. Lúc viết, nghị định thay thế 91/2020 đang ở dạng dự thảo.
- **Việt Nam, Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15 và Nghị định 356/2025:** đồng ý phải tự
  nguyện, rõ ràng, chứng minh được; im lặng không phải là đồng ý; rút lại phải dễ như khi cho; người
  dân có quyền yêu cầu xoá dữ liệu. Module lưu chữ đồng ý, phiên bản, thời điểm và nguồn; bật xác
  nhận hai bước mặc định; huỷ bằng một chạm; xuất và xoá theo email. Hãy kiểm thời hạn phải trả
  lời yêu cầu.
- **Nơi khác:** vd GDPR và quy định ePrivacy của EU, hay luật CAN-SPAM của Mỹ (xử lý yêu cầu từ
  chối trong 10 ngày làm việc — module huỷ ngay).

## AI agent (MCP)

Tool `newsletter.list_lists`, `newsletter.find_subscribers`, `newsletter.get_campaign` (tiến độ),
`newsletter.create_campaign` (nháp), `newsletter.send_campaign` và `newsletter.schedule_campaign`
(gửi kèm `version` đã đọc, như `If-Match`), cùng permission như REST.

## Cấu hình

| Biến                                               | Mặc định                  |
| -------------------------------------------------- | ------------------------- |
| `AUTOXIDE_NEWSLETTER_CONFIRM_URL`                  | — (cần cho mail xác nhận) |
| `AUTOXIDE_NEWSLETTER_PREFERENCES_URL`              | — (cần để gửi)            |
| `AUTOXIDE_NEWSLETTER_SITE_URL`                     | — (link bài mới)          |
| `AUTOXIDE_NEWSLETTER_POSTAL_ADDRESS`               | — (cần cho thư quảng cáo) |
| `AUTOXIDE_NEWSLETTER_CONFIRM_HOURS`                | `168`                     |
| `AUTOXIDE_NEWSLETTER_CONFIRM_MAILS_PER_DAY`        | `3`                       |
| `AUTOXIDE_NEWSLETTER_UNCONFIRMED_DAYS`             | `30`                      |
| `AUTOXIDE_NEWSLETTER_SUBSCRIBE_PER_IP_PER_HOUR`    | `10`                      |
| `AUTOXIDE_NEWSLETTER_SUBSCRIBE_PER_EMAIL_PER_HOUR` | `5`                       |
| `AUTOXIDE_NEWSLETTER_SUBSCRIBE_PER_USER_PER_HOUR`  | `1000`                    |
| `AUTOXIDE_NEWSLETTER_SEND_RATE_PER_MINUTE`         | `300`                     |
| `AUTOXIDE_NEWSLETTER_BATCH_SIZE`                   | `100`                     |
| `AUTOXIDE_NEWSLETTER_SOFT_BOUNCE_LIMIT`            | `3`                       |
| `AUTOXIDE_NEWSLETTER_ADVERTISING_LABEL`            | `[QC]`                    |
| `AUTOXIDE_NEWSLETTER_ADVERTISING_MAX_PER_DAY`      | `3`                       |

Cần `AUTOXIDE_HTTP_PUBLIC_URL` để gửi. Lớp chống spam (`AUTOXIDE_ANTISPAM_*`) dùng chung cho mọi
module. Mọi biến được mô tả trong [`.env.example`](../.env.example).
