# Hỗ trợ khách hàng (module `support`)

Module `support` nhận yêu cầu hỗ trợ của người mua — hỏi về đơn, sản phẩm, tài khoản, thanh toán — và để nhân viên
của cửa hàng trả lời tới khi xong. Mỗi yêu cầu là một **phiếu** gồm các tin nhắn qua lại. Phiếu thuộc một nút tổ chức
(cửa hàng, chi nhánh, gian hàng của người bán trên sàn), nên chuỗi và sàn chia việc theo nhánh như mọi dữ liệu khác.

English: [`SUPPORT.md`](SUPPORT.md).

Không làm: khiếu nại người tiêu dùng có hạn luật định nằm ở module `compliance` (phiếu được chuyển sang đó, không chép
lại), tranh chấp người mua – người bán ở `marketplace`, trả hàng ở `orders`. Chat trực tiếp là module làm sau.

## Permission

| Permission                  | Cho phép                                                                                       |
| --------------------------- | ---------------------------------------------------------------------------------------------- |
| `support.tickets.read`      | Xem phiếu của các nút trong phạm vi, cùng tin nhắn và ghi chú nội bộ                          |
| `support.tickets.reply`     | Trả lời, ghi chú nội bộ, mở phiếu thay khách, đổi trạng thái, ưu tiên, người được giao, liên kết |
| `support.tickets.manage`    | Ẩn tin nhắn, chuyển phiếu thành khiếu nại người tiêu dùng hay lên sàn                          |
| `support.settings.manage`   | Chính sách hỗ trợ, nhóm, quy tắc phân công và câu trả lời mẫu của các nút trong phạm vi        |
| `support.auto_reply.manage` | Duyệt câu trả lời mẫu cho trả lời tự động                                                      |

Mọi permission theo nút tổ chức: nhân viên của một chi nhánh thấy phiếu của chi nhánh đó và các chi nhánh dưới; người
bán trên sàn có quyền ở gian hàng mình chỉ thấy phiếu của gian hàng đó. Với độ sâu `own`, nhân viên thấy phiếu được giao
cho mình.

Người mua không cần permission: tài khoản nào đã đăng nhập cũng thấy phiếu của chính mình ở `/api/v1/support/me/tickets`.

## Phiếu

- **Từ website** — `POST /api/v1/support/public/tickets`. Đã đăng nhập: phiếu thuộc tài khoản. Khách vãng lai: cho
  `email` hoặc `phone` và các trường chống spam của form (`GET /api/v1/antispam/token?scope=support:tickets`). Câu trả
  lời có `access_token` một lần — khách giữ để xem (`GET /public/tickets/{id}?token=`) và trả lời
  (`POST /public/tickets/{id}/messages`) đúng phiếu đó. Một mã 6 chữ số được gửi tới email hay số điện thoại; khách xác
  nhận (`POST /public/tickets/{id}/verify`) trước khi viết tin thứ hai.
- **Nhân viên ghi hộ** — `POST /api/v1/support/tickets` với tài khoản, email hay số điện thoại của khách (gọi điện, tới
  cửa hàng).
- **Từ module khác** — qua trait `SupportIntake` (vd trang đơn hàng); cùng `reference` không bao giờ tạo hai phiếu.
- **Sau khi đăng ký** — `POST /api/v1/support/me/claim` gộp các phiếu vãng lai đã xác thực có số điện thoại mà tài khoản
  đã xác thực, hay có email / số của một phiếu vãng lai đã xác thực mà người mua gửi kèm token.

Trạng thái: `open` → `pending_agent` (người mua vừa viết) ⇄ `pending_customer` (nhân viên vừa trả lời) → `solved` →
`closed`. Người mua viết vào phiếu `solved` thì phiếu mở lại; `closed` là kết thúc (mở phiếu mới với `follow_up_of`).
Chỉ đi được các bước hợp lệ (`support.invalid_transition`).

Tin nhắn không bao giờ bị sửa. Tin gửi nhầm được ẩn kèm lý do (`POST /tickets/{id}/messages/{message_id}/hide`,
`support.tickets.manage`); người mua thấy có một tin đã bị ẩn. Ghi chú nội bộ (`internal: true`) chỉ nhân viên thấy và báo
cho đồng nghiệp có trong `mentions`. Người mua đã đăng nhập đính kèm tệp media chính họ tải lên (tối đa 5 tệp mỗi tin).

## Cài đặt theo nút

Mọi cài đặt là dữ liệu của một nút tổ chức và áp cho các nút bên dưới, trừ nút có cài đặt riêng.

- **Chính sách** — `PUT /api/v1/support/policies/{org_id}`: loại phiếu, số phút tới câu trả lời đầu và tới khi xử lý xong
  theo ưu tiên (`low`, `normal`, `high`, `urgent`, tính theo giờ làm việc), giờ làm và ngày nghỉ, độ lệch múi giờ, số
  ngày trước khi phiếu đã xử lý tự đóng, có nhận phiếu vãng lai không, email trả lời, trả lời tự động. Mặc định: thứ Hai
  – thứ Sáu 08:00–17:00 (UTC+7), trả lời đầu trong 8 giờ làm việc và xử lý xong trong 3 ngày làm việc cho `normal`.
- **Nhóm** — `POST /api/v1/support/teams`, thành viên bằng `PUT /teams/{id}/members/{user_id}` (`accepting` nhận phiếu
  mới; thành viên tự bật / tắt). Thành viên phải trả lời được phiếu của nút.
- **Quy tắc phân công** — `POST /api/v1/support/routing-rules` (`category`, `channel` → nhóm). Phiếu mới vào nhóm của
  quy tắc khớp đầu tiên (nút gần nhất trước) và chia vòng cho thành viên chờ lâu nhất còn `support.tickets.reply` ở nút
  của phiếu.
- **Câu trả lời mẫu** — `POST /api/v1/support/macros` với nội dung theo ngôn ngữ và biến `{customer_name}`,
  `{ticket_number}`, `{subject}`; điền cho một phiếu bằng `POST /tickets/{id}/macros/{macro_id}/render`.

## Hạn trả lời và hộp việc

Hạn trả lời đầu và hạn xử lý được tính khi phiếu mở và khi đổi ưu tiên. Khi phiếu chờ người mua, đồng hồ dừng; người mua
viết lại thì hạn xử lý lùi đúng thời gian làm việc đã chờ. Job `support.sla_check` (mỗi phút) đánh dấu phiếu trễ hạn
một lần (`breached`, event `support.ticket_breached`); `support.auto_close` đóng phiếu đã xử lý sau số ngày của chính
sách. Phiếu chờ nhân viên hiện trong hộp việc (`GET /api/v1/kernel/work`, nguồn `support.tickets`). `GET
/api/v1/support/reports/summary?from=&to=` đếm phiếu, số đã xử lý và trễ hạn, thời gian trung vị và mức hài lòng trung
bình.

## Liên kết, chuyển phiếu, mức hài lòng

- `POST /tickets/{id}/links {"target": "orders.order:<id>"}` gắn một bản ghi của module bất kỳ (phải tồn tại); trang đơn
  liệt kê phiếu liên quan bằng `GET /tickets?link=orders.order:<id>`.
- `POST /tickets/{id}/escalate {"to": "consumer_complaint"}` chuyển phiếu sang `compliance` thành khiếu nại người tiêu
  dùng có hạn luật định: phiếu giữ `case_id` và đóng với kết quả `escalated`; gọi lại không bao giờ mở vụ việc thứ hai.
  Người mua muốn khiếu nại thì gửi thẳng `POST /api/v1/compliance/public/complaints`. `{"to": "platform"}` chuyển
  phiếu của người bán lên nhân viên của sàn.
- Khi phiếu đã xử lý, người mua chấm mức hài lòng từ 1 tới 5 (`POST /me/tickets/{id}/satisfaction`, hay endpoint công
  khai kèm token).

## AI và email

- `POST /tickets/{id}/reply-suggestions` soạn bản nháp bằng module `ai` từ tin nhắn của phiếu (đã bỏ email, số điện
  thoại) và câu trả lời mẫu của nút. Đây chỉ là bản nháp: nhân viên sửa rồi gửi với `ai_assisted: true`, người mua thấy
  nhãn.
- Email trả lời (tắt mặc định; bật `inbound_email` trong chính sách): module nhận thư chuyển thư gửi tới
  `support+<khoá>@miền-của-bạn` cho trait `InboundMail`; khoá của phiếu lấy từ `InboundMail::reply_address_key`. Thư
  không phải của người mua, sai khoá hay vào phiếu đã đóng thì bỏ; thư không bao giờ mở phiếu mới. Phần trích dẫn thư cũ
  bị cắt.

## Trả lời tự động (tắt mặc định)

Bật `auto_reply` trong chính sách của nút. Khi người mua viết, AI chỉ **phân loại** câu hỏi; luật trong máy chủ quyết
định, và chỉ hai loại chữ tới được người mua:

1. **Câu trả lời mẫu đã duyệt**, nguyên văn và đã điền biến. Duyệt bằng
   `PUT /api/v1/support/macros/{id}/auto-reply {"enabled": true, "questions": {...}, "review_due_on": "YYYY-MM-DD"}`
   (`support.auto_reply.manage`); quá ngày rà lại thì thôi dùng, sửa nội dung thì mất dấu duyệt.
2. **Câu dựng từ dữ liệu chính xác** của module đã gắn `FactSource` (vd trạng thái đơn của chính người mua); không chắc
   người hỏi là chủ dữ liệu thì không trả lời.

Chỉ gửi khi phân loại đủ tin cậy (`auto_reply_min_confidence_bp`, mặc định 9000), không có cờ rủi ro, câu hỏi không có
từ khoá chặn (có sẵn: khiếu nại, hoàn tiền, trả hàng, huỷ đơn, lỗi thanh toán, dữ liệu cá nhân, từ pháp lý, đòi gặp người
— cộng `auto_reply_blocked_terms` của nút), và chưa quá `auto_reply_max_per_thread` (mặc định 2) lần tự động liên tiếp.
Còn lại nhân viên trả lời như thường, có sẵn bản nháp AI. Tin tự động có `auto_reply: true`: gắn nhãn "Trả lời tự động"
và hiện nút "Gặp nhân viên" (`POST /me/tickets/{id}/handoff`, hay endpoint công khai kèm token) — bấm là dừng tự động
trong phiếu đó. Người mua chấm câu trả lời (`…/auto-replies/{reply_id}/feedback`); câu trả lời mẫu bị báo sai
`AUTOXIDE_SUPPORT_AUTO_REPLY_FLAG_LIMIT` (3) lần trong 7 ngày thì thôi dùng. Thử một câu hỏi bằng
`POST /api/v1/support/auto-reply/preview` (không gửi gì) và xem nhật ký ở `GET /api/v1/support/auto-replies`.

## Cho AI agent (MCP)

Tool, cùng permission như REST: `support.list_tickets`, `support.get_ticket`, `support.reply_ticket`,
`support.add_internal_note`, `support.update_ticket`, `support.link_ticket`, `support.suggest_reply` (chỉ bản nháp),
`support.preview_auto_reply`.

## Cấu hình

| Biến                                      | Mặc định | Ý nghĩa                                                         |
| ----------------------------------------- | -------- | --------------------------------------------------------------- |
| `AUTOXIDE_SUPPORT_GUEST_PER_HOUR`         | 5        | Số phiếu vãng lai mỗi giờ theo địa chỉ mạng, email, số điện thoại |
| `AUTOXIDE_SUPPORT_MAX_BODY_CHARS`         | 10000    | Tin nhắn dài nhất                                               |
| `AUTOXIDE_SUPPORT_TOKEN_DAYS_AFTER_CLOSE` | 30       | Số ngày token phiếu của khách còn dùng được sau khi phiếu đóng  |
| `AUTOXIDE_SUPPORT_GUEST_URL`              | —        | Trang hiện phiếu cho khách vãng lai (`{id}`, `{token}`)         |
| `AUTOXIDE_SUPPORT_AUTO_REPLY_FLAG_LIMIT`  | 3        | Số lần bị báo sai trong 7 ngày thì thôi dùng câu trả lời tự động |

## Dữ liệu cá nhân

Xoá tài khoản thì phiếu của tài khoản được ẩn danh hoá (bỏ tên, liên lạc và chữ người mua viết) và giữ lại cho thống kê.
Phiếu và tin nhắn của người mua có trong bản xuất dữ liệu cá nhân. Căn cứ pháp lý cần đối chiếu văn bản gốc / người có
chuyên môn pháp lý: Luật Bảo vệ dữ liệu cá nhân 91/2025/QH15, Luật Bảo vệ quyền lợi người tiêu dùng 19/2023/QH15, Luật
Trí tuệ nhân tạo 134/2025/QH15 (gắn nhãn câu trả lời do AI viết).
