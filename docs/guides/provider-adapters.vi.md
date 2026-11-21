# Viết một bộ nối nhà cung cấp

Bộ nối (adapter) nối Autoxide với một dịch vụ bên ngoài: cổng thanh toán, nguồn biến động số dư ngân
hàng, hãng vận chuyển, nhà cung cấp SMS, nhà cung cấp hoá đơn điện tử. Hướng dẫn này dành cho người phát
triển thêm một bộ nối. Mọi bộ nối dựng cùng một cách, để khách cài bộ nối nào cũng bằng cùng một API.

Tiếng Anh: [`provider-adapters.md`](provider-adapters.md).

## Luật

1. **Mỗi bộ nối một crate module nhỏ**, `modules/<tên>`, chỉ phụ thuộc kernel (và module công bố hợp
   đồng, khi hợp đồng không nằm ở kernel, vd `autoxide_shipping::api`). Nó cài một hợp đồng
   (`PaymentGateway`, `BankFeed`, `SmsSender`, `EInvoiceProvider`, `ShippingCarrier`…) và được đăng ký
   trước module dùng nó.
2. **Tài khoản ở nhà cung cấp là của khách (tenant).** API key, mã cửa hàng, secret webhook là cấu hình
   theo khách thuộc nhóm `connection`, không bao giờ là biến môi trường. Hai khách trên một máy chủ
   dùng hai tài khoản khác nhau. Khách chưa đặt tài khoản thì nhận lỗi `provider.not_configured`; bộ
   nối không bao giờ rơi về tài khoản khác.
3. **Secret chỉ ghi, không đọc lại.** Cấu hình bí mật được lưu mã hoá, API không bao giờ trả ra, không
   xuất hiện trong log, event hay lỗi. Người đặt secret qua REST; tool MCP `kernel.set_setting` từ
   chối cấu hình bí mật, nên secret không đi qua agent.
4. **Viết theo tài liệu công khai của nhà cung cấp.** Không viết theo trí nhớ. Ghi URL từng trang đã
   dùng và ngày đọc vào comment đầu module. Cho tới khi bộ nối chạy với dịch vụ thật, mô tả crate và
   tài liệu của nó ghi rõ: *"written from the provider's public documentation; not yet verified against
   the live service"*. Tới lúc đó bộ nối ở hạng thử nghiệm.
5. **Không gọi mạng trong transaction ghi**, và mọi lời gọi ra ngoài có hạn giờ.
6. **Chạy trên cả SQLite và PostgreSQL**, như mọi thứ khác. Phần lớn bộ nối không có bảng nào.

## 1. Khai tài khoản

```rust
use autoxide_kernel::tenancy::{CONNECTION_GROUP, TenantSetting};

// One setting per field of the tenant's account: `<module>.connection.<kind>.<field>`.
pub(crate) const API_KEY: TenantSetting = TenantSetting::new(
    "acme.connection.api.key",
    "API key of this tenant's Acme account (Acme dashboard > Developers).",
)
.secret()
.group(CONNECTION_GROUP);

pub(crate) const SHOP_ID: TenantSetting = TenantSetting::new(
    "acme.connection.api.shop_id",
    "Shop id of this tenant's Acme account.",
)
.group(CONNECTION_GROUP);
```

Trả chúng từ `Module::tenant_settings`. Khách quản lý bằng API đã có sẵn:

| Việc | REST | Tool MCP |
| --- | --- | --- |
| Xem một kết nối cần gì và đã đặt gì | `GET /api/v1/kernel/settings?group=connection` | `kernel.list_settings` |
| Đặt một trường | `PUT /api/v1/kernel/settings/{name}` | `kernel.set_setting` (không nhận secret) |
| Bỏ một trường | `DELETE /api/v1/kernel/settings/{name}` | `kernel.delete_setting` |
| Thử kết nối | `POST /api/v1/kernel/settings/{name}/test` | `kernel.test_connection` |
| Xem địa chỉ webhook | `GET /api/v1/kernel/connections/webhooks` | `kernel.list_connection_webhooks` |

Tất cả cần permission `kernel.settings.manage`, thuộc loại nhạy cảm.

Cấu hình chứa địa chỉ do khách chọn phải được kiểm lúc lưu (`.checked(…)` với
`autoxide_kernel::net::check_target_url`) và client phải dùng `Target::TenantChosen`, để máy chủ chỉ kết
nối tới địa chỉ public.

## 2. Đọc tài khoản lúc được gọi

```rust
use autoxide_kernel::provider::ProviderAccounts;

// In `Module::start`:
let accounts = context.require::<ProviderAccounts>()?;

// In every trait method (the traits pass `TenantId`):
let tenant = self.accounts.tenant(tenant_id).await?;
let key = self.accounts.require(&tenant, &API_KEY).await?; // `provider.not_configured` when unset
let shop = self.accounts.get(&tenant, &SHOP_ID).await?; // optional field
```

## 3. Gọi nhà cung cấp

```rust
use autoxide_kernel::provider::{Target, http_client, send};

let client = http_client(Duration::from_secs(15), Target::Fixed)?;
let request = client
    .post(format!("{}/v1/orders", self.base_url))
    .bearer_auth(key.as_str())
    .header("content-type", "application/json")
    .body(serde_json::to_vec(&body).map_err(Error::internal)?);
let reply = send(request, "acme", "create_order")
    .await
    .map_err(|_| payment::provider_unavailable())?;
if !reply.is_success() {
    return Err(payment::provider_rejected("…the partner's documented error code…"));
}
```

`send` ghi log tên nhà cung cấp, việc đang làm và mã HTTP — không bao giờ ghi địa chỉ, header hay thân
— và đếm lời gọi ở `kernel_provider_calls_total`. Địa chỉ của chính nhà cung cấp không phải cấu hình của
khách: khai nó là cấu hình của người vận hành (`AUTOXIDE_<TÊN>_API_URL`, mặc định là địa chỉ production
của nhà cung cấp) để test và sandbox trỏ đi chỗ khác được.

Gửi ID Autoxide đưa (`payment_id`, `document_id`, `reference`…) làm mã chống trùng hay mã của merchant ở
nhà cung cấp, để gọi lại không tạo thêm một thanh toán, một kiện hàng, một tin nhắn hay một hoá đơn.

Đăng ký một `ConnectionProbe` (`context.connection_probe`) gọi một lời gọi rẻ có xác thực, để khách thử
tài khoản từ trang cài đặt. Báo mỗi lần dùng bằng `context.connection_status().record(…)`; câu báo không
được chứa secret.

## 4. Nhận webhook

Mỗi khách có địa chỉ webhook riêng. Đoạn cuối của nó định danh khách và không đoán được; module nhận
tìm khách từ đó trước, rồi bộ nối kiểm thông báo bằng secret của **đúng khách đó**.

```rust
use autoxide_kernel::provider::{WebhookAck, WebhookEndpoint, constant_time_eq, hex, hmac_sha256};

// In `Module::start`: the tenant then finds its address in `GET /api/v1/kernel/connections/webhooks`.
accounts.register_webhook(WebhookEndpoint::new(
    "acme.connection.api",
    "/api/v1/ledger/webhooks/acme",
    "Paste into Acme dashboard > Webhooks > Endpoint URL.",
));

// In the trait:
async fn verify_webhook(&self, tenant: TenantId, headers: &HeaderMap, body: &[u8])
    -> Result<GatewayNotification, Error> {
    let tenant = self.accounts.tenant(tenant).await?;
    let secret = self.accounts.require(&tenant, &WEBHOOK_SECRET).await?;
    let wanted = hex(&hmac_sha256(secret.as_str().as_bytes(), body)); // on the RAW body
    let given = headers.get("x-acme-signature").and_then(|v| v.to_str().ok()).unwrap_or("");
    if !constant_time_eq(wanted.as_bytes(), given.as_bytes()) {
        return Err(payment::signature_invalid());
    }
    // …only now read the body…
}

fn webhook_ack(&self) -> Option<WebhookAck> {
    Some(WebhookAck::json(200, r#"{"received":true}"#)) // what the partner's documentation asks for
}
```

- Kiểm chữ ký (hay API key) trong thời gian hằng, trên thân thô, trước khi đọc bất cứ gì.
- Cho mỗi thông báo một mã ổn định (`event_id`, `provider_txn_id`): module nhận lưu mỗi mã một lần cho
  mỗi khách, nên thông báo gửi lại không đổi gì.
- Riêng địa chỉ không chứng minh được gì. Nhà cung cấp không có chữ ký lẫn key thì không nối an toàn
  được; không nhận thông báo không xác thực.

Route nhận là `POST /api/v1/ledger/webhooks/{provider}/{hook}` (cổng thanh toán) và
`POST /api/v1/ledger/bank-feeds/{provider}/{hook}` (nguồn biến động số dư). Module nhận loại webhook
khác thêm một route cùng dạng và tìm khách bằng `ProviderAccounts::resolve_webhook`.

## 5. Test không ra mạng

```rust
use autoxide_testkit::{FakeProviderServer, TestApp, TestDatabase};

let server = FakeProviderServer::start().await.unwrap();
server.reply("POST", "/v1/orders", 200, json!({"id": "A1"}));
let app = TestApp::builder(test)
    .setting("AUTOXIDE_ACME_API_URL", server.url())
    .tenant("one")
    .tenant("two")
    // …
    .module(crate::module())
    .build()
    .await
    .unwrap();
// …each tenant sets its own account, call the adapter, then:
let seen = server.take_requests();
assert_eq!(seen[0].header("authorization"), Some("Bearer key-of-one"));
```

Test, trên cả hai database: request đúng như tài liệu của nhà cung cấp mô tả (đường dẫn, header, chữ
ký, thân), các câu trả lời lỗi có trong tài liệu, webhook sai chữ ký, cùng một webhook gửi hai lần, và
hai khách hai tài khoản không bao giờ lẫn nhau.

## 6. Đăng ký và viết tài liệu

- Thêm module vào `crates/app/src/modules.rs`, trước module dùng hợp đồng.
- Thêm `locales/vi.json` cho mọi mã lỗi của bộ nối.
- Thêm một mảnh changelog và, với tính năng tuân thủ, căn cứ pháp lý trong một quyết định.
