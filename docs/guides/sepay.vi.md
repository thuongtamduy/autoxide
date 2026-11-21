# Xác nhận chuyển khoản bằng SePay

[SePay](https://sepay.vn) báo các giao dịch của chính tài khoản ngân hàng của bạn. Với bộ nối `sepay`, một
thanh toán VietQR tạo trong Autoxide tự được xác nhận khi tiền người mua chuyển tới: tiền vào thẳng tài
khoản ngân hàng của bạn, Autoxide chỉ đọc thông báo.

Tiếng Anh: [`sepay.md`](sepay.md).

> **Thử nghiệm.** Bộ nối này viết theo tài liệu công khai của nhà cung cấp, chưa kiểm với dịch vụ thật
> (written from the provider's public documentation; not yet verified against the live service). Hãy thử
> bằng một khoản chuyển nhỏ thật (hoặc chế độ thử của SePay) trước khi dựa vào nó.

Mỗi khách (tenant) nối tài khoản SePay **của riêng mình**. Không có gì ở đây là cấu hình chung của máy
chủ; secret bạn lưu được mã hoá và không bao giờ hiện lại.

## 1. Lưu tài khoản SePay của bạn

Cần permission `kernel.settings.manage`.

```bash
# 1. The webhook secret you chose at SePay (HMAC-SHA256)
curl -X PUT "$API/api/v1/kernel/settings/sepay.connection.bank.webhook_secret" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"value": "<secret key of the SePay webhook>"}'

# 2. The API token, to read transactions a webhook missed and to test the connection
curl -X PUT "$API/api/v1/kernel/settings/sepay.connection.bank.api_token" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"value": "<API token of the SePay company>"}'

# 3. Test it
curl -X POST "$API/api/v1/kernel/settings/sepay.connection.bank.api_token/test" \
  -H "Authorization: Bearer $TOKEN"
```

| Cấu hình | Là gì |
| --- | --- |
| `sepay.connection.bank.webhook_secret` | Secret Key của webhook SePay khi chọn xác thực **HMAC-SHA256** (khuyên dùng). |
| `sepay.connection.bank.webhook_api_key` | API Key của webhook SePay khi chọn xác thực **API Key**. Chỉ dùng khi chưa đặt secret key. |
| `sepay.connection.bank.api_token` | API token của công ty bạn ở SePay (Cấu hình công ty > API Keys). |

Thông báo bị từ chối cho tới khi một trong hai cấu hình webhook được đặt. Autoxide không bao giờ nhận
thông báo không xác thực, nên đừng chọn "không xác thực" ở SePay.

## 2. Tạo webhook ở SePay

Hỏi Autoxide địa chỉ webhook của bạn. Địa chỉ này của riêng bạn; đoạn cuối định danh khách của bạn.

```bash
curl "$API/api/v1/kernel/connections/webhooks" -H "Authorization: Bearer $TOKEN"
```

Ở my.sepay.vn > Webhooks, thêm một webhook với:

- **URL**: giá trị `url` của dòng `sepay.connection.bank` (hoặc địa chỉ công khai của máy chủ nối với
  `path`).
- **Định dạng dữ liệu**: JSON.
- **Xác thực**: HMAC-SHA256, cùng secret key bạn đã lưu ở bước 1.
- **Tự động gửi lại khi server trả lỗi**: bật, để thông báo bị từ chối lúc bảo trì được gửi lại.

Autoxide trả `{"success": true}` như SePay chờ, kiểm chữ ký trên thân thô, từ chối thông báo ký quá 5
phút trước, và thông báo gửi lại chỉ được lưu một lần.

## 3. Khai tài khoản ngân hàng mà SePay báo

```bash
curl -X POST "$API/api/v1/ledger/bank-accounts" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"bank_bin": "970436", "account_number": "1017588888", "account_name": "CUA HANG A", "feed_provider": "sepay"}'
```

Từ đó tạo thanh toán bằng `POST /api/v1/ledger/payments/vietqr`. Khoản chuyển có nội dung giữ
`transfer_code` của thanh toán và đúng số tiền sẽ đánh dấu thanh toán đã trả. Sai số tiền, mã lạ hay
khoản chuyển thứ hai thì chờ người xử lý ở `GET /api/v1/ledger/exceptions`.

## Nên biết

- SePay không gửi webhook cho giao dịch qua mã QR tĩnh NAPAS. Khi đã đặt API token, Autoxide đọc danh
  sách giao dịch thay thế lúc tài khoản ngân hàng được kéo biến động.
- Giao dịch tới Autoxide qua webhook rồi lại qua danh sách giao dịch chỉ được lưu một lần khi ngân hàng
  cấp mã tham chiếu. Không có mã tham chiếu thì bản thứ hai không được khớp vào thanh toán lần nữa: nó chờ
  ở danh sách ngoại lệ.
- API của SePay cho 3 request mỗi giây; khi SePay báo "quá nhiều request" thì việc được thử lại sau.
- Trang thanh toán của cổng thanh toán SePay (thẻ, NAPAS) không được nối bởi bộ nối này.
