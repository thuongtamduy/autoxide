# Giao Hàng Nhanh (GHN)

Nối tài khoản Giao Hàng Nhanh của chính khách vào phần vận chuyển của Autoxide: phí vận chuyển lúc đặt hàng, đặt vận
đơn, in vận đơn, huỷ và trạng thái giao.

Tiếng Anh: [`ghn.md`](ghn.md).

> **Thử nghiệm.** Bộ nối này viết theo tài liệu công khai của GHN (<https://developer.ghn.vn>, đọc ngày 2026-11-21);
> chưa kiểm với dịch vụ thật. Hãy thử bằng tài khoản staging của GHN trước. Những điểm còn cần tài khoản thật để kiểm
> nằm ở mục [Chưa kiểm chứng](#chưa-kiểm-chứng).

## Cho người vận hành máy chủ

Module tắt theo mặc định. Bật bằng tên; `shipping` được bật kèm:

```bash
AUTOXIDE_MODULES=official,ghn
```

| Cấu hình | Mặc định | Nghĩa |
| --- | --- | --- |
| `AUTOXIDE_GHN_API_URL` | `https://online-gateway.ghn.vn` | API của GHN cho tài khoản thật |
| `AUTOXIDE_GHN_SANDBOX_API_URL` | `https://dev-online-gateway.ghn.vn` | API staging của GHN, cho tài khoản đánh dấu `sandbox` |

Không có cấu hình nào cho token GHN: mỗi khách dùng tài khoản của mình. Nếu token GHN có danh sách IP được phép, thêm
IP public của máy chủ này vào đó.

## Khách cần lấy gì ở GHN

1. Đăng nhập <https://developer.ghn.vn> (staging: <https://developer.ghn.dev>) bằng số điện thoại của tài khoản GHN.
2. Menu tài khoản → **Manage token**: bấm xem **Token API** (GHN gửi OTP về điện thoại) rồi chép lại.
3. Ghi lại **ShopId** của cửa hàng gửi hàng (ở phần thông tin tài khoản). Hàng được lấy tại địa chỉ của cửa hàng đó.

Staging và production là hai tài khoản riêng, hai token riêng.

## Nối tài khoản

Cần permission `shipping.carriers.manage`. Token được lưu mã hoá và không bao giờ trả lại.

```http
POST /api/v1/shipping/carrier-accounts
{
  "carrier_code": "ghn",
  "name": "GHN cửa hàng chính",
  "credentials": { "token": "<Token API>" },
  "settings": { "shop_id": 196560 },
  "sandbox": false
}
```

`sandbox: true` dành cho token lấy ở <https://developer.ghn.dev>. `settings` tuỳ chọn:

| Cài đặt | Mặc định | Nghĩa |
| --- | --- | --- |
| `required_note` | `KHONGCHOXEMHANG` | Hướng dẫn giao: `KHONGCHOXEMHANG` (không cho xem hàng), `CHOXEMHANGKHONGTHU` (cho xem), `CHOTHUHANG` (cho thử) |
| `payment_type_id` | `1` | Ai trả phí cho GHN: `1` cửa hàng, `2` người nhận |
| `label_size` | `a5` | Khổ vận đơn: `a5`, `80x80` hoặc `52x70` |
| `content` | `Hàng hóa` | Mô tả hàng gửi cho GHN |

Câu trả lời có `webhook_path` **một lần duy nhất** — giữ lại cho bước sau.

Rồi thử kết nối: `POST /api/v1/shipping/carrier-accounts/{id}/test`. `check_status` thành `ok`, hoặc `failed` kèm
`check_error_code`:

| Mã | Cần làm gì |
| --- | --- |
| `shipping.carrier_unauthorized` | GHN từ chối token (sai token, sai môi trường, hoặc IP của máy chủ này không nằm trong danh sách được phép) |
| `ghn.shop_not_found` | Token không quản lý ShopId này, hoặc cửa hàng bị khoá |
| `ghn.account_incomplete` | Thiếu `token` hoặc `shop_id` |
| `shipping.carrier_unavailable` | GHN không trả lời; thử lại |

Cuối cùng tạo một phương thức vận chuyển loại `carrier` với `carrier_account_id` này (xem
[`docs/SHIPPING.vi.md`](../../SHIPPING.vi.md)). `service_code` là `2` (hàng nhẹ, dưới 20 kg), `5` (hàng nặng hoặc
nhiều kiện), hoặc để trống để chọn theo cân nặng và số kiện.

## Webhook trạng thái

1. Ghép địa chỉ đầy đủ: địa chỉ public của máy chủ Autoxide + `webhook_path`, ví dụ
   `https://api.example.com/api/v1/shipping/webhooks/ghn/<hook>/<token>`.
2. Ở GHN Developer Portal: menu tài khoản → **Webhook configuration** → tab **Order** → dán địa chỉ → **Create
   webhook**. GHN cần khoảng 15 phút để áp thay đổi.

Hãy giữ địa chỉ này như một bí mật: GHN không ký callback, nên chính địa chỉ là thứ chứng minh callback thuộc tài
khoản này. Autoxide còn kiểm `ShopID` trong callback phải là cửa hàng của tài khoản, sai thì trả 401. Nếu địa chỉ bị
lộ, tạo tài khoản hãng mới (địa chỉ mới) và tắt tài khoản cũ.

Callback gửi lại chỉ được áp một lần. Chỉ thay đổi trạng thái được dùng; callback về phí, cân nặng và chuyển tiền COD
được nhận rồi bỏ qua.

## Trạng thái của GHN hiện ra thế nào

| GHN | Autoxide |
| --- | --- |
| `ready_to_pick`, `picking`, `money_collect_picking` | `ready_to_ship` |
| `picked` | `picked_up` |
| `storing`, `sorting`, `transporting` | `in_transit` |
| `delivering`, `money_collect_delivering` | `out_for_delivery` |
| `delivered` | `delivered` |
| `delivery_fail` | `delivery_failed` |
| `waiting_to_return`, `return`, `return_transporting`, `return_sorting`, `returning`, `return_fail` | `returning` |
| `returned` | `returned` |
| `cancel` | `cancelled` |
| `lost`, `damage`, `scrap` | `lost` |
| `exception` | không áp (xử lý với GHN) |

Trạng thái nguyên văn của GHN được giữ ở `carrier_status` của từng bước hành trình.

## Giới hạn

- **Địa chỉ** gửi theo mô hình hai cấp của Việt Nam có hiệu lực từ 01/07/2025 (tỉnh + phường / xã, bằng tên). Không
  dùng mã quận của mô hình cũ của GHN, nên không có bảng mã nào phải bảo trì.
- Chỉ trong Việt Nam, chỉ tiền VND.
- Giới hạn của GHN: 50.000 g mỗi đơn, 200 cm mỗi cạnh, thu hộ tối đa 50.000.000 đ. Giá trị khai báo trên 5.000.000 đ
  được gửi là 5.000.000 (mức trần bảo hiểm của GHN).
- Kiện không có kích thước được gửi là 10 × 10 × 10 cm; GHN cân lại và có thể đổi phí.
- **Thu hộ (COD)**: GHN thu và trả thẳng cho cửa hàng; tiền không bao giờ đi qua Autoxide. Tài liệu công khai của GHN
  không có API bảng kê, nên đối soát bằng cách nhập bảng kê của GHN (`POST /api/v1/shipping/cod/statements`).
- Lấy hàng trả từ người mua chưa đặt được qua bộ nối (`ghn.direction_unsupported`).
- Tra trạng thái bằng hỏi định kỳ chỉ trả trạng thái hiện tại; hành trình đầy đủ đến từ webhook.

## Mã lỗi

| Mã | Nghĩa |
| --- | --- |
| `ghn.account_incomplete` | Tài khoản hãng thiếu `token` hoặc `shop_id` |
| `ghn.setting_invalid` | Một cài đặt tuỳ chọn của tài khoản mang giá trị GHN không nhận |
| `ghn.shop_not_found` | Token không quản lý cửa hàng, hoặc cửa hàng bị khoá |
| `ghn.permission_missing` | GHN trả 403: tài khoản thiếu quyền; liên hệ GHN |
| `ghn.currency_unsupported` | Số tiền không phải VND |
| `ghn.country_unsupported` | Địa chỉ ngoài Việt Nam |
| `ghn.service_unknown` | `service_code` không phải `2` hay `5` |
| `ghn.direction_unsupported` | Chưa hỗ trợ lấy hàng trả |
| `ghn.phone_invalid` | GHN từ chối một số điện thoại |
| `ghn.address_not_recognized` | GHN không nhận ra tên tỉnh hoặc phường / xã |
| `ghn.route_not_supported` | GHN không phục vụ tuyến với dịch vụ đã chọn |
| `ghn.order_not_found` | GHN không có đơn này trong tài khoản |
| `ghn.webhook_body` | Thân callback không phải JSON của GHN |
| `shipping.carrier_error` | Mọi từ chối khác; `message` mang lý do của GHN |

## Chưa kiểm chứng

Những điểm sau làm theo tài liệu ở chỗ tài liệu chưa nói rõ, cần chạy với tài khoản thật:

- Báo giá dùng chức năng xem trước đơn của GHN với người nhận giữ chỗ (`Quote`, `0900000000`).
- Tên tỉnh và phường / xã gửi đúng như module `address` ghi (ví dụ "Thành phố Hồ Chí Minh"); danh mục của GHN ghi tên
  chuẩn ngắn kèm các biến thể.
- Trang in vận đơn được chuyển nguyên như GHN phục vụ (PDF khi file là PDF, còn lại là HTML).
- GHN không nêu giới hạn tần suất gọi.
