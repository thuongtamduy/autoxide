# Thử thay đổi trong sandbox

Sandbox là bản thử của khách hàng (tenant) của bạn. Sửa trang, role, mẫu thông báo hay cài đặt ở đó, thử
thanh toán và thông báo mà không chạm tới khách thật hay đối tác thật, rồi đem những gì vừa ý về thành
**bản nháp**. Sandbox chỉ có khi máy chủ chạy nhiều khách (`AUTOXIDE_TENANCY_MODE=multi`).

English: [`sandbox.md`](sandbox.md).

## Tạo sandbox

Cần permission `kernel.sandboxes.manage` ở khách của bạn. Gói dịch vụ quyết bạn giữ được bao nhiêu
sandbox cùng lúc (`kernel.sandboxes`, gói không đặt thì 1).

```bash
curl -sS "$API/api/v1/kernel/tenants/$TENANT/sandboxes" \
  -H "Authorization: Bearer $TOKEN" -H "Idempotency-Key: $(uuidgen)" \
  --json '{"name": "Try the new theme", "role_id": null}'
```

Trả `201` với sandbox (`status: "provisioning"`) và `job_id`. Theo dõi job ở
`GET /api/v1/kernel/jobs/{job_id}`; xong thì sandbox thành `active`.

**Được sao:** cài đặt không bí mật, role, trang (trang đang đăng vẫn đăng), mẫu thông báo đang dùng.
**Không bao giờ sao:** người dùng, người mua, đơn, sổ cái, chứng từ, thông báo, file riêng tư, secret và
thông tin kết nối đối tác, tên miền, API key.

## Làm việc bên trong

Người giữ role `role_id` ở khách của bạn (mặc định role `admin`) vào sandbox bằng một header; không có tài
khoản riêng:

```bash
curl -sS "$API/api/v1/kernel/tenant" \
  -H "Authorization: Bearer $TOKEN" -H "Autoxide-Tenant: $SANDBOX"
```

Mọi response của sandbox mang `Autoxide-Sandbox: true` và `X-Robots-Tag: noindex, nofollow`. Website
công khai của nó có sitemap rỗng và hiện nhãn "bản thử" thay logo của Bộ Công Thương.

Trong sandbox:

- **Đối tác luôn là bộ giả.** Thanh toán thành công, chi trả thành công, hoá đơn điện tử có số bắt đầu
  bằng `SANDBOX-`, xác thực danh tính được duyệt. Không gì tới ngân hàng, hãng vận chuyển, nhà cung cấp
  hoá đơn hay cổng SMS, dù khách của bạn đã cấu hình gì. Các kết quả này không bao giờ là căn cứ pháp lý.
- **Mail** chỉ tới email đã xác minh của nhân viên vào được sandbox. Mail tới người khác bị giữ lại,
  không gửi.
- **Webhook** chỉ gửi tới URL bắt đầu bằng một tiền tố trong cài đặt khách `kernel.sandbox.webhook_urls`
  (đặt trong sandbox; mặc định rỗng). Lần giao khác nhận trạng thái `sandboxed`.

Mọi thứ bị giữ lại nằm trong hộp thư giả của sandbox:

```bash
curl -sS "$API/api/v1/kernel/sandbox/messages?channel=mail" \
  -H "Authorization: Bearer $TOKEN" -H "Autoxide-Tenant: $SANDBOX"
```

## Đem thay đổi về

Từ khách thật (không gửi `Autoxide-Tenant`), đẩy bản ghi của sandbox về. Danh sách loại đẩy được:
`GET /api/v1/kernel/sandboxes/promotable`.

```bash
curl -sS "$API/api/v1/kernel/sandboxes/$SANDBOX/promote" \
  -H "Authorization: Bearer $TOKEN" -H "Idempotency-Key: $(uuidgen)" \
  --json '{"items": [{"kind": "content.page", "id": "'"$PAGE"'"}]}'
```

Mỗi bản ghi nhận một kết quả:

| Kết quả | Nghĩa |
| --- | --- |
| `created_draft` | Bản nháp mới ở khách của bạn (bản ghi được tạo trong sandbox) |
| `updated_draft` | Bản nháp của bản gốc được thay; bản đang chạy giữ nguyên cho tới khi bạn đăng |
| `unchanged` | Đã đẩy với cùng nội dung |
| `conflict` | Slug, tên hay mã đã có ở khách của bạn; xem `conflict.field` |

Không gì được đăng hay bật hộ bạn, và bạn cần cả permission của module (vd `content.write` cho trang).
Agent làm như vậy bằng tool MCP `kernel.list_promotable` và `kernel.promote_sandbox`.

## Gia hạn hay xoá

`POST /api/v1/kernel/sandboxes/{id}/extend` cho thêm `AUTOXIDE_TENANCY_SANDBOX_DAYS` ngày (mặc định 30)
tính từ bây giờ. `DELETE /api/v1/kernel/sandboxes/{id}` xoá ngay sandbox và mọi thứ đã thử trong đó; đẩy
về trước những gì muốn giữ.
