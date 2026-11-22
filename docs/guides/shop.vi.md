# Mở một cửa hàng: đơn đầu tiên từ sản phẩm tới hoá đơn điện tử

Hướng dẫn này đưa một máy chủ mới dựng theo [quickstart](../../README.vi.md#chạy-nhanh) tới lần bán thật đầu
tiên: một sản phẩm có giá và tồn kho, giỏ hàng của người mua vãng lai, phí giao do Giao Hàng Nhanh (GHN) báo,
thanh toán bằng MoMo, tin SMS qua eSMS ở từng bước, kiện hàng đặt với GHN, và hoá đơn điện tử phát hành qua SePay
eInvoice khi hàng đã giao. Mỗi lệnh là một lời gọi `curl` dán vào là chạy. Cùng luồng này chạy thành test trên
SQLite và PostgreSQL ở mọi bản build (`crates/app/src/e2e/shop.rs`).

English: [`shop.md`](shop.md).

> **Bộ nối thử nghiệm.** Bốn bộ nối nhà cung cấp (MoMo, GHN, eSMS, SePay eInvoice) được viết theo tài liệu công
> khai của từng nhà cung cấp và chưa chạy thử với dịch vụ thật. Dùng môi trường thử (sandbox) của từng nhà cung
> cấp trước, rồi một đơn thật nhỏ. Tiền luôn đi thẳng từ người mua tới tài khoản MoMo **của chính bạn**; Autoxide
> không bao giờ giữ tiền.

## Cần có

- Một máy chủ dựng từ quickstart, Internet gọi tới được qua HTTPS (MoMo và GHN gửi thông báo về đó). Thử trên máy
  mình thì một đường hầm (vd `cloudflared` hay `ngrok`) tới `http://127.0.0.1:8080` là đủ.
- Tài khoản ở từng nhà cung cấp, đứng tên cửa hàng của bạn: tài khoản doanh nghiệp MoMo (Partner Code, Access
  Key, Secret Key), cửa hàng GHN (API token và ShopId), tài khoản eSMS có brandname đã đăng ký (ApiKey,
  SecretKey) và tài khoản SePay eInvoice (client id, client secret, provider account id). Hướng dẫn riêng của
  từng nhà cung cấp ghi chỗ lấy các giá trị này.
- Trước khi bán cho công chúng ở Việt Nam: thông báo website bán hàng với Bộ Công Thương (Nghị định 52/2013/NĐ-CP
  và văn bản sửa đổi) và công bố điều kiện giao dịch chung; xem module `compliance`. Hướng dẫn này không thay
  cho tư vấn pháp lý.

## 1. Bật các bộ nối

Quickstart chỉ chạy module chính thức. Thêm các dòng sau vào `.env` cạnh `compose.quickstart.yml`, rồi khởi
động lại:

```bash
cat >> .env <<'EOF'
AUTOXIDE_MODULES=official,momo,ghn,esms,einvoice_sepay
AUTOXIDE_HTTP_PUBLIC_URL=https://shop-api.example.com
EOF
docker compose -f compose.quickstart.yml up -d
```

`AUTOXIDE_HTTP_PUBLIC_URL` là địa chỉ HTTPS mà Internet dùng để gọi máy chủ của bạn (địa chỉ của đường hầm khi
thử). Quickstart đã nạp sẵn dữ liệu tham chiếu hướng dẫn này cần: đơn vị hành chính Việt Nam (`address.import`)
và bộ thuế (`tax.import_pack`).

Đăng nhập và giữ token (sống 15 phút; lời gọi trả `401` thì đăng nhập lại). Lưu tài khoản nhà cung cấp đòi lần
đăng nhập gần đây, nên đăng nhập lại trước bước 2 nếu đã lâu.

```bash
API=http://127.0.0.1:8080
TOKEN=$(curl -sS "$API/api/v1/auth/login" -H 'Content-Type: application/json' \
  -d '{"email":"admin@example.com","password":"<your password>"}' | jq -r .data.access_token)
ORG=$(curl -sS "$API/api/v1/organizations/units?limit=1" -H "Authorization: Bearer $TOKEN" | jq -r '.data[0].id')
```

`ORG` là cửa hàng của bạn: nút gốc của cây tổ chức.

## 2. Nối tài khoản nhà cung cấp

Mọi giá trị chỉ lưu cho cửa hàng của bạn; secret được mã hoá và không bao giờ hiện lại. Chi tiết của từng nhà cung
cấp nằm ở hướng dẫn riêng: [MoMo](providers/momo.vi.md), [GHN](providers/ghn.vi.md),
[eSMS](providers/esms.vi.md), [SePay eInvoice](sepay-einvoice.vi.md).

**MoMo** — thanh toán:

```bash
for pair in partner_code=MOMOXXXX access_key=<Access Key> secret_key=<Secret Key>; do
  curl -sS -X PUT "$API/api/v1/kernel/settings/momo.connection.account.${pair%%=*}" \
    -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
    -d "{\"value\": \"${pair#*=}\"}"
done
```

**eSMS** — tin SMS gửi người mua. Chưa đặt thì thông báo SMS được bỏ qua với lý do `channel_disabled` (không thử
lại, không báo lỗi):

```bash
for pair in api_key=<ApiKey> secret_key=<SecretKey> brandname=<Brandname>; do
  curl -sS -X PUT "$API/api/v1/kernel/settings/esms.connection.sms.${pair%%=*}" \
    -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
    -d "{\"value\": \"${pair#*=}\"}"
done
```

**SePay eInvoice** — hoá đơn điện tử, và một ký hiệu hoá đơn dùng nó:

```bash
for pair in client_id=<client id> client_secret=<client secret> provider_account_id=<provider account id>; do
  curl -sS -X PUT "$API/api/v1/kernel/settings/einvoice_sepay.connection.api.${pair%%=*}" \
    -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
    -d "{\"value\": \"${pair#*=}\"}"
done
curl -sS "$API/api/v1/invoices/series" -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"kind": "vat_invoice", "symbol": "1C26TAA", "year": 2026, "einvoice_provider": "sepay"}'
```

**GHN** — giao hàng. Tài khoản hãng trả `webhook_path` **một lần**: giữ lại. Rồi một phương thức giao dùng tài
khoản đó và tự đặt từng kiện với GHN (`auto_book`):

```bash
GHN=$(curl -sS "$API/api/v1/shipping/carrier-accounts" -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d "{\"org_id\": \"$ORG\", \"carrier_code\": \"ghn\", \"name\": \"GHN\", \"credentials\": {\"token\": \"<GHN token>\"}, \"settings\": {\"shop_id\": <ShopId>}}")
echo "$GHN" | jq -r .data.webhook_path
curl -sS "$API/api/v1/shipping/methods" -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d "{\"org_id\": \"$ORG\", \"code\": \"ghn\", \"kind\": \"carrier\", \"names\": {\"vi\": \"Giao Hàng Nhanh\", \"en\": \"GHN\"}, \"currency\": \"VND\", \"carrier_account_id\": \"$(echo "$GHN" | jq -r .data.id)\", \"auto_book\": true}"
```

Trong trang quản lý của GHN, đăng ký `AUTOXIDE_HTTP_PUBLIC_URL` + `webhook_path` làm webhook trạng thái (xem
[hướng dẫn GHN](providers/ghn.vi.md#webhook-trạng-thái)). MoMo không cần làm gì: Autoxide gửi địa chỉ nhận thông
báo của bạn kèm mỗi lần thu tiền.

## 3. Sản phẩm có giá và tồn kho

Nhập cân nặng và kích thước của một món: GHN tính phí kiện hàng theo đó.

```bash
PRODUCT=$(curl -sS "$API/api/v1/catalog/products" -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"kind": "physical", "tax_category": "vat.standard", "locale": "vi", "text": {"name": "Ấm trà"},
       "variant": {"sku": "TEA-1", "weight_grams": 600, "length_mm": 300, "width_mm": 200, "height_mm": 100}}')
PRODUCT_ID=$(echo "$PRODUCT" | jq -r .data.id)
VARIANT=$(echo "$PRODUCT" | jq -r '.data.variants[0].id')

LIST=$(curl -sS "$API/api/v1/catalog/price-lists" -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"name": "Retail", "currency": "VND", "tax_inclusive": true}' | jq -r .data.id)
curl -sS "$API/api/v1/catalog/price-lists/$LIST/prices" -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d "{\"reference\": \"opening\", \"prices\": [{\"variant_id\": \"$VARIANT\", \"amount\": {\"amount\": \"150000\", \"currency\": \"VND\"}}]}"

curl -sS "$API/api/v1/inventory/adjustments" -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d "{\"org_id\": \"$ORG\", \"reference\": \"opening-1\", \"lines\": [{\"variant_id\": \"$VARIANT\", \"quantity\": 10, \"reason_code\": \"found\", \"unit_cost\": {\"amount\": \"90000\", \"currency\": \"VND\"}}]}"

curl -sS -X POST "$API/api/v1/catalog/products/$PRODUCT_ID/status" -H "Authorization: Bearer $TOKEN" \
  -H 'If-Match: *' -H 'Content-Type: application/json' -d '{"status": "active"}'
```

## 4. Mua như người mua vãng lai

Đây là việc website của bạn làm. Không đăng nhập: giỏ được nhận ra bằng `Cart-Token` nó trả về. Giá, giảm giá,
thuế và phí giao luôn do máy chủ tính.

```bash
CART=$(curl -sS "$API/api/v1/orders/carts" -H 'Content-Type: application/json' -d '{}')
CART_ID=$(echo "$CART" | jq -r .data.id)
CART_TOKEN=$(echo "$CART" | jq -r .data.cart_token)

curl -sS -X PUT "$API/api/v1/orders/carts/$CART_ID/lines/$VARIANT" -H "Cart-Token: $CART_TOKEN" \
  -H 'Content-Type: application/json' -d '{"quantity": 2}'

# Where to deliver: codes from GET /api/v1/address/provinces and GET /api/v1/address/units?province=<code>.
curl -sS -X PUT "$API/api/v1/orders/carts/$CART_ID/delivery" -H "Cart-Token: $CART_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"recipient_name": "Nguyễn An", "phone": "0912345678",
       "address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "12 Tràng Tiền"}}' \
  | jq '.data.shipping.options'
```

Danh sách lựa chọn có phí của GHN cho kiện này (người nhận, địa chỉ, cân nặng và kích thước kiện được gửi cho GHN).
Chọn một bằng cách gửi `option_key` của nó cùng địa chỉ, rồi đặt hàng. Giữ nguyên `Idempotency-Key`: gửi lại cùng
yêu cầu trả lại đúng đơn cũ.

```bash
QUOTE=$(curl -sS -X PUT "$API/api/v1/orders/carts/$CART_ID/delivery" -H "Cart-Token: $CART_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"recipient_name": "Nguyễn An", "phone": "0912345678", "shipping_option": "<option_key>",
       "address": {"country": "VN", "province_code": "01", "commune_code": "00004", "line": "12 Tràng Tiền"}}' \
  | jq -r .data.quote_token)

curl -sS "$API/api/v1/orders/checkout" -H "Cart-Token: $CART_TOKEN" -H 'Idempotency-Key: first-order-1' \
  -H 'Content-Type: application/json' \
  -d "{\"cart_id\": \"$CART_ID\", \"quote_token\": \"$QUOTE\", \"payment_method\": \"wallet\",
       \"return_url\": \"https://shop.example.com/thank-you\",
       \"contact\": {\"name\": \"Nguyễn An\", \"phone\": \"0912345678\"}}"
```

`return_url` là trang của website mà người mua quay về sau khi trả; MoMo bắt buộc có (hoặc đặt một lần
`momo.connection.account.return_url` cho cửa hàng). Câu trả lời có đơn (`pending_payment`) và `payment_actions[0]`: một `redirect` tới trang thanh toán của MoMo. Người
mua trả xong, MoMo báo về máy chủ của bạn và đơn tự sang `confirmed` / `paid`. Người mua nhận SMS khi đơn được đặt
và khi thanh toán được xác nhận.

## 5. Giao hàng

Ghi lại id của đơn trong câu trả lời đặt hàng. Yêu cầu giao hàng thì đơn được nhận (`preparing`); GHN tự đặt kiện
vì phương thức có `auto_book`:

```bash
ORDER=<order id>
curl -sS "$API/api/v1/orders/$ORDER/shipments" -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' -d '{}'
curl -sS "$API/api/v1/shipping/shipments?order_id=$ORDER" -H "Authorization: Bearer $TOKEN"
```

Từ đó webhook trạng thái của GHN tự chuyển đơn: đã lấy hàng → `shipping`, đã giao → `delivered`. Lúc giao, hoá đơn
điện tử được phát hành qua SePay eInvoice (`orders.invoice_timing` mặc định là `on_delivered`) và người mua nhận
SMS.

```bash
curl -sS "$API/api/v1/orders/$ORDER" -H "Authorization: Bearer $TOKEN" | jq '.data | {status, payment_status}'
curl -sS "$API/api/v1/invoices/documents?source_module=orders&source_id=$ORDER" -H "Authorization: Bearer $TOKEN" \
  | jq '.data[0] | {status, number, authority_code}'
```

## Khi có việc không xảy ra

| Bạn thấy | Vì sao, và làm gì |
| --- | --- |
| Lời gọi trả `provider.not_configured` | Thiếu một trường của tài khoản nhà cung cấp đó. `GET /api/v1/kernel/settings?group=connection` liệt kê mỗi kết nối cần gì. |
| Một tin SMS có trạng thái `skipped`, lý do `channel_disabled` (`GET /api/v1/notifications/messages`) | Cửa hàng chưa đặt eSMS. Tin trước lúc đó không được gửi lại. |
| Đặt hàng báo thiếu `provider` | Máy chủ có nhiều đối tác thanh toán. Chọn đối tác mặc định: `PUT /api/v1/kernel/settings/ledger.payments.default_provider` với `{"value": "momo"}`. |
| `payment_actions[0].kind` là `none` | Không mở được lượt thu ở MoMo (vd thiếu `return_url`). Sửa xong thì mở lại lượt thu: `POST /api/v1/orders/{id}/payment-link` (nhân viên cửa hàng), hoặc `POST /api/v1/orders/me/orders/{id}/payments` (người mua đã đăng nhập). |
| Đơn nằm ở `pending_payment` | MoMo không gọi tới được `AUTOXIDE_HTTP_PUBLIC_URL`. Kiểm địa chỉ và xem lỗi gần nhất ở `GET /api/v1/kernel/settings?group=connection`. |
| Giỏ không có lựa chọn GHN | GHN từ chối báo giá. Tài khoản hãng hiện lỗi gần nhất (`GET /api/v1/shipping/carrier-accounts`); kiểm cân nặng của món và địa chỉ. |
| Đơn không đổi sau khi GHN đã lấy hàng | Chưa đăng ký webhook GHN, hoặc GHN chưa áp dụng (khoảng 15 phút). |
| Lỗi `ETag` / `If-Match` | Đọc lại bản ghi và gửi `ETag` hiện tại (`If-Match: *` bỏ qua phép kiểm). |
