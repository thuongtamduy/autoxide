# Biểu mẫu (forms)

*English: [FORMS.md](FORMS.md)*

Module `forms` giữ định nghĩa biểu mẫu, nhận bài gửi từ người dùng website (form liên hệ, đăng
ký tư vấn, khảo sát đơn giản), kiểm mọi câu trả lời ở máy chủ, lưu vào hộp thư quản trị, gửi mail
báo người phụ trách, và xuất hay xoá dữ liệu. Module chỉ có API: website hiển thị form và gửi
bài; màn hình quản trị là dự án riêng. Đường dẫn nằm dưới `/api/v1/forms`. Hợp đồng đầy đủ ở
[`openapi.json`](openapi.json).

Không cần tài khoản dịch vụ bên thứ ba: spam bị chặn bằng lớp chống spam dùng chung của core (ô
bẫy ẩn, token form có chữ ký, giới hạn số lần, dò bài trùng; tuỳ chọn thêm đề proof-of-work tự
chạy trên máy chủ).

## Permission

| Permission     | Cho phép                                                                       |
| -------------- | ------------------------------------------------------------------------------ |
| `forms.read`   | Đọc form và bài gửi (dữ liệu cá nhân)                                          |
| `forms.manage` | Tạo, sửa, mở, đóng, xoá form                                                   |
| `forms.review` | Đánh dấu bài đã đọc, lưu trữ, spam hay không phải spam; xoá một bài            |
| `forms.export` | Xuất CSV; xuất và xoá mọi bài gửi của một địa chỉ email                        |

Người điền form không cần permission nào: mọi thứ dưới `/api/v1/forms/public/` đều mở.

## Định nghĩa form

```bash
curl -X POST http://localhost:8080/api/v1/forms \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -H "Idempotency-Key: $(uuidgen)" \
  -d '{"key": "contact", "name": "Contact us",
       "fields": [
         {"key": "name", "type": "text", "label": {"en": "Name", "vi": "Tên"}, "required": true},
         {"key": "email", "type": "email", "label": {"en": "Email"}, "required": true},
         {"key": "message", "type": "textarea", "label": {"en": "Message", "vi": "Lời nhắn"}},
         {"key": "agree", "type": "consent", "required": true,
          "label": {"en": "You may contact me about my request.", "vi": "Tôi đồng ý được liên hệ."}}
       ],
       "allowed_origins": ["https://www.example.com"],
       "notify_emails": ["sales@example.com"],
       "success_message": {"en": "Thank you!", "vi": "Cảm ơn bạn!"}}'
```

- **Kiểu trường** (danh sách đóng): `text`, `email`, `phone`, `textarea`, `number` (`min`,
  `max`), `select`, `radio`, `checkbox` (chọn nhiều), `date` (`YYYY-MM-DD`), `hidden` (trang tự
  điền, vd mã chiến dịch), `consent` (ô đồng ý, không được tích sẵn; nhãn chính là chữ hiện ra).
  `text`, `textarea`, `hidden` nhận `min_length` / `max_length`. Nhãn, gợi ý, placeholder, nhãn
  lựa chọn viết `{"en": …, "vi": …}`.
- Mọi lỗi của định nghĩa được báo một lần (`fields[2].options`…).
- **Phiên bản.** Đổi `fields` thì tạo `version` mới. Mỗi bài gửi giữ phiên bản lúc gửi và luôn
  hiện với nhãn của đúng phiên bản đó (`answers[]`), nên bài cũ vẫn đọc được sau khi form đổi.
- `PATCH` và `DELETE` form cần `If-Match` (`ETag` đã đọc). Đóng form (`{"status": "closed"}`)
  giữ bài gửi; xoá form thì xoá luôn bài gửi.
- `notify_emails` nhận mail cho mỗi bài mới, `Reply-To` là địa chỉ người gửi. Endpoint công khai
  không bao giờ trả các địa chỉ này, và người nhận không bao giờ lấy từ dữ liệu khách gửi lên.

## Website

1. `GET /api/v1/forms/public/contact?locale=vi` — các trường với chữ theo một ngôn ngữ,
   `token_scope` và lời cảm ơn. Không kèm thiết lập quản trị nào.
2. Mỗi lần hiện form (không bao giờ đưa vào HTML được cache):
   `GET /api/v1/antispam/token?scope=forms:contact` — lấy token form. Hiện thêm một ô bẫy ẩn mà
   người thật để trống.
3. `POST /api/v1/forms/public/contact/submissions`:

```json
{"data": {"name": "Ada", "email": "ada@example.com", "message": "Hello", "agree": true},
 "locale": "vi", "page_url": "https://www.example.com/contact",
 "form_token": "…", "trap": ""}
```

- Câu trả lời là `201 {"data": {"id": …, "message": "Cảm ơn bạn!"}}` — **giống hệt** dù bài được
  nhận, bị giữ ở spam (gửi quá nhanh, trùng) hay bị bỏ (dính bẫy, quá nhiều lần từ một mạng hay
  một địa chỉ): bot không học được gì.
- Câu trả lời sai: `422`, mọi lỗi trong `errors` (`data.email` → `email`, khoá lạ → `unknown`).
  Thiếu token hay token hết hạn: `400 antispam.token_*` — tải lại form. Form đã đóng: `409
  forms.closed`. Trang có `Origin` không nằm trong `allowed_origins`: `403
  forms.origin_not_allowed` (danh sách rỗng cho mọi site).
- Gửi lại cùng `form_token` (bấm hai lần, mạng chập chờn) trả lại `id` lần đầu, không lưu thêm.
- Máy chủ của website có thể gửi bằng API key của nó: không cần token, giới hạn theo
  `AUTOXIDE_FORMS_SUBMISSIONS_PER_USER_PER_HOUR`; dùng header `Idempotency-Key`.

## Hộp thư

- `GET /api/v1/forms/{id}/submissions?status=&q=` — mới trước, không có `spam` trừ khi hỏi;
  `q` tìm trong câu trả lời, không phân biệt hoa thường và dấu.
- `PATCH /api/v1/forms/submissions/{id}` `{"status": "read" | "archived" | "spam" | "new"}` kèm
  `If-Match`. Chuyển bài ra khỏi `spam` thì người phụ trách nhận mail như bài mới.
- `GET /api/v1/forms/{id}/submissions/export?format=csv` — CSV cho bảng tính. Ô bắt đầu bằng `=`,
  `+`, `-`, `@`, tab hay CR được thêm `'` ở đầu để không bao giờ chạy như công thức.

## Dữ liệu cá nhân

- `GET /api/v1/forms/data?email=…` liệt kê, `DELETE /api/v1/forms/data?email=…` xoá hẳn mọi bài
  gửi bằng một địa chỉ, ở mọi form (permission `forms.export`) — cho yêu cầu của chính người
  có dữ liệu.
- Không lưu địa chỉ IP: chỉ lưu bản băm có khoá của khối mạng, xoá sau
  `AUTOXIDE_FORMS_IP_RETENTION_DAYS`.
- Ô đồng ý được tích thì lưu đúng chữ đã hiện, phiên bản form, thời điểm và nguồn.
- Event (và vì vậy webhook) chỉ mang ID và key của form, không mang câu trả lời hay địa chỉ;
  hệ thống khác đọc chi tiết qua API có permission.
- Tài khoản bị xoá: form người đó tạo vẫn giữ; `created_by` thành `null`.

## Pháp lý

Thời hạn giữ dữ liệu là cấu hình, không cài cứng trong code. Form liên hệ có cần xin đồng ý không,
được giữ bài gửi bao lâu, phải trả lời yêu cầu về dữ liệu trong bao lâu tuỳ luật áp dụng cho bạn
(vd Luật Bảo vệ dữ liệu cá nhân 2025 và Nghị định 356/2025 của Việt Nam): **hãy tự kiểm văn bản
gốc cho trường hợp của mình** rồi đặt `AUTOXIDE_FORMS_RETENTION_DAYS` (hoặc `retention_days` của
từng form) cho phù hợp.

## AI agent (MCP)

Các tool, cùng permission và cùng cách kiểm với REST:

- `forms.list`, `forms.get` (`forms.read`); `forms.create` và `forms.update` (`forms.manage`).
  `forms.get` trả form kèm `etag`; truyền lại vào `etag` của `forms.update`, như `If-Match` (không
  phải `version` của form — số đó đếm số lần đổi danh sách trường). Đóng form bằng `forms.update`
  với `status: closed`.
- `forms.list_submissions`, `forms.get_submission` (`forms.read`) và
  `forms.set_submission_status` (`forms.review`; truyền `version` của bài gửi đã đọc).

Không có tool xoá form (xoá form xoá luôn mọi bài gửi, không hoàn tác được; dùng REST) và không có
tool gửi form.

## Cấu hình

| Biến                                            | Mặc định |
| ----------------------------------------------- | -------- |
| `AUTOXIDE_FORMS_MAX_SUBMISSION_BYTES`           | `65536`  |
| `AUTOXIDE_FORMS_MAX_FIELDS`                     | `50`     |
| `AUTOXIDE_FORMS_SUBMISSIONS_PER_IP_PER_HOUR`    | `10`     |
| `AUTOXIDE_FORMS_SUBMISSIONS_PER_EMAIL_PER_HOUR` | `5`      |
| `AUTOXIDE_FORMS_SUBMISSIONS_PER_USER_PER_HOUR`  | `1000`   |
| `AUTOXIDE_FORMS_RETENTION_DAYS`                 | `0` (giữ mãi) |
| `AUTOXIDE_FORMS_SPAM_RETENTION_DAYS`            | `30`     |
| `AUTOXIDE_FORMS_IP_RETENTION_DAYS`              | `90`     |
| `AUTOXIDE_FORMS_NOTIFY_LANGUAGE`                | `en`     |

Mỗi form có thể đặt riêng `ip_limit_per_hour`, `email_limit_per_hour` và `retention_days`. Lớp
chống spam (`AUTOXIDE_ANTISPAM_*`) dùng chung cho mọi module. Mọi biến được mô tả trong
[`.env.example`](../.env.example).
