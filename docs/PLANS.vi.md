# Gói, thuê bao và hạn mức

[English](PLANS.md)

Module `plans` cho phép bán Autoxide theo gói mà không sửa code. Một **gói** là dữ liệu: khách được
dùng module nào, tính năng nào bật, được dùng bao nhiêu (hạn mức), và giá. Khách có gói qua **thuê
bao**; kernel cưỡng chế ở mọi request, tool MCP, job và event.

Chưa tạo gói nào thì không đổi gì: khách không có thuê bao dùng **gói mặc định**, không có gói mặc
định thì không bị giới hạn. Khách nền tảng (nhà vận hành) không bao giờ bị giới hạn.

Mọi việc dưới đây do nhân sự nền tảng làm — người của khách nền tảng có `plans.plans.manage` /
`plans.subscriptions.manage` — qua REST (`/api/v1/plans/...`) hay MCP (tool `plans.*`).

## Tạo gói

Xem module, tính năng, hạn mức đang có ở `GET /api/v1/kernel/modules` (`features`, `limits`,
`always_enabled`, `requires`). Module `always_enabled` (đăng nhập, file, audit…) luôn bật với mọi
khách.

```bash
curl -X POST https://api.example.com/api/v1/plans/plans \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"code": "pro", "name": "Pro", "status": "active", "trial_days": 14,
       "modules": ["content", "media", "forms"],
       "features": ["content.ai_suggestions", "auth.api_keys"],
       "limits": {"organizations.units": 10, "auth.members": 25,
                  "files.storage_bytes": 10737418240, "kernel.api_requests": null}}'
```

- Hạn mức đặt `null` là không giới hạn; hạn mức không nêu là `0`.
- Hạn mức chia phần (`parts` trong `GET /api/v1/kernel/modules`) nhận thêm mức riêng cho từng phần:
  `"organizations.units.store": 2` cho tối đa 2 nút loại `store`, trong giới hạn chung
  `organizations.units`. Phần không nêu thì không có mức riêng.
- Gói chính bật một module thì phải bật cả module nó cần (`requires`), không thì trả
  `plans.module_dependency_missing`.
- `"is_default": true` làm gói của khách chưa có thuê bao (chỉ một gói mặc định).
- `"kind": "addon"` là gói thêm: hạn mức cộng thêm vào gói chính, nhân số lượng mua.
- Cài đặt vòng đời theo gói: `grace_days` (7), `suspend_days` (30), `retention_days` (90),
  `renewal_lead_days` (7).

Thêm giá bằng `POST /api/v1/plans/plans/{id}/prices`
(`{"kind": "recurring", "interval": "month", "amount": {"amount": "199000", "currency": "VND"}}`).
Giá không sửa: thêm giá mới rồi tắt giá cũ (`POST /prices/{id}/deactivate`); thuê bao giữ giá cũ tới
khi đổi gói. Gói không bao giờ bị xoá, chỉ ngừng bán (`POST /plans/{id}/retire`).

## Tạo khách theo gói

Nhân sự nền tảng có `plans.provision` tạo một khách dùng được ngay trong một lệnh gọi: khách, thuê
bao, dữ liệu mẫu của các module gói bật (khi gói hay yêu cầu nêu), và lời mời qua email cho quản trị
viên với role `admin`.

```bash
curl -X POST https://api.example.com/api/v1/plans/provisionings \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"slug": "acme", "name": "Acme Ltd", "plan_id": "...", "admin_email": "owner@acme.example"}'
```

Việc chạy nền: theo dõi bằng `GET /api/v1/plans/provisionings/{id}` (`steps`, và `error` khi
`failed`); sửa nguyên nhân rồi `POST …/retry` chạy tiếp từ bước đã xong. Trên máy chủ:
`autoxide tenant create --slug acme --name "Acme Ltd" --plan <id gói> --admin-email
owner@acme.example` làm như vậy ngay. Gói `dedicated` cần `placement`: một placement loại `dedicated`
thêm trước bằng `autoxide placement add`. Gói `requires_kyc` không cấp phát thẳng được: khách tự đăng
ký và qua bước xác thực danh tính (mục sau).

## Khách tự đăng ký

Bật bằng `AUTOXIDE_PLANS_SIGNUP=true` và đánh dấu gói cho chọn bằng `"self_signup": true` (gói chính
đang bán). Trang đăng ký của website gọi API ở tên miền của khách nền tảng (hay gửi
`Autoxide-Tenant: <slug khách nền tảng>`), không cần đăng nhập:

1. `GET /api/v1/antispam/token?scope=plans:signup` lấy token form, rồi `POST /api/v1/plans/signups`
   với `email`, `tenant_name`, `slug`, `plan_code`, `"accept_terms": true` (người đăng ký chấp nhận
   điều khoản sử dụng và chính sách bảo mật hiện hành của nền tảng) và `form_token`. Câu trả lời luôn
   là `202`; giá trị sai trả `422` kèm mọi lỗi một lần (`plan_code` không mở cho tự đăng ký, `slug` đã
   có người dùng…).
2. Một thư gửi tới địa chỉ đó, kèm link dùng được 24 giờ (`AUTOXIDE_PLANS_SIGNUP_URL`, có `{token}`;
   không đặt thì thư chứa token). Trang gửi token tới `POST /api/v1/plans/signups/confirm`: việc
   chấp nhận điều khoản được ghi lại (module `compliance`), và
   - gói `requires_kyc`: đăng ký thành `awaiting_kyc`, `kyc.url` là nơi người đăng ký xác thực danh
     tính (gọi xác nhận lần nữa thì mở lượt mới; xong, nhà cung cấp đưa người dùng về
     `AUTOXIDE_PLANS_SIGNUP_RETURN_URL`). Đạt → tạo khách; không đạt → `rejected` (`kyc_rejected`);
   - gói khác: tạo khách ngay.
3. Tạo khách là đúng lượt cấp phát nhân sự vận hành dùng (`provisioning_id`); người đăng ký được mời
   qua thư làm quản trị viên (role `admin`). Theo dõi bằng `GET /api/v1/plans/signups/{id}?token=…`:
   `pending_email` → (`awaiting_kyc`) → `provisioning` → `completed` (`tenant_id`), hoặc `rejected`
   (`reason`) / `expired`.

Gửi lại cùng email và slug khi đang chờ thì có thư mới (link cũ hết dùng). Hai người cùng chờ một slug:
người đến bước cấp phát trước giữ slug, người sau bị `rejected` (`tenant_slug_taken`). Đăng ký chưa
xác nhận hết hạn sau 24 giờ, chưa xác thực danh tính xong hết hạn sau 30 ngày; đăng ký đã kết thúc bị
xoá sau 30 ngày. Nhân sự nền tảng xem danh sách bằng `GET /api/v1/plans/signups`
(`plans.subscriptions.read`, MCP `plans.list_signups`).

## Gán gói cho khách

```bash
curl -X PUT https://api.example.com/api/v1/plans/tenants/$TENANT/subscription \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"plan_id": "...", "price_id": "...", "addons": [{"plan_id": "...", "quantity": 2}]}'
```

Lần đầu bắt đầu thời gian dùng thử của gói (trừ khi `"trial": false`, hay khách đã dùng thử). Riêng
một khách, có thể bật / tắt một module, tính năng, hay thay một hạn mức, kèm lý do và hạn tuỳ chọn:
`POST /api/v1/plans/tenants/{id}/overrides`.

Thay đổi có hiệu lực ngay ở máy chủ làm thay đổi, và sau tối đa `AUTOXIDE_PLANS_CACHE_SECONDS` (mặc
định 30) ở máy chủ khác.

## Khách thấy gì

- `GET /api/v1/plans/entitlements` (mọi người đã đăng nhập): module, tính năng, từng hạn mức kèm lượng
  đang dùng, và `access` (`full` hay `read_only`) — giao diện ẩn những gì gói không có.
- Endpoint của module gói không có trả `403 module_not_enabled`; tính năng `403 feature_not_enabled`;
  hạn mức đầy `403 limit_exceeded`; hết lượng request của tháng `429 quota_exhausted` (chỉ khi gói đặt
  mức tối đa).
- Có `plans.billing.read` / `plans.billing.manage`: `GET /subscription`, `GET /usage`,
  `GET /billing-periods`, `POST /subscription/change` (nâng gói có hiệu lực ngay và tính tiền phần
  còn lại của kỳ; hạ gói có hiệu lực khi hết kỳ), `POST /subscription/cancel`,
  `POST /subscription/resume`.

## Vòng đời

| Trạng thái | Quyền | Tiếp theo |
| --- | --- | --- |
| `trialing` | đầy đủ | hết dùng thử: gói miễn phí → `active`; có giá → đã trả kỳ kế → `active`, chưa → `past_due` |
| `active` | đầy đủ | hết kỳ: kỳ kế đã trả (hay miễn phí) → kỳ mới; chưa trả → `past_due` |
| `past_due` | đầy đủ (ân hạn) | trả → `active`; quá `grace_days` → `suspended` |
| `suspended` | chỉ đọc | trả → `active`; quá `suspend_days` → `canceled` |
| `canceled` | chỉ đọc | tiếp tục trước `delete_after` → `active`; quá `retention_days` → `deleted` |
| `deleted` | không | khách và dữ liệu bị xoá hẳn (giữ chứng từ kế toán module phải lưu) |

Một job chuyển trạng thái mỗi 5 phút. Trước khi hết kỳ `renewal_lead_days`, kỳ thanh toán kế được lập
và mail nhắc gửi tới email thanh toán của thuê bao (`AUTOXIDE_PLANS_REMINDER_DAYS`, mặc định `7,1`),
gửi lại khi quá hạn và mỗi 3 ngày trong thời gian ân hạn.

## Thanh toán (hiện thu tay)

Tới khi nối cổng thanh toán (bước 36), kỳ thanh toán được trả ngoài hệ thống: kiểm chuyển khoản rồi
`POST /api/v1/plans/billing-periods/{id}/mark-paid` (`{"provider_reference": "..."}`); thuê bao quá hạn
hay tạm khoá thành đang dùng lại. `POST /billing-periods/{id}/void` huỷ kỳ lập nhầm.

Nhân sự nền tảng xem kỳ thanh toán của một khách bằng `GET /api/v1/plans/tenants/{id}/billing-periods`
và hồ sơ xuất hoá đơn (tên pháp lý, mã số thuế, email nhận hoá đơn, địa chỉ — khách tự đặt bằng
`PUT /api/v1/plans/billing-profile`) bằng `GET /api/v1/plans/tenants/{id}/billing-profile`.

Muốn chặn hẳn một khách (cả đọc), dùng `POST /api/v1/plans/tenants/{id}/subscription/suspend` kèm lý
do; mở lại bằng `PATCH /api/v1/kernel/tenants/{id}` (`"status": "active"`).
`GET /api/v1/kernel/tenants?subscription_status=past_due` và `?near_limit=true` liệt kê khách cần theo
dõi.

## Xuất dữ liệu của một khách

Nhân sự nền tảng (`kernel.tenants.operate`) và quản trị viên của chính khách (`kernel.data.export`)
xuất được toàn bộ dữ liệu của khách thành một file JSON Lines: `POST /api/v1/kernel/tenants/{id}/exports`,
rồi `GET …/exports/{export_id}` tới khi `ready`, rồi `GET …/exports/{export_id}/content` (hay
`POST …/link` lấy link 24 giờ mở được không cần đăng nhập). Dòng đầu mô tả bản xuất; mỗi dòng sau là
`{"table": "...", "row": {...}}`. Bản băm mật khẩu và secret bị bỏ ra, nội dung file đã tải lên cũng
không có (có dòng ghi của chúng; tải từng file qua API của module file). Thuê bao bị huỷ thì một lượt
xuất tự bắt đầu. Bản xuất bị xoá sau `AUTOXIDE_TENANCY_EXPORT_RETENTION_DAYS` (30) ngày.

## Xoá hẳn một khách

`POST /api/v1/kernel/tenants/{id}/erase` (`kernel.tenants.delete`, thân `{"confirm": "<slug>",
"reason": "..."}`) xoá hẳn một khách đang tạm khoá cùng dữ liệu của nó; không hoàn tác được. Thuê bao
quá thời hạn lưu tự làm việc này.

## Cấu hình

| Biến | Mặc định | |
| --- | --- | --- |
| `AUTOXIDE_PLANS_CACHE_SECONDS` | `30` | Mỗi máy chủ giữ quyền dùng của khách trong bộ nhớ bao lâu |
| `AUTOXIDE_PLANS_HOLD_MINUTES` | `15` | Chỗ giữ hạn mức chưa xác nhận được giữ bao lâu |
| `AUTOXIDE_PLANS_REMINDER_DAYS` | `7,1` | Trước hạn bao nhiêu ngày thì gửi mail nhắc |
| `AUTOXIDE_PLANS_SIGNUP` | `false` | Cho khách mới tự đăng ký |
| `AUTOXIDE_PLANS_SIGNUP_URL` | — | Trang xác nhận đăng ký, có `{token}` |
| `AUTOXIDE_PLANS_SIGNUP_RETURN_URL` | — | Trang người dùng quay về sau xác thực danh tính, `{id}` = lượt đăng ký |
| `AUTOXIDE_JOBS_BLOCKED_DELAY_SECONDS` | `3600` | Job của module gói không có phải chờ bao lâu |
| `AUTOXIDE_TENANCY_EXPORT_RETENTION_DAYS` | `30` | Giữ bản xuất dữ liệu khách bao lâu |
