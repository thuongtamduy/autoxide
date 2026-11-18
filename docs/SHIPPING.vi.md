# Vận chuyển (module `shipping`)

[English](SHIPPING.md)

Module `shipping` giữ sổ địa chỉ của người mua, tính phí vận chuyển do từng cửa hàng tự đặt, cắm hãng vận
chuyển dưới dạng module bộ nối, theo dõi vận đơn và đối soát tiền thu hộ (COD). Module không phụ thuộc
`orders` hay `catalog`: `orders` báo giá qua `autoxide_shipping::api` và gửi yêu cầu giao bằng event.

Hướng dẫn này lớn dần theo module. Mục ghi *(đang làm)* là phần còn đang dựng.

## Sổ địa chỉ

Người mua đã đăng nhập lưu tối đa `AUTOXIDE_SHIPPING_MAX_ADDRESSES_PER_BUYER` (mặc định 20) địa chỉ:

| Method | Đường dẫn | Ai |
|---|---|---|
| `GET`, `POST` | `/api/v1/shipping/me/addresses` | mọi tài khoản đã đăng nhập, chỉ địa chỉ của mình |
| `GET`, `PATCH`, `DELETE` | `/api/v1/shipping/me/addresses/{id}` | như trên |
| `POST` | `/api/v1/shipping/me/addresses/{id}/default` | như trên |
| `GET` | `/api/v1/shipping/buyers/{buyer_id}/addresses` | `shipping.addresses.read` (chăm sóc khách) |

- **Việt Nam:** `province_code` và `commune_code` lấy từ danh mục đơn vị hành chính chính thức của module
  `address` (hai cấp từ 01/7/2025). Xã phải thuộc tỉnh và còn hiệu lực; tên tỉnh / xã lưu cùng địa chỉ, nên danh
  mục đổi sau không đổi địa chỉ đã lưu.
- **Nước khác:** dạng chung — `province_code` là vùng (bang, tỉnh…), `city` là thành phố, `postal_code` tuỳ chọn.
  Trong response, thành phố nằm ở `address.commune_name` và `address.commune_code` rỗng.
- Đúng một địa chỉ mặc định (địa chỉ đầu tiên tự thành mặc định). Xoá địa chỉ mặc định thì địa chỉ sửa gần nhất
  còn lại thành mặc định.
- Sổ địa chỉ là dữ liệu cá nhân: nằm trong bản xuất dữ liệu của tài khoản và bị xoá khi tài khoản bị xoá. Vận đơn
  giữ bản chụp người nhận riêng.

Module khác đọc qua `dyn AddressBook` (`get`, `default_for`).

## Phí vận chuyển

Mỗi cửa hàng tự đặt phí dưới `/api/v1/shipping` (permission `shipping.settings.manage` ở nút):

| Method | Đường dẫn | Việc |
|---|---|---|
| `GET`, `POST` | `/zones` | vùng: nhóm tỉnh (hay cả nước) |
| `GET`, `PATCH`, `DELETE` | `/zones/{id}` | một vùng (xoá vùng thì xoá cả bậc phí của nó) |
| `PUT` | `/zones/{id}/regions` | thay danh sách tỉnh của vùng |
| `GET`, `POST` | `/methods` | phương thức giao: `table_rate`, `carrier`, `pickup` |
| `GET`, `PATCH`, `DELETE` | `/methods/{id}` | một phương thức kèm bậc phí và điểm nhận |
| `PUT` | `/methods/{id}/rates` | thay bậc cân nặng của phương thức |
| `PUT` | `/methods/{id}/pickup-points` | thay danh sách cửa hàng của phương thức `pickup` |
| `POST` | `/quote` | báo giá một giỏ (`shipping.read` ở cửa hàng) — cùng cách tính đơn hàng dùng |

- **Kế thừa:** cửa hàng dùng các phương thức đang bật của nút mình và mọi nút trên nó; phương thức của nút gần hơn
  thay phương thức cùng `code` ở trên (cửa hàng ghi đè được `standard` của chuỗi).
- **Vùng:** vùng có tỉnh đích thắng vùng cả nước.
- **Cân nặng:** cân tính phí = max(cân thật, cân quy đổi); cân quy đổi một món = dài × rộng × cao (mm³) /
  `volumetric_divisor` (mặc định 6000) gram, làm tròn lên. Hàng không cần giao (hàng số, dịch vụ) không tính cân; giỏ
  không có món nào cần giao thì không có cách giao. Món thiếu cân dùng `default_item_weight_grams` của phương thức;
  không có thì phương thức nằm trong `unavailable` với `shipping.weight_missing`.
- **Bậc:** `min_weight_grams ≤ cân < max_weight_grams` (không có max = không giới hạn) và cạnh dài nhất ≤
  `max_side_mm`; mỗi `step_grams` (hay phần lẻ) vượt mức tối thiểu cộng `step_amount`. Bậc chồng nhau bị từ chối.
- **Miễn phí:** `free_over` so với giá trị đơn sau giảm giá; lựa chọn trả `free_over` để giỏ báo "mua thêm bao nhiêu".
- **GTGT:** phí đã gồm thuế; mỗi lựa chọn mang `tax_category` của phương thức để đơn hàng ghi dòng thuế của phí giao
  lên hoá đơn.
- **Nhận tại cửa hàng:** mỗi cửa hàng một lựa chọn, phí = `handling_fee` của phương thức, không cần địa chỉ hay cân.
- Mỗi lựa chọn có `option_key`. Gửi lại khi đặt hàng: `ShippingRates::confirm_option` trả `shipping.option_changed`
  khi phí hay địa chỉ đã đổi.

## Hợp đồng cho module khác

```rust
let rates = context.require::<dyn autoxide_shipping::api::ShippingRates>()?;   // quote, confirm_option
let book = context.require::<dyn autoxide_shipping::api::AddressBook>()?;
let carriers = context.require::<dyn autoxide_shipping::api::CarrierRegistry>()?;
```

Module của bạn đăng ký sau `shipping` trong `crates/app/src/modules.rs`. Dựng yêu cầu bằng hàm dựng
(`QuoteRequest::new`, `QuoteItem::new(…).weight(…)`, `ShipmentRequest::new`): các kiểu là `#[non_exhaustive]` nên
thêm trường mới không làm hỏng bên gọi.

`orders` xin giao bằng event `orders.fulfillment_requested` (hay `orders.return_pickup_requested`) với
`{"after": {"shipment": <ShipmentRequest>}}`; `shipping` trả lời bằng các event `shipping.shipment_*` có `after` là
`ShipmentEvent` (không mang dữ liệu cá nhân).

## Viết bộ nối hãng vận chuyển

Bộ nối là một module riêng (`shipping-<hãng>`) đăng ký sau `shipping`. Nó cài `ShippingCarrier` và đăng ký trong
`Module::start`:

```rust
fn start(&mut self, context: &mut StartContext<'_>) -> Result<(), Error> {
    let registry = context.require::<dyn autoxide_shipping::api::CarrierRegistry>()?;
    registry.register(Arc::new(MyCarrier::new()))
}
```

Luật mọi bộ nối phải theo (ví dụ đầy đủ: `autoxide_shipping::api::fake::FakeCarrier`):

- `create_shipment` không tạo vận đơn thứ hai cho cùng `client_reference` (ID vận đơn của Autoxide);
  `find_by_reference` tìm lại vận đơn đã tạo khi câu trả lời bị mất giữa chừng.
- `parse_webhook` kiểm chữ ký trên thân thô (so sánh thời gian hằng) trước khi đọc, sai thì trả
  `shipping.webhook_signature`.
- Đổi giờ của hãng sang UTC (hãng Việt Nam hay gửi giờ địa phương không kèm múi giờ).
- Ánh xạ trạng thái của hãng về `ShipmentStatus` chung; giữ nguyên văn của hãng ở `carrier_status`.
- Không bao giờ log thông tin đăng nhập: `CarrierCredentials` che giá trị khi in `Debug`.

`AUTOXIDE_SHIPPING_FAKE_CARRIER=true` đăng ký hãng giả `fake` để thử mọi thứ trên máy phát triển. `autoxide
doctor` cảnh báo khi bật; không bao giờ bật trên production.

## Vận đơn, COD, ngày giao *(đang làm)*

Tài khoản hãng, webhook, vận đơn nhiều kiện, đối soát
COD, lịch giao và ước tính công khai sẽ được ghi ở đây khi xong.

## Cấu hình

| Biến | Mặc định | Nghĩa |
|---|---|---|
| `AUTOXIDE_SHIPPING_FAKE_CARRIER` | `false` | Đăng ký hãng giả trong bộ nhớ (chỉ để phát triển) |
| `AUTOXIDE_SHIPPING_CARRIER_TIMEOUT_SECONDS` | `10` | Thời gian chờ tối đa cho một lần gọi hãng |
| `AUTOXIDE_SHIPPING_MAX_ADDRESSES_PER_BUYER` | `20` | Số địa chỉ một người mua được lưu |
| `AUTOXIDE_SHIPPING_WEBHOOK_MAX_BYTES` | `262144` | Thân webhook lớn nhất của hãng |
| `AUTOXIDE_SHIPPING_COD_SETTLEMENT_DAYS` | `14` | Số ngày trước khi khoản COD chưa đối soát bị coi là quá hạn |

Thông tin đăng nhập hãng, lịch giao và ngày nghỉ là dữ liệu của từng khách, không phải biến môi trường.
