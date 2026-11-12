# Nháp quyết định của module `newsletter`

Nháp để agent chính chép vào `docs/DECISIONS.md` (đánh số tiếp theo) khi gộp. Nguồn: đề bài
`docs/prompts/forms-newsletter-comments.md` mục E, hợp đồng chung
`docs/prompts/reader-input-contract.md`, khảo sát `docs/research/forms-newsletter-comments.md`.
Chỗ khảo sát khác đề bài thì theo đề bài; chỗ đề bài khác hợp đồng thì theo hợp đồng.

## Nháp 1 · Bảng, trạng thái, không foreign key — agent quyết

Quyết định:

- Sáu bảng: `newsletter_lists`, `newsletter_subscribers` (email unique, đã chuẩn hoá),
  `newsletter_subscriptions` (khoá chính `(subscriber_id, list_id)`), `newsletter_campaigns`
  (unique `(source, source_ref)`), `newsletter_deliveries` (unique `(campaign_id,
  subscriber_id)`, unique `unsubscribe_token_hash`), `newsletter_suppressions` (email unique).
  Cộng các bảng của kernel theo thứ tự của hợp đồng: events (+ `actor_kind`), jobs, throttle,
  antispam nonces, tokens. Không foreign key, kể cả trong module: service xoá dòng con cùng
  transaction.
- Trạng thái đúng hợp đồng §4: đăng ký `unconfirmed` / `confirmed` / `unsubscribed`; người đăng
  ký `active` / `blocked`; chiến dịch `draft` / `scheduled` / `sending` / `paused` / `sent` /
  `cancelled`. Lượt gửi `pending` / `sent` / `failed` / `skipped` (đề bài E). Lý do chặn:
  `hard_bounce`, `complaint`, `manual` (đề bài E) và thêm `soft_bounce` (soft bounce tới ngưỡng)
  để không ghi sai thành `hard_bounce`.
- Thêm cột ngoài bảng của đề bài: `subscriptions.hold_reason` (lý do `Quarantine`, như
  `hold_reason` của comments — hợp đồng §4), `subscribers.soft_bounces`,
  `campaigns.locale` (ngôn ngữ chân thư), `campaigns.run` và `prepared_at` (xem nháp 4),
  `deliveries.claimed_at`, `lists.new_post_locale` (xem nháp 7).
- Số đếm của chiến dịch **không lưu** trong `newsletter_campaigns` mà đếm từ
  `newsletter_deliveries` khi đọc (`GROUP BY status`, có index `(campaign_id, status, id)`):
  lưu bộ đếm thì mỗi lượt gửi thêm một lần ghi tranh nhau trên cùng một dòng (SQLite một người
  ghi), và dễ lệch.

Đã cân nhắc: bảng nhật ký đồng ý chỉ thêm và bảng `feedback_events` của khảo sát — đề bài E không
có; bằng chứng đồng ý nằm ở cột `consent` của đăng ký (hợp đồng §3), bounce ghi vào danh sách chặn.

## Nháp 2 · Token huỷ đăng ký: riêng cho từng mail, không phải từng đăng ký — agent quyết

Bối cảnh: đề bài E nói "token ngẫu nhiên riêng của từng đăng ký, lưu băm, không hết hạn (không
phụ thuộc khoá ký)". Nếu chỉ lưu bản băm thì máy chủ không có lại token gốc để đặt vào các mail
sau; muốn có thì phải lưu token gốc (trái "lưu băm") hoặc dẫn từ khoá (trái "không phụ thuộc
khoá ký"), hoặc sinh token mới mỗi lần gửi và xoá token cũ (link trong mail cũ chết — trái "không
hết hạn").

Quyết định:

- Mỗi mail chiến dịch có token ngẫu nhiên 32 byte riêng (`secret::random_token`), chỉ lưu
  SHA-256 ở `newsletter_deliveries.unsubscribe_token_hash`. Token trỏ tới người đăng ký và danh
  sách của chiến dịch → huỷ đúng danh sách của mail đó; trang lựa chọn đọc / sửa mọi danh sách
  của người đó. Không hết hạn, không phụ thuộc khoá — đúng ý đề bài, chỉ khác đơn vị (mail thay
  vì đăng ký). Đây cũng là cách của khảo sát (mẫu Listmonk).
- Vì token nằm ở lượt gửi: chiến dịch đã gửi không xoá được (`newsletter.campaign_not_deletable`);
  xoá dữ liệu của một người thì token trong mail cũ của họ hết tác dụng (dữ liệu đã xoá).
- Mail gửi thử dùng token ngẫu nhiên không lưu: link không làm gì.

## Nháp 3 · Huỷ một chạm nhận thân form-urlencoded — ngoại lệ của `ValidJson` — agent quyết

Quyết định: `POST /api/v1/newsletter/public/unsubscribe?token=…` nhận thân bất kỳ (thường là
`List-Unsubscribe=One-Click`, `application/x-www-form-urlencoded`, RFC 8058) và bỏ qua nó — token
nằm ở query của URL trong header `List-Unsubscribe`. Không dùng `ValidJson` vì nhà cung cấp hộp
thư không gửi JSON. Câu trả lời luôn `200 {"data": {"unsubscribed": true}}`, kể cả token lạ (không
lộ gì). Có hiệu lực ngay trong request. `GET` không đổi dữ liệu: trang lựa chọn đọc bằng `GET
/public/preferences?token=`, sửa bằng `PUT`.

Đã cân nhắc: `axum::Form` (từ chối 415 khi thiếu / sai content type — nhà cung cấp lạ sẽ hỏng
lặng lẽ); đặt token trong đường dẫn (`/unsubscribe/{token}`) — log của kernel ghi đường dẫn
nhưng không ghi query string, nên token để ở query.

## Nháp 4 · Gửi chiến dịch: hai loại job, nhận dòng bằng `UPDATE` có điều kiện, nhiều nhất một lần — agent quyết

Quyết định:

- `newsletter.prepare` chụp người nhận: keyset theo `subscriber_id` trên các đăng ký `confirmed`,
  500 dòng mỗi transaction, chèn `ON CONFLICT DO NOTHING`; chạy quá 60 giây thì đưa job tiếp theo
  (khoá `newsletter.prepare:<id>:<run>:<sau-người>`) vào hàng đợi. Xong thì ghi `prepared_at` và
  đưa lô 0 vào hàng đợi.
- `newsletter.send` gửi một lô `AUTOXIDE_NEWSLETTER_BATCH_SIZE` (mặc định 100, tối đa 1000) dòng
  `pending`: kiểm lại ngay trước khi gửi (chặn, khoá, đã huỷ, giới hạn quảng cáo → `skipped` kèm
  lý do), rồi mỗi dòng: `UPDATE … SET unsubscribe_token_hash = ?, claimed_at = ? WHERE id = ? AND
  status = 'pending' AND unsubscribe_token_hash IS NULL` — đúng một worker thắng trên cả hai
  database; đưa mail vào hàng đợi của `mail` với `dedupe_key = newsletter.delivery:<id>`; đánh
  `sent`. Lô sau có `run_at` = bây giờ + `cỡ lô × 60 / AUTOXIDE_NEWSLETTER_SEND_RATE_PER_MINUTE`
  giây, khoá `newsletter.send:<id>:<run>:<số lô>`. Một lô chỉ là vài trăm lần ghi database nên
  xong rất xa dưới 240 giây.
- `dedupe_key` có tiền tố `newsletter.delivery:` (đề bài: "= ID delivery") vì khoá chống trùng của
  module `mail` dùng chung mọi module, và để nhận ra mail của mình trong `mail.delivery_failed`.
- `sent` nghĩa là "đã giao cho hàng đợi mail"; mail bị máy chủ nhận từ chối hẳn sau đó thì event
  `mail.delivery_failed` đổi lượt gửi sang `failed` (nháp 6).
- Khe hở "worker chết sau khi xếp mail, trước khi đánh `sent`": dòng đã nhận quá 300 giây (thời
  hạn giữ job) mà vẫn `pending` thành `failed` / `interrupted` — **nhiều nhất một lần**, không gửi
  lại vì token trong mail có thể đã đi mà bản băm đã bị ghi đè. Ghi rõ trong hướng dẫn.
- Gửi / hẹn giờ / tiếp tục tăng `campaigns.run`; job mang `run`, thấy khác số (hay chiến dịch
  không còn `sending`) thì thôi. Tạm dừng có hiệu lực sau lô đang chạy; tiếp tục gửi đúng phần
  `pending` còn lại; huỷ đổi mọi dòng `pending` chưa ai nhận thành `skipped` / `cancelled`.
- Hẹn giờ là job `prepare` có `run_at`; tới giờ job đổi `scheduled` → `sending`.

Đã cân nhắc: trạng thái `sending` cho từng lượt gửi (khảo sát) — đề bài chỉ có bốn trạng thái;
"đã nhận" thể hiện bằng `unsubscribe_token_hash` khác `NULL`. Một job cho mỗi người nhận (Keila)
— quá nhiều dòng job cho danh sách lớn.

## Nháp 5 · Đăng ký công khai: một câu trả lời, giữ lại thì buộc xác nhận, giới hạn mail xác nhận — agent quyết

Quyết định:

- `POST /public/subscribe` luôn `202 {"data": {"accepted": true}}` (mới, đang chờ, đã xác nhận,
  bị khoá, bị chặn, bẫy, vượt giới hạn, gửi lại cùng token). Lỗi chỉ cho dữ liệu sai (422 — kể cả
  `key` không phải danh sách công khai), token / đề hỏng (400 của `SpamGuard`), thiếu cấu hình
  (503 `newsletter.confirm_url_missing`).
- `Verdict::Quarantine` (gửi quá nhanh…) không bị bỏ: đăng ký bị **buộc xác nhận qua mail** kể cả ở
  danh sách không bật double opt-in, lý do lưu ở `hold_reason`. Người thật bấm link là xong; bot
  không có hộp thư.
- Người đã có: tên và ngôn ngữ chỉ ghi khi tạo mới — người lạ không sửa được tên của người khác.
- Mail xác nhận không chứa chữ người đăng ký gõ (kể cả tên): chỉ tên danh sách (quản trị viên đặt)
  và link. Tối đa `AUTOXIDE_NEWSLETTER_CONFIRM_MAILS_PER_DAY` (3) mail xác nhận tới một địa chỉ
  mỗi 24 giờ (`Throttle`, đếm trước transaction vì bộ đếm ghi bằng kết nối riêng).
- Token xác nhận: `OneTimeTokens` mục đích `confirm`, `subject` = ID người đăng ký, `data` = các
  ID danh sách; hạn `AUTOXIDE_NEWSLETTER_CONFIRM_HOURS` (168). Xác nhận bằng `POST
  /public/confirm` từ trang của website; xác nhận không đăng ký lại danh sách người đó đã huỷ
  sau khi xin.
- Nonce của token form lưu ID người đăng ký làm kết quả; `Claim::Replay` → rollback, trả cùng câu.

## Nháp 6 · Bounce và danh sách chặn — agent quyết

Quyết định:

- Nghe `autoxide_kernel::mail::DELIVERY_FAILED` **bằng tên**. Lỗi vĩnh viễn của mail có
  `dedupe_key` của module (`newsletter.delivery:` → lượt gửi thành `failed` / `hard_bounce`), hay
  gửi tới một địa chỉ đang là người đăng ký (vd mail xác nhận tới hộp thư không có), coi là hard
  bounce → chặn. Giao lại cùng event không đổi gì thêm.
- `POST /api/v1/newsletter/bounces` `{email, type: hard|soft|complaint, reason}` cần
  `newsletter.manage` (đề bài chỉ có bốn permission; không thêm permission riêng cho bounce).
  `hard` / `complaint` chặn ngay; `soft` cộng `soft_bounces`, tới
  `AUTOXIDE_NEWSLETTER_SOFT_BOUNCE_LIMIT` (3) thì chặn với lý do `soft_bounce`. Bỏ chặn thì đếm lại
  từ 0.
- Danh sách chặn theo email (đề bài E), không theo hash như khảo sát.

## Nháp 7 · Chiến dịch cho bài mới — agent quyết

Quyết định:

- Nghe `content.entry_published` bằng tên, không phụ thuộc crate `content`. Kernel báo lỗi khởi
  động nếu đăng ký nhận event không module nào khai, nên module chỉ đăng ký khi
  `context.events().contains(…)` (tương tự cho `mail.delivery_failed`) — chương trình không có
  `content` vẫn chạy.
- Chỉ bài viết (`published.kind = post`), đang công khai (`ResourceDirectory`, D-117); trang
  (`page`) không sinh bản tin. Tiêu đề, tóm tắt, đường dẫn lấy qua `resolve("content.entry", id)`.
- `source = "content.entry"`, `source_ref = "<entry_id>:<list_id>"`, unique, chèn `ON CONFLICT DO
  NOTHING`: một bài sinh nhiều nhất một chiến dịch cho mỗi danh sách, mãi mãi — event đăng lại
  hay giao lại không sinh gì mới.
- Thêm `lists.new_post_locale` (`en` / `vi` / `null`): không có thì một bài và bản dịch của nó sinh
  hai bản tin cho cùng danh sách.
- Link tới bài cần địa chỉ website: thêm cấu hình `AUTOXIDE_NEWSLETTER_SITE_URL`
  (`Resource.url` chỉ là đường dẫn). Chế độ `send` mà thiếu địa chỉ website, URL công khai hay
  trang lựa chọn thì chỉ tạo nháp, ghi cảnh báo — không để job lỗi lặp lại.

Đã cân nhắc: `AUTOXIDE_NEWSLETTER_AUTO_POST_MAX_AGE_HOURS` của khảo sát (bỏ qua bài cũ khi nhập
hàng loạt) — đề bài không có; chế độ mặc định `off` và `draft` đủ an toàn. Ghi trong hướng dẫn.

## Nháp 8 · Thư quảng cáo — agent quyết

Quyết định: cờ `advertising` theo chiến dịch (đề bài E). Tiêu đề thêm
`AUTOXIDE_NEWSLETTER_ADVERTISING_LABEL` (mặc định `[QC]`, `none` để bỏ). Mỗi địa chỉ nhận tối đa
`AUTOXIDE_NEWSLETTER_ADVERTISING_MAX_PER_DAY` (mặc định 3, `0` là không giới hạn) mail quảng cáo
trong 24 giờ, đếm từ lượt gửi `sent` của các chiến dịch quảng cáo; vượt thì lượt gửi `skipped` /
`advertising_limit` (không dời sang hôm sau — đơn giản, dễ đoán). Chiến dịch quảng cáo cần
`AUTOXIDE_NEWSLETTER_POSTAL_ADDRESS` (503 `newsletter.postal_address_missing` khi gửi / hẹn giờ /
gửi thử). Mọi giá trị là cấu hình, chưa đối chiếu Nghị định 91/2020 bản gốc (D-106 mục 4).

## Nháp 9 · Permission, `If-Match`, đường dẫn, MCP — agent quyết

Quyết định:

- Bốn permission theo đề bài E: `newsletter.read`; `newsletter.manage` (danh sách, người đăng ký,
  danh sách chặn, báo bounce, **soạn chiến dịch nháp**); `newsletter.send` (gửi, hẹn giờ, tạm
  dừng, tiếp tục, huỷ, gửi thử — gửi thử tới địa chỉ bất kỳ nên cần quyền gửi);
  `newsletter.export` (xuất / xoá theo email).
- `If-Match` bắt buộc: `PATCH` / `DELETE` danh sách và chiến dịch, `PATCH` người đăng ký, `send`,
  `schedule` (gửi đúng bản đã duyệt). Tuỳ chọn: `pause`, `resume`, `cancel`. `DELETE` người đăng
  ký: không cần (xoá là xoá).
- Đường dẫn dưới `/api/v1/newsletter`: `/lists`, `/lists/{id}`, `/subscribers`,
  `/subscribers/import`, `/subscribers/{id}`, `/campaigns`, `/campaigns/{id}`,
  `/campaigns/{id}/{send,schedule,pause,resume,cancel,test}`, `/suppressions`,
  `/suppressions/{id}`, `/bounces`, `/data?email=`; công khai `/public/lists`,
  `/public/subscribe`, `/public/confirm`, `/public/unsubscribe`, `/public/preferences`.
- Nhập người đăng ký: `POST /subscribers/import` (1–1000 người, 100 người mỗi transaction), luôn ở
  `confirmed` và **bắt buộc** `consent_source`; không đăng ký lại người đã huỷ ở danh sách đó.
  Quản trị viên không đăng ký thẳng được ai không có nguồn đồng ý (`PATCH` chỉ huỷ được).
- Tool MCP: `newsletter.list_lists`, `newsletter.find_subscribers`, `newsletter.get_campaign`
  (tiến độ), `newsletter.create_campaign` (nháp, `newsletter.manage`),
  `newsletter.send_campaign`, `newsletter.schedule_campaign` (`newsletter.send`, nhận `version`
  như `If-Match`).
- Tên schema OpenAPI trùng với module khác (`Locale`, `Localized`, `DeliveryResponse`,
  `EmailDataResponse`, `ErasedResponse`, `ScheduleRequest`) được đổi bằng `#[schema(as =
  Newsletter…)]`: kernel ghép component theo tên và bản sau ghi đè bản trước **không báo lỗi**.

## Nháp 10 · Dữ liệu cá nhân — agent quyết

Quyết định:

- Người đăng ký **không gắn với tài khoản**: đăng ký công khai chỉ có email, không có `user_id`.
  Mọi yêu cầu của chủ dữ liệu đi theo email: `GET` / `DELETE /api/v1/newsletter/data?email=`
  (`newsletter.export`). `PersonalData` và `USER_DELETED` chỉ áp cho `created_by` của danh sách
  và chiến dịch (như `forms`).
- Xoá theo email xoá người đăng ký, đăng ký (kèm bằng chứng đồng ý), lượt gửi, token xác nhận —
  nhưng **giữ dòng của danh sách chặn** nếu có: nghĩa vụ không gửi lại cho người đã bounce / khiếu
  nại / bị chặn tay cần giữ địa chỉ. Đây là ngoại lệ có lý do của quyền xoá; người cài xoá dòng đó
  bằng `DELETE /suppressions/{id}` nếu người đó yêu cầu. Bản xuất có dòng chặn.
- IP chỉ lưu dạng `ip_hash::fingerprint` bên trong bằng chứng đồng ý (giữ cùng bằng chứng, như
  D-121); không có cột IP riêng nên không cần cấu hình thời hạn xoá IP.
- Event không mang email; webhook nhận ID và `key` danh sách.
- Lượt gửi giữ mãi (link huỷ trong mail không hết hạn); job hằng ngày chỉ xoá đăng ký chưa xác
  nhận quá `AUTOXIDE_NEWSLETTER_UNCONFIRMED_DAYS` (30) và người không còn gì.

## Nháp 11 · Cấu hình — agent quyết

Quyết định: `AUTOXIDE_NEWSLETTER_CONFIRM_URL`, `…_PREFERENCES_URL` (có `{token}`, như
`AUTOXIDE_AUTH_*_URL`), `…_SITE_URL`, `…_CONFIRM_HOURS` (168), `…_CONFIRM_MAILS_PER_DAY` (3),
`…_UNCONFIRMED_DAYS` (30), `…_SUBSCRIBE_PER_IP_PER_HOUR` (10), `…_SUBSCRIBE_PER_EMAIL_PER_HOUR`
(5), `…_SUBSCRIBE_PER_USER_PER_HOUR` (1000), `…_SEND_RATE_PER_MINUTE` (300), `…_BATCH_SIZE` (100),
`…_SOFT_BOUNCE_LIMIT` (3), `…_ADVERTISING_LABEL` (`[QC]`), `…_ADVERTISING_MAX_PER_DAY` (3),
`…_POSTAL_ADDRESS`. Không biến nào bắt buộc lúc khởi động; thiếu thì chỉ chức năng cần nó báo
503 rõ ràng: trang xác nhận (đăng ký vào danh sách double opt-in), `AUTOXIDE_HTTP_PUBLIC_URL` và
trang lựa chọn (gửi, hẹn giờ, tiếp tục, gửi thử), địa chỉ người gửi (thư quảng cáo).
