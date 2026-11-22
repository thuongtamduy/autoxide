# Gửi SMS qua eSMS

Module `esms` gửi mã một lần và tin chăm sóc khách hàng qua [eSMS](https://esms.vn), nhà cung cấp SMS
brandname ở Việt Nam. Mỗi khách dùng tài khoản eSMS và brandname **của riêng mình**.

> **Thử nghiệm.** Bộ nối này viết theo tài liệu công khai của nhà cung cấp
> (<https://developers.esms.vn>, đọc ngày 2026-11-21), chưa kiểm với dịch vụ thật. Hãy thử bằng môi
> trường test của eSMS (bên dưới) trước khi dùng thật.

Tiếng Anh: [`esms.md`](esms.md).

## Làm gì, và không làm gì

| | |
| --- | --- |
| Mã một lần (xác thực số điện thoại, đăng nhập, mã đơn hàng và đặt lịch) | gửi bằng tin chăm sóc khách hàng của eSMS (`SmsType` 2) |
| Tin giao dịch của module `notifications` | gửi cùng cách |
| Tin quảng cáo | **bị từ chối** với `esms.marketing_unsupported` |
| Số ngoài Việt Nam (không phải `+84`) | bị từ chối với `esms.phone_unsupported` |
| Báo cáo tới máy (tin đã tới điện thoại chưa) | không đọc; tin được nhận nghĩa là eSMS đã nhận |

SMS quảng cáo ở Việt Nam cần người nhận đồng ý trước, danh sách không quảng cáo, khung giờ được phép
và brandname quảng cáo (Nghị định 91/2020/NĐ-CP; cần đối chiếu văn bản gốc hoặc hỏi người có chuyên
môn pháp lý). eSMS có hàm gửi hàng loạt riêng cho loại này, mỗi request ít nhất 30 số. Bộ nối không
gửi quảng cáo qua đường chăm sóc khách hàng.

## 1. Người vận hành: bật module

Module ở hạng thử nghiệm nên tắt cho tới khi được nêu tên:

```
AUTOXIDE_MODULES=official,esms
```

`AUTOXIDE_ESMS_API_URL` (mặc định `https://rest.esms.vn`) chỉ cần đổi khi trỏ về máy chủ thử. Không
có tài khoản eSMS chung cho cả hệ thống.

Khi module bật, mỗi khách gửi SMS qua tài khoản eSMS của mình; khách chưa đặt tài khoản nhận lỗi
`provider.not_configured` ở chỗ lẽ ra gửi SMS.

## 2. Khách: lấy gì ở eSMS

1. Đăng ký ở [esms.vn](https://esms.vn) và đăng nhập.
2. Mở **Quản lý API**, chép **ApiKey** và **SecretKey**.
3. Nhờ eSMS đăng ký **brandname** cho tin chăm sóc khách hàng, và các **mẫu tin** bạn sẽ gửi. Nhà
   mạng chỉ chuyển tin chăm sóc khách hàng khớp mẫu đã đăng ký; tin khác bị từ chối
   (`esms.template_rejected`).
4. Nếu eSMS giới hạn tài khoản theo địa chỉ IP, đưa cho họ IP của máy chủ Autoxide (không thì:
   `esms.ip_not_allowed`).

Mẫu cần đăng ký cho các mã Autoxide gửi hiện nay (thay phần trong ngoặc nhọn bằng cú pháp biến eSMS
yêu cầu):

| Dùng cho | Nội dung |
| --- | --- |
| Xác thực số điện thoại | `Ma xac thuc cua ban: {code}. Het han sau {minutes} phut. Khong chia se ma nay.` |
| Đăng nhập bằng số điện thoại | `Ma dang nhap cua ban: {code}. Het han sau {minutes} phut. Khong chia se ma nay.` |
| Mã đặt lịch (`bookings`) | `Your booking code is {code}. It expires in {minutes} minutes.` |
| Mã đơn hàng (`orders`) và thông báo (`notifications`) | nội dung các mẫu của chính bạn trong hai module đó |

## 3. Khách: nhập tài khoản

Mọi lời gọi cần permission `kernel.settings.manage`. Secret chỉ đặt qua REST; được lưu mã hoá và
không bao giờ trả lại.

```bash
curl -X PUT https://your-server/api/v1/kernel/settings/esms.connection.sms.api_key \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"value": "YOUR_API_KEY"}'
```

| Cấu hình | Bí mật | Giá trị |
| --- | --- | --- |
| `esms.connection.sms.api_key` | có | ApiKey |
| `esms.connection.sms.secret_key` | có | SecretKey |
| `esms.connection.sms.brandname` | không | brandname đã đăng ký |
| `esms.connection.sms.environment` | không | `sandbox` để dùng môi trường test của eSMS: không tin nào về máy, không trừ tiền. Không đặt hoặc `production`: gửi thật. |

`GET /api/v1/kernel/settings?group=connection` cho biết trường nào đã đặt, kết nối được dùng lần cuối
khi nào và lỗi cuối của nó.

## 4. Thử kết nối

```bash
curl -X POST https://your-server/api/v1/kernel/settings/esms.connection.sms.api_key/test \
  -H "Authorization: Bearer $TOKEN"
```

Lời gọi này hỏi eSMS số dư tài khoản (số dư không được trả ra). `"ok": true` nghĩa là ApiKey và
SecretKey được nhận. Nó không kiểm brandname hay mẫu tin: muốn kiểm, đặt
`esms.connection.sms.environment` thành `sandbox`, cho gửi một mã, rồi đọc lỗi cuối của kết nối.

eSMS không có địa chỉ sandbox hay tài khoản sandbox riêng: môi trường test là cùng API và cùng
ApiKey / SecretKey, kèm cờ `Sandbox` mà bộ nối thêm vào từng request. Đặt lại cấu hình thành
`production` (hoặc bỏ nó) để gửi thật.

## Webhook

Không có. Bộ nối không dùng callback trạng thái của eSMS (callback không có chữ ký), nên không có địa
chỉ nào cần dán vào eSMS.

## Lỗi

| Code | Nghĩa | Thử lại |
| --- | --- | --- |
| `provider.not_configured` | khách chưa đặt cấu hình được nêu tên | sau khi đặt |
| `esms.credentials_rejected` | sai ApiKey / SecretKey (eSMS 101) | sau khi sửa |
| `esms.insufficient_balance` | không đủ số dư (eSMS 103) | được, sau khi nạp |
| `esms.brandname_rejected` | brandname không có hoặc chưa kích hoạt (eSMS 104) | sau khi sửa |
| `esms.template_rejected` | nội dung không khớp mẫu đã đăng ký (eSMS 146) | sau khi đăng ký |
| `esms.carrier_not_registered` | brandname chưa đăng ký với nhà mạng đó (eSMS 177) | sau khi đăng ký |
| `esms.phone_rejected` | số sai định dạng hoặc bị chặn (eSMS 99, 108) | không |
| `esms.content_blocked` | nội dung có từ khoá bị chặn (eSMS 201) | không |
| `esms.ip_not_allowed` | IP máy chủ chưa nằm trong danh sách cho phép của eSMS (eSMS 140) | sau khi eSMS thêm |
| `esms.account_inactive` | tài khoản chưa kích hoạt hoặc bị khoá (eSMS 143) | sau khi eSMS mở |
| `esms.request_rejected` | mã eSMS khác; câu lỗi nêu mã đó | không |
| `esms.unavailable` | eSMS không trả lời hoặc trả lỗi máy chủ | được |
| `esms.marketing_unsupported`, `esms.phone_unsupported` | xem bảng đầu trang | không |

Một tin gửi lại với cùng mã tham chiếu không về máy hai lần: mã đó được truyền làm `RequestId` của
eSMS, và eSMS từ chối lặp lại trong 24 giờ.

Số điện thoại và nội dung tin không bao giờ được ghi vào log.
