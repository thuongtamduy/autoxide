# Xuất hoá đơn điện tử qua SePay eInvoice

Với bộ nối `einvoice_sepay`, hoá đơn phát hành trong Autoxide được gửi tới cơ quan thuế qua
[SePay eInvoice](https://developer.sepay.vn/vi/einvoice-api/v1/tong-quan), và kết quả (được chấp nhận, kèm
mã của cơ quan thuế, hay bị từ chối, kèm lý do) được ghi lại lên chứng từ.

Tiếng Anh: [`sepay-einvoice.md`](sepay-einvoice.md).

> **Thử nghiệm.** Bộ nối này viết theo tài liệu công khai của nhà cung cấp, chưa kiểm với dịch vụ thật
> (written from the provider's public documentation; not yet verified against the live service). Hãy thử
> với sandbox của SePay trước khi dựa vào nó. Bạn vẫn chịu trách nhiệm về nội dung hoá đơn của mình; hướng
> dẫn này không phải tư vấn pháp lý.

Mỗi khách (tenant) nối tài khoản SePay eInvoice **của riêng mình**. Client secret được lưu mã hoá và
không bao giờ hiện lại.

## 1. Lưu tài khoản SePay eInvoice của bạn

Cần permission `kernel.settings.manage`.

```bash
for field in client_id client_secret provider_account_id; do
  read -r -p "$field: " value
  curl -X PUT "$API/api/v1/kernel/settings/einvoice_sepay.connection.api.$field" \
    -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
    -d "{\"value\": \"$value\"}"
done

# Test it: gets a token and reads the provider accounts
curl -X POST "$API/api/v1/kernel/settings/einvoice_sepay.connection.api.client_id/test" \
  -H "Authorization: Bearer $TOKEN"
```

| Cấu hình | Là gì |
| --- | --- |
| `einvoice_sepay.connection.api.client_id` | Client id của tài khoản SePay eInvoice của bạn. |
| `einvoice_sepay.connection.api.client_secret` | Client secret của nó. |
| `einvoice_sepay.connection.api.provider_account_id` | Tài khoản nhà cung cấp hoá đơn dùng để phát hành. Có thể để trống khi tài khoản SePay của bạn có đúng một tài khoản nhà cung cấp đang hoạt động. |

## 2. Trỏ một ký hiệu hoá đơn vào SePay

Đặt `einvoice_provider` của ký hiệu chứng từ thành `sepay`. Chứng từ thuộc ký hiệu đó được gửi khi phát
hành, và được hỏi trạng thái cho tới khi cơ quan thuế trả lời.

## Gửi gì, không gửi gì

- Hoá đơn giá trị gia tăng (mẫu `1` của SePay) và hoá đơn bán hàng (mẫu `2`), bằng đồng Việt Nam.
- Ký hiệu hoá đơn lấy từ tài khoản SePay của bạn; ký hiệu của chuỗi chứng từ trong Autoxide không được dùng.
- Thuế suất 0, 5, 8 và 10 %, và "không chịu thuế". Thuế suất khác bị từ chối trước khi gửi.
- Hoá đơn sửa sai được gửi dưới dạng thay thế hoặc điều chỉnh hoá đơn gốc.
- Gửi lại cùng một chứng từ không bao giờ tạo hoá đơn thứ hai: SePay nhận ra nó theo ID chứng từ.
- **Số hoá đơn do nhà cung cấp cấp.** Số này nằm trên file PDF và XML bạn tải về; số của chứng từ trong
  Autoxide có thể khác.
- **Không có huỷ hoá đơn** qua API của SePay: sửa hoá đơn bằng thay thế hoặc điều chỉnh.
- Người vận hành dùng sandbox của SePay bằng
  `AUTOXIDE_EINVOICE_SEPAY_API_URL=https://einvoice-api-sandbox.sepay.vn`.
