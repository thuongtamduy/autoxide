# Nhận thanh toán qua MoMo

[MoMo](https://momo.vn) là ví điện tử ở Việt Nam. Với bộ nối `momo`, một thanh toán tạo trong Autoxide
có trang thanh toán MoMo (và mã QR) cho người mua, và tự được xác nhận khi MoMo báo kết quả. Tiền đi từ
người mua vào tài khoản MoMo doanh nghiệp **của chính bạn**; Autoxide không bao giờ giữ tiền.

English: [`momo.md`](momo.md).

> **Thử nghiệm.** Bộ nối này viết theo tài liệu công khai của nhà cung cấp, chưa kiểm với dịch vụ thật.
> Hãy thử với môi trường sandbox của MoMo, rồi với một thanh toán thật số tiền nhỏ, trước khi dựa vào
> nó. Bộ nối dùng Payment Gateway API v2 của MoMo (`captureWallet`).

Mỗi khách (tenant) nối tài khoản MoMo **của riêng mình**. Không có tài khoản nào dùng chung cho cả máy
chủ, và các khoá bạn lưu được mã hoá, không bao giờ hiện lại.

## Trước khi bắt đầu

- Người vận hành máy chủ bật module: `AUTOXIDE_MODULES=official,momo` (xem
  [`docs/MODULES.vi.md`](../../MODULES.vi.md)).
- Người vận hành máy chủ đặt `AUTOXIDE_HTTP_PUBLIC_URL` là địa chỉ mà internet gọi tới API
  (`https://…`). MoMo gửi kết quả thanh toán về địa chỉ đó.
- Bạn có tài khoản MoMo doanh nghiệp. Ở trang MoMo for Business (business.momo.vn), mục thông tin tích
  hợp có ba giá trị: **Partner Code**, **Access Key** và **Secret Key**. Môi trường sandbox và môi
  trường production mỗi bên có một bộ riêng.

## 1. Lưu tài khoản MoMo của bạn

Cần permission `kernel.settings.manage`.

```bash
curl -X PUT "$API/api/v1/kernel/settings/momo.connection.account.partner_code" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"value": "<Partner Code>"}'

curl -X PUT "$API/api/v1/kernel/settings/momo.connection.account.access_key" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"value": "<Access Key>"}'

curl -X PUT "$API/api/v1/kernel/settings/momo.connection.account.secret_key" \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"value": "<Secret Key>"}'

# Thử kết nối: Autoxide hỏi MoMo về một thanh toán không có. MoMo trả "không tìm thấy" khi nó nhận bộ
# khoá của bạn.
curl -X POST "$API/api/v1/kernel/settings/momo.connection.account.secret_key/test" \
  -H "Authorization: Bearer $TOKEN"
```

| Cấu hình | Bắt buộc | Là gì |
| --- | --- | --- |
| `momo.connection.account.partner_code` | có | Partner Code của tài khoản MoMo doanh nghiệp. |
| `momo.connection.account.access_key` | có | Access Key. Lưu mã hoá, không bao giờ trả lại. |
| `momo.connection.account.secret_key` | có | Secret Key: ký mọi request và kiểm mọi thông báo. Lưu mã hoá, không bao giờ trả lại. |
| `momo.connection.account.environment` | không | `production` (mặc định) hoặc `sandbox`. Dùng `sandbox` với bộ khoá thử của MoMo: không có tiền thật. |
| `momo.connection.account.return_url` | không | Trang trên website của bạn mà người mua quay về sau khi trả tiền, dùng khi thanh toán không có trang quay về riêng. |

Hai khoá là secret: đặt bằng REST như trên. Tool MCP `kernel.set_setting` từ chối secret, nên khoá không
bao giờ đi qua AI agent.

## 2. Địa chỉ nhận thông báo — không phải dán gì

MoMo gọi đây là "IPN". Autoxide gửi địa chỉ nhận thông báo riêng của bạn cho MoMo trong từng thanh toán
nó tạo, nên không phải cấu hình gì ở MoMo. Bạn xem được địa chỉ đó (đoạn cuối định danh khách của bạn
và không đoán được):

```bash
curl "$API/api/v1/kernel/connections/webhooks" -H "Authorization: Bearer $TOKEN"
```

Tìm dòng `momo.connection.account`. Mọi thông báo đều được kiểm bằng Secret Key **của bạn** trước khi
ghi nhận bất cứ gì; thông báo ký bằng khoá khác bị từ chối với `payment.signature_invalid`. MoMo có thể
gửi cùng một thông báo nhiều lần; nó chỉ được ghi nhận một lần.

## 3. Nhận một thanh toán

Tạo thanh toán với đối tác `momo` và phương thức `wallet`, ví dụ bằng `POST /api/v1/ledger/payments`
(permission `ledger.payments_create`), hoặc qua một module thu tiền thay bạn (đơn hàng, đặt lịch…). Câu
trả lời có:

- `checkout_url` — trang thanh toán của MoMo. Đưa người mua tới đó.
- `qr_payload` — nội dung mã QR, khi MoMo trả cho tài khoản của bạn.

Khi người mua trả xong, MoMo báo cho Autoxide và thanh toán chuyển sang `succeeded`. Nếu MoMo báo số
tiền khác với số thanh toán yêu cầu thì không ghi sổ: thanh toán chuyển sang `needs_review` và chờ người
xử lý ở hàng chờ đối soát.

Giới hạn theo tài liệu của MoMo: chỉ VND, từ 1.000 đến 50.000.000 VND cho một thanh toán bằng ví.

## 4. Hoàn tiền

Hoàn tiền một thanh toán như thường lệ (`POST /api/v1/ledger/payments/{id}/refunds`). Hoàn toàn bộ và
hoàn một phần đều gửi tới MoMo và có kết quả ngay.

## Mã lỗi

| Mã | Nghĩa | Làm gì |
| --- | --- | --- |
| `provider.not_configured` | Thiếu một cấu hình bắt buộc ở bước 1. | Đặt nó; câu lỗi nêu tên. |
| `momo.account_rejected` | MoMo từ chối bộ khoá (mã kết quả 11 và 13 của MoMo). | Kiểm ba giá trị và môi trường. |
| `momo.return_url_required` | MoMo cần trang quay về. | Gửi `return_url` kèm thanh toán, hoặc đặt `momo.connection.account.return_url`. |
| `orders.return_url_not_allowed` (đơn hàng), hoặc lỗi ô `ledger.return_url_not_allowed` (`POST /api/v1/ledger/payments`) | `return_url` gửi lên không phải địa chỉ của cửa hàng. | Dùng địa chỉ `https://` trên tên miền đã xác minh, hoặc khai origin của nó ở `ledger.payments.return_origins` (xem [hướng dẫn mở cửa hàng](../shop.vi.md#2-nối-tài-khoản-nhà-cung-cấp)). |
| `momo.amount_out_of_range` | Ngoài khoảng 1.000–50.000.000 VND. | Dùng phương thức thanh toán khác. |
| `momo.currency_not_supported` | Không phải VND. | Thu bằng VND. |
| `momo.order_already_created` | Một lần gọi trước đã tới MoMo, và MoMo không cấp lại link thanh toán. | Tạo thanh toán mới với mã tham chiếu mới. |
| `momo.refund_already_sent` | Một lần gọi trước của lệnh hoàn này đã tới MoMo; không gửi lại. | Xem lệnh hoàn ở trang MoMo for Business. |
| `payment.provider_rejected` | MoMo từ chối; câu lỗi có mã kết quả của MoMo. | Xem danh sách mã kết quả của MoMo. |
| `payment.provider_unavailable` | MoMo không trả lời, hoặc đang bận. | Thử lại sau. |
| `public_url_missing` | Máy chủ chưa có địa chỉ công khai. | Người vận hành đặt `AUTOXIDE_HTTP_PUBLIC_URL`. |

## Chưa làm

- Sản phẩm khác của MoMo: thẻ ATM và thẻ quốc tế qua MoMo, trả sau, thanh toán định kỳ, chi hộ.
- Sao kê: API thanh toán của MoMo không có, nên đối soát theo sao kê
  (`POST /api/v1/ledger/reconciliations`) trả `payment.not_supported` cho `momo`. Hãy so với báo cáo ở
  trang MoMo for Business.

## Cho người vận hành

| Biến | Mặc định | Là gì |
| --- | --- | --- |
| `AUTOXIDE_MOMO_API_URL` | `https://payment.momo.vn` | API production của MoMo. |
| `AUTOXIDE_MOMO_SANDBOX_API_URL` | `https://test-payment.momo.vn` | API sandbox của MoMo, cho khách đặt `environment` = `sandbox`. |

Nguồn: trang dành cho nhà phát triển của MoMo, <https://developers.momo.vn/v3/docs/payment/api/wallet/onetime/>,
và mã mẫu chính thức của MoMo, <https://github.com/momo-wallet/payment> (đọc ngày 2026-11-21).
