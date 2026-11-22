## D-20261121-momo-adapter · Bộ nối MoMo: cổng thanh toán ví qua `PaymentGateway`, API v2, mã đơn là ID trong sổ — 2026-11-21 — agent quyết

Chủ đề: Thương mại

Quyết định (bước 70 P7, "Thanh toán: MoMo"; theo D-20261121-vn-providers và D-20261121-provider-accounts):

1. Module `momo` (crate `autoxide-momo`, hạng thử nghiệm) cài **`PaymentGateway`**, phương thức `wallet`, đăng ký trước
   `ledger`. Tiền đi từ người mua vào tài khoản MoMo doanh nghiệp của chính khách (D-187: core không giữ tiền). Dùng
   "Payment Gateway API" **bản v2** của MoMo (`/v2/gateway/api/create`, `/query`, `/refund`), `requestType`
   `captureWallet`, chữ ký HMAC-SHA256 trên chuỗi `key=value` theo thứ tự a-z.
2. **Tài khoản theo khách**: `momo.connection.account.partner_code`, `.access_key` (bí mật), `.secret_key` (bí mật),
   tuỳ chọn `.environment` (`production` mặc định | `sandbox`) và `.return_url`. Địa chỉ API là cấu hình người vận hành
   (`AUTOXIDE_MOMO_API_URL`, `AUTOXIDE_MOMO_SANDBOX_API_URL`); khách chỉ chọn MỘT trong hai địa chỉ đó, không nhập địa
   chỉ tuỳ ý — bộ khoá thử của MoMo chỉ chạy với máy chủ thử, nên chọn môi trường phải theo từng khách.
3. **Mã đơn**: `orderId` = `requestId` = ID thanh toán (hay ID lượt hoàn) trong sổ. `provider_ref` của thanh toán là
   `orderId`, vì MoMo chỉ cấp `transId` sau khi người mua trả. Hoàn tiền cần `transId` nên bộ nối truy vấn giao dịch gốc
   trước rồi mới gửi lệnh hoàn — không thêm bảng nào.
4. **IPN**: MoMo đòi `ipnUrl` trong từng lần tạo giao dịch, nên bộ nối gửi URL webhook riêng của khách
   (`AUTOXIDE_HTTP_PUBLIC_URL` + `/api/v1/ledger/webhooks/momo/{hook}`); khách không phải dán gì ở MoMo. Chữ ký kiểm
   bằng Secret Key của đúng khách, so sánh thời gian hằng, và `partnerCode` phải khớp. Trả HTTP 204. Mã thông báo
   `orderId:transId:resultCode` nên gửi lại không xử lý trùng; không dùng cửa sổ thời gian vì MoMo không ký thời điểm
   gửi. Thông báo mang kết quả chưa cuối (1000, 7000, 7002, 8000, 9000) bị từ chối `momo.notification_not_final`,
   không ghi gì.
5. **Gọi lại an toàn hơn là tiện**: MoMo trả mã 40 / 41 khi `requestId` / `orderId` đã có. Lúc tạo giao dịch →
   `momo.order_already_created` (MoMo không cấp lại link; tạo thanh toán mới). Lúc hoàn → `momo.refund_already_sent`:
   không gửi lệnh mới, không tự coi là xong, để người đối soát xem ở MoMo. Mã 10, 43, 7000, 7002, 1080 →
   `payment.provider_unavailable` (thử lại được); 11, 13 → `momo.account_rejected`; còn lại →
   `payment.provider_rejected` kèm mã của MoMo.
6. Kiểm trước khi gọi: chỉ VND (`momo.currency_not_supported`), 1.000–50.000.000 VND (`momo.amount_out_of_range`), có
   trang quay về (`momo.return_url_required`). Không có sao kê (`payment.not_supported`). Thử kết nối = truy vấn một
   `orderId` không có: MoMo trả mã 42 nghĩa là bộ khoá được nhận.

Nguồn (đọc ngày 2026-11-21) và mức kiểm chứng: `developers.momo.vn` trả HTTP 403 cho môi trường viết bộ nối, nên chỉ
**mã mẫu chính thức** <https://github.com/momo-wallet/payment> (commit `d4746c5`: tạo giao dịch, chuỗi ký của IPN, truy
vấn) được đọc trực tiếp; các trang `/v3/docs/payment/api/wallet/onetime/`, `…/result-handling/notification/`,
`…/payment-api/query/`, `…/payment-api/refund/`, `…/result-handling/resultcode/`, `…/result-handling/idempotency/` và
`/v3/docs/payment/onboarding/test-instructions/` được đọc **gián tiếp** qua trích đoạn của công cụ tìm kiếm. Chưa chạy
với dịch vụ thật. Cần đối chiếu văn bản gốc khi có tài khoản thử: bảng trường trả về của truy vấn (bộ nối chỉ dùng
`resultCode`, `amount`, `transId`); hành vi chính xác khi gửi lại cùng `requestId`; MoMo có gửi IPN cho hoàn tiền
không (bộ nối coi là không, lấy kết quả đồng bộ); trang thanh toán một lần ghi trả IPN bằng 200 kèm JSON còn trang
thông báo ghi 204 (bộ nối theo trang thông báo); giới hạn số tiền của hoàn tiền theo từng phương thức.

Lý do: `orderId` là thứ duy nhất có từ lúc tạo và MoMo chống trùng theo nó, nên dùng làm mã tham chiếu thì không cần
lưu thêm gì và không bao giờ tạo hai giao dịch cho một thanh toán. Với tiền, khi không biết lần gọi trước ra sao thì
báo lỗi rõ cho người xử lý an toàn hơn là đoán "đã xong" theo một câu trả lời chưa đọc được trong tài liệu.

Đã cân nhắc: lưu `transId` vào bảng riêng của bộ nối khi IPN về — loại: thêm bảng và migration cho thứ hỏi lại MoMo
được bằng một lời gọi. Dùng API truy vấn hoàn tiền (`/v2/gateway/api/refund/query`) để tự xử lý mã 40 / 41 — loại cho
tới khi đọc được bảng trường trả về của nó: đoán sai là báo "đã hoàn" cho một lệnh chưa hoàn. Một biến môi trường chọn
sandbox cho cả hệ thống — loại: khách đang thử và khách đang bán thật không ở chung một máy chủ được. `requestType`
`payWithMethod` (ví, thẻ ATM, thẻ quốc tế trên cùng trang) — để sau: mỗi phương thức có giới hạn và mã lỗi riêng chưa
đọc được. Kiểm chữ ký của câu trả lời từ MoMo — không làm: tài liệu đọc được không nêu chuỗi ký của bản v2, và kết nối
là HTTPS tới địa chỉ cố định.
