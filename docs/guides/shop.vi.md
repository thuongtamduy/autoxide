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
- `curl` và `jq` (các ví dụ đọc câu trả lời bằng `jq`).
- Chưa có tài khoản nhà cung cấp? Làm mục [Thử không cần tài khoản thật](#thử-không-cần-tài-khoản-thật) trước:
  cùng các bước, trong một sandbox, nơi mọi đối tác đều là bản giả.
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

Trang người mua quay về sau khi trả phải là một địa chỉ của chính cửa hàng: địa chỉ `https://` trên tên miền bạn đã
xác minh (`POST /api/v1/kernel/domains`), hoặc origin bạn khai ở đây — website, tên miền universal link của app,
`http://localhost:3000` lúc phát triển. `return_url` khác bị từ chối, để link thanh toán không đưa người mua của bạn
sang trang của người khác:

```bash
curl -sS -X PUT "$API/api/v1/kernel/settings/ledger.payments.return_origins" \
  -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"value": "https://shop.example.com"}'
```

Xác minh danh tính người bán hay nhân viên theo cùng luật: `return_url` của
`POST /api/v1/compliance/kyc/profiles/{id}/sessions` (và `/me/kyc/sessions`) phải là `https://` trên tên miền đã xác
minh hoặc origin khai ở cấu hình `compliance.kyc.return_origins` (cùng dạng, vd `https://shop.example.com`); không thì
lời gọi trả 422 `compliance.return_url_not_allowed`.

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

Chưa có tài khoản hoá đơn điện tử? Dãy hoá đơn không có `einvoice_provider` vẫn đánh số hoá đơn trong Autoxide
(chứng từ nội bộ, không gửi cơ quan thuế). Không có dãy nào thì không có hoá đơn nào phát hành khi giao hàng; có
hai dãy cùng loại, cùng năm, cùng nút thì chọn một khi phát hành (`series_id`). Yêu cầu lại hoá đơn của một đơn
bằng `POST /api/v1/orders/{id}/invoice`.

```bash
curl -sS "$API/api/v1/invoices/series" -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"kind": "vat_invoice", "symbol": "1C26TAA", "year": 2026}'
```

**Hồ sơ thuế** — ai bán và theo quy tắc thuế nào. Thuế của giỏ hàng và thông tin người bán trên mọi hoá đơn (tên,
mã số thuế, địa chỉ) lấy từ đây; chưa có thì thuế tính theo `vat_credit` và hoá đơn không có thông tin người bán.
`regime` là `vat_credit`, `vat_direct`, `household` hay `exempt`; `business_lines` là `goods`, `services`,
`production_transport` hay `other`:

```bash
curl -sS -X PUT "$API/api/v1/tax/profiles/$ORG" -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d '{"jurisdiction": "VN", "regime": "vat_credit", "tax_code": "0101234567", "legal_name": "Công ty TNHH Trà Việt",
       "business_lines": ["goods"]}'
```

**Phí giao của chính bạn** (không có tài khoản hãng vận chuyển): một vùng, một cách giao với giá cố định theo khối
lượng. Vùng ghi một tỉnh (`{"country": "VN", "province_code": "01"}`) thắng vùng cả nước. Hàng không ghi khối lượng
dùng `default_item_weight_grams`:

```bash
ZONE=$(curl -sS "$API/api/v1/shipping/zones" -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d "{\"org_id\": \"$ORG\", \"name\": \"Viet Nam\", \"regions\": [{\"country\": \"VN\"}]}" | jq -r .data.id)
METHOD=$(curl -sS "$API/api/v1/shipping/methods" -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' \
  -d "{\"org_id\": \"$ORG\", \"code\": \"standard\", \"kind\": \"table_rate\", \"names\": {\"vi\": \"Giao tiêu chuẩn\", \"en\": \"Standard\"},
       \"currency\": \"VND\", \"default_item_weight_grams\": 500}" | jq -r .data.id)
curl -sS -X PUT "$API/api/v1/shipping/methods/$METHOD/rates" -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' \
  -d "{\"rates\": [{\"zone_id\": \"$ZONE\", \"min_weight_grams\": 0, \"amount\": {\"amount\": \"30000\", \"currency\": \"VND\"}}]}"
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

`return_url` là trang của website mà người mua quay về sau khi trả (trên một địa chỉ của cửa hàng, xem bước 2); MoMo
bắt buộc có (hoặc đặt một lần `momo.connection.account.return_url` cho cửa hàng). Câu trả lời có đơn (`pending_payment`) và `payment_actions[0]`: một `redirect` tới trang thanh toán của MoMo. Người
mua trả xong, MoMo báo về máy chủ của bạn và đơn tự sang `confirmed` / `paid`. Người mua nhận SMS khi đơn được đặt
và khi thanh toán được xác nhận.

`payment_method` là `cod` (trả khi nhận hàng, khi giỏ có giao hàng), `bank_transfer`, hay một phương thức của đối tác
thanh toán (`wallet`, `card`…); `payment_methods` của giỏ liệt kê `cod`, `bank_transfer` và `gateway`, trong đó
`gateway` đại diện cho mọi phương thức của đối tác. Đặt hàng không cho người mua vãng lai token nào để xem đơn về
sau: người mua xin một token bằng số đơn và số điện thoại, rồi xác nhận mã gửi qua SMS. `access_token` khi đó đi
trong header `Order-Token` của các lời gọi `/api/v1/orders/public/orders/…`:

```bash
LOOKUP=$(curl -sS "$API/api/v1/orders/public/orders/lookup" -H 'Content-Type: application/json'   -d '{"number": "<order number>", "phone": "0912345678"}' | jq -r .data.lookup_id)
ORDER_TOKEN=$(curl -sS "$API/api/v1/orders/public/orders/lookup/$LOOKUP/confirm" -H 'Content-Type: application/json'   -d '{"code": "<code from the SMS>"}' | jq -r .data.access_token)
```

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

## Thử không cần tài khoản thật

Khi chưa có tài khoản nhà cung cấp, chạy cả luồng trong một [sandbox](sandbox.vi.md): bản sao để thử của một cửa
hàng, nơi mọi đối tác là bản giả (thanh toán, giao hàng, SMS, hoá đơn điện tử) và không gì tới đối tác thật.
Sandbox cần máy chủ nhiều khách, và thanh toán trong đó được hoàn tất bằng tay, nên bật cả hai (cả hai mặc định
tắt):

```bash
# Next to compose.quickstart.yml: several tenants, and simulated payments allowed inside sandboxes.
cat > compose.try.yml <<'EOF'
services:
  autoxide:
    environment:
      AUTOXIDE_TENANCY_MODE: multi
      AUTOXIDE_LEDGER_SANDBOX_SIMULATION: "true"
EOF
docker compose -f compose.quickstart.yml -f compose.try.yml up -d

# A shop tenant and its first administrator (the command prints the password).
X="docker compose -f compose.quickstart.yml -f compose.try.yml exec autoxide autoxide"
$X tenant create --slug demo --name "Demo shop"
$X user create --tenant demo --email owner@demo.example --name "Owner" --admin

# Sign in to the shop, then create a sandbox of it.
API=http://127.0.0.1:8080
TOKEN=$(curl -sS "$API/api/v1/auth/login" -H 'Autoxide-Tenant: demo' -H 'Content-Type: application/json' \
  -d '{"email":"owner@demo.example","password":"<printed password>"}' | jq -r .data.access_token)
TENANT=$(curl -sS "$API/api/v1/kernel/tenant" -H "Authorization: Bearer $TOKEN" -H 'Autoxide-Tenant: demo' | jq -r .data.id)
SANDBOX=$(curl -sS "$API/api/v1/kernel/tenants/$TENANT/sandboxes" -H "Authorization: Bearer $TOKEN" \
  -H 'Autoxide-Tenant: demo' -H "Idempotency-Key: $(openssl rand -hex 16)" -H 'Content-Type: application/json' \
  -d '{"name": "First sale"}' | jq -r .data.id)
curl -sS "$API/api/v1/kernel/tenant" -H "Authorization: Bearer $TOKEN" -H "Autoxide-Tenant: $SANDBOX" | jq .data.status
```

Trạng thái thành `active` khi sao xong (vài giây). Rồi làm bước 1 tới 5 với **hai thay đổi**:

- Gửi `-H "Autoxide-Tenant: $SANDBOX"` với **mọi** lời gọi, kể cả lời gọi công khai (giỏ hàng, đặt hàng, mã địa
  chỉ): khi không đăng nhập, header đó chọn cửa hàng. Bỏ qua tài khoản nhà cung cấp ở bước 2; ở bước 1
  `AUTOXIDE_MODULES` vẫn nêu các bộ nối, để có bản giả của chúng.
- Sandbox mới thì trống: nó sao cấu hình, role và trang, không sao sản phẩm, giá, tồn kho, cách giao hàng, hồ sơ
  thuế hay dãy hoá đơn. Tạo chúng bên trong sandbox (dãy hoá đơn của bước 2 không có `einvoice_provider`, hồ sơ
  thuế, phí giao của chính bạn; bước 3). Có nhiều đối tác thanh toán thì đặt cả đối tác mặc định trong sandbox.

Sau khi đặt hàng, lượt thu đứng `pending` vì không đối tác nào báo về sandbox. Hoàn tất nó như đối tác sẽ làm: xem
[Hoàn tất một thanh toán giả](sandbox.vi.md#hoàn-tất-một-thanh-toán-giả). Đơn thành `confirmed` / `paid`, rồi làm
tiếp bước 5. Xong thì xoá sandbox (`DELETE /api/v1/kernel/sandboxes/{id}` từ cửa hàng, không gửi
`Autoxide-Tenant: $SANDBOX`).

## Khi có việc không xảy ra

| Bạn thấy | Vì sao, và làm gì |
| --- | --- |
| Lời gọi trả `provider.not_configured` | Thiếu một trường của tài khoản nhà cung cấp đó. `GET /api/v1/kernel/settings?group=connection` liệt kê mỗi kết nối cần gì. |
| Một tin SMS có trạng thái `skipped`, lý do `channel_disabled` (`GET /api/v1/notifications/messages`) | Cửa hàng chưa đặt eSMS. Tin trước lúc đó không được gửi lại. |
| Đặt hàng báo thiếu `provider` | Máy chủ có nhiều đối tác thanh toán. Chọn đối tác mặc định: `PUT /api/v1/kernel/settings/ledger.payments.default_provider` với `{"value": "momo"}`. |
| `payment_actions[0].error.code` là `orders.return_url_not_allowed` (hoặc `payment-link` trả 422 với mã đó) | `return_url` không nằm trên địa chỉ nào của cửa hàng. Thêm origin của nó vào `ledger.payments.return_origins` (bước 2) hoặc xác minh tên miền, rồi mở lại lượt thu. |
| `payment_actions[0].kind` là `none` | Không mở được lượt thu ở MoMo; `payment_actions[0].error` nói lý do (`code` và `message`, vd `momo.return_url_required`). Sửa xong thì mở lại lượt thu: `POST /api/v1/orders/{id}/payment-link` (nhân viên cửa hàng), `POST /api/v1/orders/me/orders/{id}/payment-link` (người mua đã đăng nhập) hoặc `POST /api/v1/orders/public/orders/{id}/payment-link` kèm header `Order-Token` (người mua vãng lai); người mua gửi được `{"return_url": "…"}`. |
| `GET /api/v1/ledger/payments/{id}` trả 404 với một `payment_id` | `payment_actions[].payment_id` là khoản của chính đơn. Dùng `payment_actions[].ledger_payment_id` với các endpoint của `ledger`. |
| `payment-link` trả `payment_id` mới | Đối tác báo lượt thu trước đã hỏng (người mua từ chối hay bỏ dở) nên một lần thử mới được mở (khoản cũ có trạng thái `superseded`). Đưa người mua việc mới và dùng `status_token` của nó. Nếu đối tác vẫn thu lượt cũ, tiền đó tự được hoàn qua đối tác. |
| Một khoản ở `needs_review` và đơn đứng `pending_payment` | Đối tác báo số tiền khác số phải thu. Thông báo sửa lại cho cùng giao dịch không gỡ được: người có `ledger.reconcile` tìm mục `amount_mismatch` (`GET /api/v1/ledger/exceptions?status=open`) rồi xử lý bằng `POST /api/v1/ledger/exceptions/{id}/resolve`: `{"action": "accept_received", "payment_id": "<id lượt thu ở ledger>"}` ghi số thực thu và xác nhận đơn nếu đơn còn chờ thanh toán. Hãy xử lý trước hạn thanh toán của đơn (mặc định 30 phút, `AUTOXIDE_LEDGER_PAYMENT_EXPIRY_MINUTES`): sau hạn, đơn đã bị huỷ, nên số đã thu được tự hoàn qua đối tác và đơn không mở lại. `refunded_outside` hay `ignore` đóng mục và đơn bị huỷ khi tới hạn. |
| Có mục `late_payment` đang mở trong `GET /api/v1/ledger/exceptions` | Tiền về sau hạn của lượt thu hay sau khi đối tác đã báo lượt thu hỏng. Đơn quyết định: nếu đơn vẫn tính khoản đó (đơn đi tiếp), mục tự đóng ngay sau đó (resolution `kept`); khi đơn không còn cần khoản đó, Autoxide tự hoàn qua đối tác, và mục tự đóng khi khoản đã hoàn đủ (resolution `refunded`). Không bao giờ hoàn tay một mục mà đơn của nó đã đi tiếp. Mục còn mở nghĩa là đối tác đã từ chối lần hoàn đó: trước hết xem `refunds` trong `GET /api/v1/ledger/payments/{id}` (có thể còn một lượt hoàn đang chờ ở đối tác), rồi mới hoàn ngoài Autoxide và xử lý mục bằng `refunded_outside`, để người mua không bị hoàn hai lần. |
| Có mục `refund_failed` đang mở trong `GET /api/v1/ledger/exceptions` | Đối tác đã từ chối một lượt hoàn của khoản đó, hay lượt hoàn không gửi đi được (ví dụ khoản chuyển khoản ngân hàng, không đối tác nào hoàn, của một đơn bị huỷ sau khi tiền đã về). Không có gì tự gửi lại. Hoàn cho người mua ngoài Autoxide, rồi xử lý mục bằng `refunded_outside`; nếu sau đó một lượt hoàn qua đối tác trả đủ khoản, mục tự đóng. Lượt hoàn nhận `409`, `429` hay lỗi máy chủ thì chưa bị từ chối: nó giữ `pending` và được gửi lại cùng mã tham chiếu. |
| Đơn nằm ở `pending_payment` | MoMo không gọi tới được `AUTOXIDE_HTTP_PUBLIC_URL`. Kiểm địa chỉ và xem lỗi gần nhất ở `GET /api/v1/kernel/settings?group=connection`. |
| Giỏ không có lựa chọn GHN | GHN từ chối báo giá. Tài khoản hãng hiện lỗi gần nhất (`GET /api/v1/shipping/carrier-accounts`); kiểm cân nặng của món và địa chỉ. |
| Đơn không đổi sau khi GHN đã lấy hàng | Chưa đăng ký webhook GHN, hoặc GHN chưa áp dụng (khoảng 15 phút). |
| Lỗi `ETag` / `If-Match` | Đọc lại bản ghi và gửi `ETag` hiện tại (`If-Match: *` bỏ qua phép kiểm). |
