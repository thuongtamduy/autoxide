## D-20261121-esms-adapter · Bộ nối eSMS: chỉ gửi mã một lần và tin chăm sóc khách hàng, từ chối tin quảng cáo, chưa nhận callback — 2026-11-21 — agent quyết

Chủ đề: Thương mại

Quyết định (bước 70 P7, theo D-20261121-vn-providers và D-20261121-provider-accounts):

1. Module `esms` (hạng thử nghiệm, không bảng, không route, không tool MCP) cài `SmsSender`. Viết theo tài liệu công
   khai của eSMS, đọc ngày 2026-11-21, chưa kiểm với dịch vụ thật: `developers.esms.vn` các trang
   `readme/co-che-hoat-dong`, `esms-api/ham-gui-tin/tin-nhan-sms-otp-cskh`, `esms-api/ham-gui-tin/tin-sms-quang-cao`,
   `esms-api/ham-truy-xuat-va-dang-ky/ham-lay-so-du-tai-khoan`, `esms-api/bang-ma-loi`,
   `esms-api/moi-truong-test-sandbox`, `esms-api/callback-url`.
2. **Chọn tài khoản theo khách mà không đổi trait của core:** `SmsMessage::tenant` có sẵn, module công bố một
   `dyn SmsSender` và mỗi lần gửi đọc `esms.connection.sms.{api_key, secret_key, brandname, sandbox}` của đúng khách.
   Chưa đặt → `provider.not_configured`.
3. `SmsPurpose::Otp` và `Transactional` → hàm `SendMultipleMessage_V4_post_json`, `SmsType` 2 (tin chăm sóc khách
   hàng). `reference` → `RequestId` (eSMS chặn trùng 24 giờ; mã 124 "trùng RequestId" coi là tin đã được nhận). Chỉ
   nhận số `+84`, gửi dạng nội địa `0…` như ví dụ của tài liệu.
4. **`SmsPurpose::Marketing` bị từ chối** (`esms.marketing_unsupported`). Căn cứ: Nghị định 91/2020/NĐ-CP về chống
   tin nhắn rác, thư điện tử rác, cuộc gọi rác (hiệu lực 01/10/2020) — tin quảng cáo chỉ gửi khi người nhận đã đồng
   ý trước, không gửi tới số trong Danh sách không quảng cáo, trong khung giờ cho phép, có nhãn quảng cáo, qua tên
   định danh đã đăng ký. **Cần đối chiếu văn bản gốc / người có chuyên môn pháp lý** về điều khoản và khung giờ cụ
   thể. Hàm quảng cáo của eSMS (`SendMultipleSMSBrandname_json`, `SmsType` 1) đòi ít nhất 30 số mỗi request và chờ
   duyệt, không khớp hợp đồng một tin một số; gửi quảng cáo qua đường chăm sóc khách hàng là né phân loại của nhà
   mạng và của nghị định nên không làm.
5. **Chưa nhận callback trạng thái:** eSMS gọi lại bằng HTTPS GET không có chữ ký (chỉ công bố ba IP nguồn), URL
   mang số điện thoại; hướng dẫn bộ nối cấm nhận thông báo không xác thực, và core chưa có hợp đồng "tin đã tới
   máy" cho SMS. Bộ nối không truyền `CallbackUrl`.
6. Thử kết nối bằng `GetBalance_json` (key trong thân). Không dùng các hàm GET của eSMS vì chúng đặt ApiKey /
   SecretKey trong URL.
7. Mã lỗi của eSMS ánh xạ sang `code` ổn định `esms.*` (bảng ở `docs/guides/providers/esms.md`); câu `ErrorMessage`
   của eSMS không được chép ra. Thiếu số dư (103), HTTP 5xx, không trả lời → lỗi tạm thời (thử lại được); còn lại
   là lỗi vĩnh viễn.

Lý do: giữ đúng mục 1 của D-20261121-vn-providers (core không đổi hợp đồng vì một nhà cung cấp) và luật số hai (không
làm đường tắt quanh quy định về tin quảng cáo). Tin chăm sóc khách hàng phải khớp mẫu đã đăng ký với nhà mạng nên
hướng dẫn liệt kê các câu mã một lần mà core gửi.

Đã cân nhắc: gửi tin quảng cáo từng số qua hàm chăm sóc khách hàng (né luật, và eSMS sẽ từ chối vì sai mẫu); gom tin
quảng cáo thành lô 30 số (cần hợp đồng gửi hàng loạt mà `SmsSender` không có); nhận callback rồi hỏi lại
`GetSendStatus` để xác thực (làm được khi core có hợp đồng báo trạng thái gửi, nhưng hàm đó đặt key trong URL); cho
mỗi khách chọn nhà cung cấp SMS (chưa cần khi mới có một bộ nối; khi có bộ nối thứ hai thì core cần bộ chọn).
