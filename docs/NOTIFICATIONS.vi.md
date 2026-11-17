# Thông báo

[English](NOTIFICATIONS.md)

Module `notifications` gửi thông báo tới người dùng qua **email, SMS, Zalo ZNS, web push, đẩy tới ứng
dụng di động và hộp thư trong ứng dụng**. Module khác khai loại thông báo mình gửi; người dùng chọn loại
nào mình nhận; tin quảng cáo theo sổ đồng ý và quy tắc pháp lý của module `compliance`. Nhân sự nền tảng
gửi được thông báo tới quản trị viên của mọi khách.

> Các ghi chú pháp lý dưới đây (đồng ý, khung giờ, nhãn quảng cáo) lấy từ NĐ 91/2020/NĐ-CP và Luật Bảo
> vệ dữ liệu cá nhân 91/2025/QH15, cần người có chuyên môn đối chiếu văn bản gốc trước khi chạy thật.

## Loại và nhóm

Mỗi thông báo thuộc một **loại** do module khai, vd `orders.order_shipped`. Một loại có:

| Trường | Nghĩa |
|---|---|
| nhóm | `security` (đăng nhập, mật khẩu: luôn gửi, không tắt được), `transactional` (do việc người dùng làm: bật trừ khi họ tắt), `marketing` (quảng cáo: cần đồng ý) |
| kênh | nơi có thể gửi: `email`, `sms`, `zns`, `web_push`, `in_app`, `mobile_push` |
| câu mặc định | tiêu đề, nội dung và bản ngắn (SMS, ZNS, đẩy) bằng tiếng Anh, có chỗ điền `{tên}`; bản dịch tiếng Việt nằm ở `locales/vi.json` của module khai |
| mục đích | chỉ tin quảng cáo: mục đích đồng ý cần kiểm (mặc định `marketing`) |
| tin mời | chỉ tin quảng cáo: lời mời đăng ký nhận, mỗi địa chỉ và kênh nhận tối đa một lần |

`GET /api/v1/notifications/kinds` liệt kê mọi loại (người đã đăng nhập), để giao diện dựng trang tuỳ chọn.

## Một thông báo được gửi thế nào

1. Module gọi `Notifier::notify` (từ handler event hay job, không bao giờ trong transaction ghi của
   mình) với loại, người nhận (một người dùng, một email, một số điện thoại, hay mọi quản trị viên của
   khách), giá trị điền vào câu, link (tuỳ chọn) và `dedupe_key` **bắt buộc**.
2. Module chọn kênh: các kênh của loại, bỏ kênh người dùng đã tắt (tin bảo mật bỏ qua tuỳ chọn), bỏ
   kênh không có địa chỉ (email của tài khoản, số điện thoại, đăng ký đẩy), bỏ kênh máy chủ không có bên
   gửi. Tin quảng cáo còn cần đồng ý trong sổ của `compliance`. Mỗi người nhận tối đa
   `AUTOXIDE_NOTIFICATIONS_PER_RECIPIENT_PER_HOUR` (20) tin giao dịch và quảng cáo mỗi giờ.
3. Mỗi kênh một tin trong nhật ký gửi kèm một job gửi, trong một transaction. Cùng `dedupe_key` lần nữa
   trả lại các tin đã có, không gửi thêm — gọi lại an toàn.
4. Job gửi đọc lại tin. Với tin quảng cáo, nó **kiểm lại đồng ý** ngay trước khi gửi (đã rút →
   `skipped` / `consent_withdrawn`) và khung giờ: ngoài khung thì tin `deferred` tới lần mở khung kế tiếp,
   và bị bỏ (`quiet_hours_expired`) nếu không gửi được trong 24 giờ. Rồi nó dựng câu (câu của khách, không
   có thì bản dịch, không có nữa thì tiếng Anh) và đưa cho bên gửi. Lỗi tạm thời được thử lại; lỗi vĩnh
   viễn đánh dấu tin `failed` kèm `error_code`.

Kênh bị bỏ qua có trong kết quả (`skipped`) và trong nhật ký gửi, kèm lý do: `no_consent`,
`consent_withdrawn`, `preference_off`, `no_address`, `channel_disabled`, `quiet_hours_expired`,
`invitation_used`, `throttled`.

## Tin quảng cáo

- Chỉ gửi tới người đã đồng ý cho mục đích và kênh đó (`PUT /api/v1/compliance/me/consents`, hay module
  ghi qua `ConsentLedger`).
- Khung giờ và nhãn là **quy tắc pháp lý có ngày hiệu lực** trong `compliance`, không nằm trong code:
  `notifications.send_windows` (SMS và ZNS 07:00–22:00 giờ Việt Nam, `utc_offset_minutes: 420`) và
  `notifications.marketing_labels` (`QC ` trước SMS, `[QC] ` trước tiêu đề email và thông báo đẩy). Nhân
  sự nền tảng nhập bản mới bằng `POST /api/v1/compliance/rules`.
- Mỗi email quảng cáo mang header `List-Unsubscribe` và chân thư có link tới trang
  `AUTOXIDE_NOTIFICATIONS_UNSUBSCRIBE_URL` (có `{token}`) của website. Trang đó gửi token tới
  `POST /api/v1/notifications/public/unsubscribe` (không cần đăng nhập): đồng ý bị rút, tin quảng cáo đang
  chờ bị huỷ. Không đặt biến này thì email quảng cáo không được gửi (`failed` /
  `unsubscribe_url_missing`). Link dùng được trong thời gian nhật ký gửi còn giữ địa chỉ
  (`AUTOXIDE_NOTIFICATIONS_RETENTION_DAYS`, 90 ngày).
- Đồng ý bị rút ở bất kỳ đâu (event `compliance.consent_withdrawn`): tin quảng cáo đang chờ và đang hoãn
  của người đó bị huỷ.

## Kênh và bên gửi

| Kênh | Bên gửi (trait của kernel) | Địa chỉ |
|---|---|---|
| `email` | `Mailer` (module `mail`) | email của tài khoản, hay email được nêu |
| `sms` | `SmsSender` | số điện thoại được nêu |
| `zns` | `ZnsSender` | số điện thoại được nêu; mã mẫu Zalo đã duyệt đặt theo khách bằng `PUT /templates/{kind}/zns/{locale}` (`zns_template_id`) |
| `web_push` | `WebPushSender` | các trình duyệt người dùng đã đăng ký (`POST /me/push-subscriptions`) |
| `mobile_push` | `MobilePushSender` | các thiết bị người dùng đã đăng ký (`POST /me/device-tokens`) |
| `in_app` | có sẵn | hộp thư của người dùng (`GET /me/inbox`) |

Bên gửi SMS, ZNS, web push và đẩy di động là module riêng cần khoá của nhà cung cấp; bản này chưa kèm
bên nào (test dùng bản giả). Không có bên gửi thì kênh bị bỏ qua (`channel_disabled`). Secret của đẩy
(khoá `auth` của trình duyệt, token thiết bị) được lưu đã mã hoá. Dịch vụ đẩy báo đăng ký hay token đã
mất thì nó bị xoá.

Đẩy di động: ứng dụng đăng ký token FCM / APNs bằng `POST /me/device-tokens`
(`{"platform": "android", "token": "…", "app_id": "com.example.shop"}`; đăng ký lại cùng token thì cập
nhật), xem các thiết bị của người dùng bằng `GET /me/device-tokens` (không bao giờ trả token) và gỡ một
thiết bị bằng `DELETE /me/device-tokens/{id}`. Đẩy di động theo cùng luật như web push: tuỳ chọn nhận theo
loại, tin quảng cáo cần đồng ý (kênh `push` trong sổ đồng ý) và trong khung giờ của
`notifications.send_windows` (khoá `push`). Token nhà cung cấp báo đã mất hay từ chối bị xoá ngay; token
lỗi 10 lần liên tiếp cũng bị xoá. Module nhà cung cấp cài trait `MobilePushSender` của kernel; bộ nối
Firebase Cloud Messaging và Apple Push Notification service cần khoá của nhà cung cấp, không có trong bản
này.

## Cho người dùng: hộp thư và tuỳ chọn

- `GET /me/inbox` (một trang, `unread=true` để chỉ lấy tin chưa đọc), `GET /me/inbox/unread-count`,
  `POST /me/inbox/{id}/read`, `POST /me/inbox/read-all`.
- `GET /me/preferences` liệt kê mọi loại với từng kênh bật hay tắt; `PUT /me/preferences` nhận
  `{"items": [{"target": "category:marketing", "channel": "email", "enabled": false}]}`. Đích là
  `kind:<khoá>` hay `category:<nhóm>`; cài theo loại thắng cài theo nhóm. Tắt thông báo bảo mật trả
  `notifications.preference_locked`.
- `GET /public/web-push-key` cho khoá dùng với `PushManager.subscribe`.

## Cho quản trị viên

| Endpoint | Permission |
|---|---|
| `GET /templates`, `PUT` / `DELETE /templates/{kind}/{channel}/{locale}` — thay câu mặc định (chỉ dùng chỗ điền `{tên}` của câu mặc định và `{link}`) | `notifications.templates.manage` |
| `POST /messages` — gửi thông báo | `notifications.send` (tin quảng cáo cần thêm `notifications.send_marketing`) |
| `GET /messages`, `GET /messages/{id}` — nhật ký gửi | `notifications.messages.read` |
| `POST /platform-notices` — thông báo tới quản trị viên mọi khách (nhân sự nền tảng) | `notifications.platform_notices.send` |

Thông báo của nền tảng chạy nền, mỗi lượt 50 khách, tới hộp thư và email của mọi quản trị viên còn hoạt
động; chạy lại không bao giờ gửi trùng. `category` là `transactional` (quản trị viên tắt được) hay
`security`. Module `plans` dùng cùng đường này để báo quản trị viên khách khi thuê bao bị tạm khoá hay
quá hạn, và mỗi tháng một lần khi dùng tới 80% một hạn mức.

Tool MCP: `notifications.list_kinds`, `notifications.send_notification`, `notifications.list_messages`,
`notifications.list_my_inbox`, `notifications.mark_inbox_read`, `notifications.list_my_devices`,
`notifications.get_template_schema`,
`notifications.preview_template`, `notifications.update_template_draft`.

## Mẫu do khách tự soạn

Khách tự soạn mẫu cho từng loại, kênh và ngôn ngữ, theo cú pháp của bộ dựng mẫu dùng chung: `{{ code }}`,
`{% if link %}…{% endif %}`, `{% for line in lines %}…{% endfor %}`, bộ lọc `money`, `number`, `date`. Mẫu là
dữ liệu không tin cậy: không đọc file, không gọi mạng, giới hạn độ dài, số bước chạy và kết quả
(`template.limit_exceeded`), và mọi giá trị chèn vào HTML đều được thoát.

| Endpoint (dưới `/templates/{kind}/{channel}/{locale}`) | Permission |
|---|---|
| `GET /kinds/{kind}/schema` — biến mẫu được dùng (đánh dấu biến bắt buộc) và dữ liệu mẫu | đăng nhập |
| `GET` / `PUT` / `DELETE …/draft` — bản nháp; lưu thì kiểm cú pháp, biến lạ và biến bắt buộc, báo mọi lỗi một lần kèm dòng và cột | xem: `notifications.templates.read`; ghi: `notifications.templates.manage` |
| `POST …/publish` — bản nháp thành phiên bản mới (không đổi được nữa) và được dùng từ tin kế tiếp | `notifications.templates.manage` |
| `GET …/versions` — 20 phiên bản gần nhất, `live` đánh dấu bản đang dùng; `POST …/rollback {"version": 1}` dùng lại bản cũ; `DELETE …/published` thôi dùng mẫu | xem / `notifications.templates.manage` |
| `POST …/preview {"source": "draft", "data": {…}}` — dựng bằng dữ liệu mẫu hay giá trị của bạn (chỉ biến của loại); không gửi gì | `notifications.templates.read` |
| `POST …/test-send` — chỉ gửi tới chính bạn: email đã xác minh (`email`) hay hộp thư của bạn (`in_app`); 10 lần mỗi giờ | `notifications.templates.manage` |
| `GET` / `PUT /branding` — tên, logo (https), màu, chân thư của mail HTML | xem / `notifications.branding.manage` |

Trường: `subject` (chủ đề email, tiêu đề đẩy và hộp thư), `body_text` (chữ thuần: phần chữ của email, hộp
thư), `body_html` (chỉ email; bỏ trống → sinh từ `body_text`), `short` (SMS, ZNS, đẩy), `zns_template_id`.

Mọi email có phần HTML trong khung MJML có thương hiệu (dựng bằng `mrml`, chạy đúng trên Outlook, Gmail và
điện thoại) và phần chữ thuần. Chân thư có chân thư của thương hiệu cùng tên và địa chỉ chủ sở hữu (hồ sơ
pháp lý của `compliance`). Tin quảng cáo luôn có nhãn và chân thư huỷ nhận; mẫu không gỡ được. Câu chữ dùng
khi gửi: mẫu đã xuất bản đang dùng, không có thì câu thay thế (`PUT /templates/{kind}/{channel}/{locale}`),
không có nữa thì câu mặc định theo ngôn ngữ của người nhận. Mẫu đã xuất bản không dựng được với dữ liệu thật
thì dùng câu mặc định; tin vẫn gửi và ghi `error_code = template_fallback`.

Module khai biến của loại bằng `.variables(&[Variable::required("link", "…")])` và dữ liệu mẫu bằng
`.sample(r#"{"link": "https://…"}"#)`; không khai thì được dùng chỗ điền `{tên}` của câu mặc định và
`link`.

## Khai một loại trong module

Đăng ký module **sau** `notifications` trong `crates/app/src/modules.rs`, rồi:

```rust
use autoxide_notifications::api::{Category, Channel, Notification, NotificationKind, NotificationTemplate, Notifier, Recipient};

const SHIPPED: NotificationKind = NotificationKind::new(
    Category::Transactional,
    &[Channel::Email, Channel::InApp],
    NotificationTemplate::new(
        "orders.order_shipped",
        "Order {code} is on its way",
        "Your order {code} was handed to the carrier.",
        "Order {code} shipped",
    ),
);

// Trong Module::start:
let notifier = context.require::<dyn Notifier>().ok();
if let Some(notifier) = &notifier {
    notifier.declare(SHIPPED)?;
}

// Trong handler event hay job:
let notification = Notification::new(SHIPPED.key(), Recipient::User(user_id), json!({"code": code}), event.id.to_string())
    .link(format!("/orders/{id}"));
notifier.notify(tenant_id, notification).await?;
```

Thêm `orders.order_shipped.title`, `.body` và `.short` vào `locales/vi.json` của module (`cargo xtask
check` đòi đủ). Tin quảng cáo: `Category::Marketing`, có thể thêm `.purpose("…")`; lời mời đăng ký nhận:
`.invitation()`.

## Cấu hình

| Biến | Mặc định | Nghĩa |
|---|---|---|
| `AUTOXIDE_NOTIFICATIONS_RETENTION_DAYS` | 90 | số ngày giữ địa chỉ và giá trị của tin trong nhật ký gửi |
| `AUTOXIDE_NOTIFICATIONS_INBOX_RETENTION_DAYS` | 180 | số ngày một thông báo nằm trong hộp thư |
| `AUTOXIDE_NOTIFICATIONS_UNSUBSCRIBE_URL` | — | trang của bạn để rút đồng ý, có `{token}` |
| `AUTOXIDE_NOTIFICATIONS_PER_RECIPIENT_PER_HOUR` | 20 | số tin giao dịch và quảng cáo mỗi người nhận mỗi giờ |

Một job hằng ngày (`notifications.purge`) áp các hạn lưu này. Hộp thư, tuỳ chọn, thiết bị và nhật ký gửi của
một người nằm trong bản xuất dữ liệu cá nhân của họ; tài khoản bị xoá thì chúng bị xoá (nhật ký gửi giữ dòng nhưng
bỏ địa chỉ và giá trị).
