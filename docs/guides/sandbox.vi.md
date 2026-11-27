# Thử thay đổi trong sandbox

Sandbox là bản thử của khách hàng (tenant) của bạn. Sửa trang, role, mẫu thông báo hay cài đặt ở đó, thử
thanh toán và thông báo mà không chạm tới khách thật hay đối tác thật, rồi đem những gì vừa ý về thành
**bản nháp**. Sandbox chỉ có khi máy chủ chạy nhiều khách (`AUTOXIDE_TENANCY_MODE=multi`).

English: [`sandbox.md`](sandbox.md).

## Tạo sandbox

Cần permission `kernel.sandboxes.manage` ở khách của bạn. Gói dịch vụ quyết bạn giữ được bao nhiêu
sandbox cùng lúc (`kernel.sandboxes`, gói không đặt thì 1).

```bash
TENANT=$(curl -sS "$API/api/v1/kernel/tenant" -H "Authorization: Bearer $TOKEN" | jq -r .data.id)
curl -sS "$API/api/v1/kernel/tenants/$TENANT/sandboxes" \
  -H "Authorization: Bearer $TOKEN" -H "Idempotency-Key: $(openssl rand -hex 16)" \
  --json '{"name": "Try the new theme", "role_id": null}'
```

Trả `201` với sandbox (`status: "provisioning"`) và `job_id`. Theo dõi job ở
`GET /api/v1/kernel/jobs/{job_id}`; xong thì sandbox thành `active`.

**Được sao:** cài đặt không bí mật, role, trang (trang đang đăng vẫn đăng), mẫu thông báo đang dùng.
**Không bao giờ sao:** người dùng, người mua, đơn, sổ cái, chứng từ, thông báo, file riêng tư, secret và
thông tin kết nối đối tác, tên miền, API key. **Cũng không sao:** sản phẩm, giá, tồn kho, cách giao và phí giao,
hồ sơ thuế, dãy hoá đơn. Muốn thử bán hàng thì dựng cửa hàng bên trong sandbox; [hướng dẫn mở cửa
hàng](shop.vi.md#thử-không-cần-tài-khoản-thật) liệt kê các bước.

## Làm việc bên trong

Người giữ role `role_id` ở khách của bạn (mặc định role `admin`) vào sandbox bằng một header; không có tài
khoản riêng:

```bash
curl -sS "$API/api/v1/kernel/tenant" \
  -H "Authorization: Bearer $TOKEN" -H "Autoxide-Tenant: $SANDBOX"
```

Gửi header với **mọi** lời gọi, kể cả lời gọi công khai (giỏ hàng và đặt hàng của người mua vãng lai, mã địa
chỉ): khi không đăng nhập, chính header chọn cửa hàng. Ở đó dùng slug của sandbox cũng được, như id.

Mọi response của sandbox mang `Autoxide-Sandbox: true` và `X-Robots-Tag: noindex, nofollow`. Website
công khai của nó có sitemap rỗng và hiện nhãn "bản thử" thay logo của Bộ Công Thương.

Trong sandbox:

- **Đối tác luôn là bộ giả.** Thanh toán được tạo ở đối tác giả và đứng `pending` tới khi bạn hoàn tất
  ([bên dưới](#hoàn-tất-một-thanh-toán-giả)); hoàn tiền và chi trả thành công; hoá đơn điện tử được chấp nhận
  với mã tham chiếu bắt đầu bằng `SANDBOX-` (`provider_ref`, `lookup_code`, `authority_code`; `number` của hoá
  đơn vẫn như thường); xác thực danh tính được duyệt. Không gì tới ngân hàng, hãng vận chuyển, nhà cung cấp
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

### Hoàn tất một thanh toán giả

Không đối tác nào gửi thông báo cho sandbox, nên khoản thanh toán tạo trong đó (vd một đơn trả bằng
`card` hay `wallet`) đứng `pending` cho tới khi bạn đóng vai đối tác. Người vận hành máy chủ cho phép
trước bằng `AUTOXIDE_LEDGER_SANDBOX_SIMULATION=true` (mặc định tắt; ngoài sandbox không bao giờ dùng
được). Rồi, với `ledger.reconcile`, tìm lượt thu của đơn ở `ledger` (lượt thu của một đơn có `source_id`
là id của đơn; việc thanh toán của đơn cũng cho id này ở `ledger_payment_id`) và trả lời thay đối tác. Lượt thu mới nhất đứng đầu: sau một lượt hỏng, bắt đầu thanh toán lại (`payment-link`)
tạo lượt mới.

```bash
# The order's payment at the partner (the order's id is $ORDER).
LEDGER_PAYMENT_ID=$(curl -sS "$API/api/v1/ledger/payments?source_module=orders&source_id=$ORDER" \
  -H "Authorization: Bearer $TOKEN" -H "Autoxide-Tenant: $SANDBOX" | jq -r '.data[0].id')

curl -sS "$API/api/v1/ledger/payments/$LEDGER_PAYMENT_ID/simulate" \
  -H "Authorization: Bearer $TOKEN" -H "Autoxide-Tenant: $SANDBOX" \
  -H "Idempotency-Key: $(openssl rand -hex 16)" -H 'Content-Type: application/json' \
  -d '{"outcome": "succeeded"}'
```

Lệnh đi đúng đường của thông báo từ đối tác: khoản thành `succeeded` và đơn thành `confirmed` ngay sau
đó. `{"outcome": "failed"}` làm khoản hỏng; `amount` khác số phải thu
(`{"outcome": "succeeded", "amount": {"amount": "100000", "currency": "VND"}}`) đưa khoản vào
`needs_review`, để tập gỡ chênh lệch. Chỉ giả lập được khoản `pending` tạo trong sandbox; chưa hỗ trợ
thanh toán chuyển khoản (VietQR). AI agent dùng tool MCP `ledger.simulate_payment`.

## Đem thay đổi về

Từ khách thật (không gửi `Autoxide-Tenant`), đẩy bản ghi của sandbox về. Danh sách loại đẩy được:
`GET /api/v1/kernel/sandboxes/promotable`.

```bash
curl -sS "$API/api/v1/kernel/sandboxes/$SANDBOX/promote" \
  -H "Authorization: Bearer $TOKEN" -H "Idempotency-Key: $(openssl rand -hex 16)" \
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
