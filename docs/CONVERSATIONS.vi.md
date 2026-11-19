# Chat chăm sóc khách (module `conversations`)

Module `conversations` cho khách đã đăng nhập nhắn tin với cửa hàng — hay với gian hàng của người bán trên sàn — về một
sản phẩm, một đơn hay việc khác. Nhân viên trả lời từ hộp thư chung. Mỗi cuộc trò chuyện thuộc một nút của cây tổ chức,
nên chuỗi và sàn chia việc theo nhánh như mọi bản ghi khác.

English: [`CONVERSATIONS.md`](CONVERSATIONS.md).

Gọi điện (gọi cửa hàng từ trình duyệt, "gọi lại cho tôi") sẽ dựng trên module này; cần tài khoản nhà cung cấp thoại và
chưa có trong module.

## Permission

| Permission              | Cho phép                                                                                   |
| ----------------------- | ------------------------------------------------------------------------------------------ |
| `conversations.read`    | Xem cuộc trò chuyện của các nút trong phạm vi và nhận event của chúng theo thời gian thực  |
| `conversations.reply`   | Trả lời, nhận cuộc, đóng, bản nháp AI, chuyển thành phiếu hỗ trợ                           |
| `conversations.manage`  | Giao cho đồng nghiệp, chặn khách, xử lý báo vi phạm                                        |

Với độ sâu `own`, nhân viên thấy cuộc được giao cho mình. Khách không cần permission: tài khoản đã đăng nhập dùng
`/api/v1/conversations/me/threads`.

## Cuộc trò chuyện và tin nhắn

- `POST /me/threads` bắt đầu cuộc trò chuyện với một nút (`org_id`, mặc định: cửa hàng) về một bản ghi (`subject`,
  `<kind>:<id>`, vd `catalog.product:<id>`). Khách đã có cuộc đang mở với cùng nút về cùng bản ghi thì tin vào cuộc đó.
- Mỗi tin mang `client_id` do ứng dụng tự đặt. Gửi lại cùng `client_id` (retry sau khi mất mạng) trả tin đầu tiên với mã
  200, không bao giờ thêm tin thứ hai.
- Tin là chữ thuần và tối đa 5 ảnh của thư viện media. Không sửa được; người viết thu hồi được trong 2 phút (bỏ chữ và
  ảnh, giữ chỗ).
- Nhân viên xem hộp thư ở `GET /threads` (lọc `status`, `assignee=me|none|<id>`, `unread`, `needs_agent`, `org_id`,
  `subject`), đánh dấu đã đọc (`POST /threads/{id}/read`; phía kia thấy con trỏ đọc), nhận hay giao cuộc
  (`POST /threads/{id}/assign`) và đóng cuộc. Cuộc đã đóng không nhận tin nữa; tin sau của khách mở cuộc mới.

## Thời gian thực và thông báo

Mở `GET /api/v1/kernel/events/stream?names=conversations.*` (máy chủ phải đặt `AUTOXIDE_EVENTS_STREAM=true`). Khách và
nhân viên được giao nhận event của cuộc của mình mà không cần permission; nhân viên có `conversations.read` ở cả khách
nhận tất cả. Event không mang nội dung tin: khi có `conversations.message_created`, đọc tin qua REST.
`POST …/typing` phát `conversations.typing`.

Tin còn chưa đọc sau `AUTOXIDE_CONVERSATIONS_NOTIFY_AFTER_MINUTES` (mặc định 5) phút sinh một thông báo cho khách, hay
cho nhân viên được giao — mỗi cuộc mỗi khung một thông báo, dù có bao nhiêu tin.

## Trợ giúp nhân viên

- Trả lời tự động: mỗi tin của khách được đưa cho phần trả lời tự động của module `support` (cùng câu trả lời đã duyệt,
  cùng luật, bật theo nút ở đó, tắt mặc định). Tin tự động có `auto_reply: true`; hiện nút "Gặp nhân viên" gọi
  `POST /me/threads/{id}/handoff`.
- `POST /threads/{id}/suggest-reply` xin AI một bản nháp (`draft_reply`); không gì tự gửi.
- `POST /threads/{id}/escalate` mở phiếu hỗ trợ kèm các tin gần nhất và liên kết về cuộc trò chuyện; gọi lại trả cùng
  phiếu.

## Chống lạm dụng và pháp luật

- `AUTOXIDE_CONVERSATIONS_MESSAGES_PER_MINUTE` (mặc định 20) mỗi người; quá thì 429.
- `PUT /threads/{id}/block` chặn khách viết tới nút đó; khách và nhân viên báo vi phạm bằng `POST …/reports`, xử lý ở
  `GET /reports`.
- Cài đặt theo nút: `conversations.require_verified_account` (khách phải có số điện thoại đã xác thực, Nghị định
  147/2024), `conversations.mask_contacts` và `conversations.mask_terms` (che số điện thoại, email, link và từ khai thêm —
  sàn dùng để giữ giao dịch trong sàn).
- Dữ liệu cá nhân (Luật 91/2025/QH15): cuộc trò chuyện bị xoá sau `AUTOXIDE_CONVERSATIONS_RETENTION_DAYS` (mặc định 365)
  ngày kể từ tin cuối; xoá tài khoản thì xoá nội dung tin của khách đó và bỏ liên kết cuộc trò chuyện; bản xuất dữ liệu
  cá nhân có các cuộc này.
