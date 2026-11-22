## D-20261121-sepay-einvoice · Bộ nối SePay eInvoice sau `EInvoiceProvider`: gửi và phát hành ngay, hỏi trạng thái theo mã tracking — 2026-11-21 — agent quyết

Chủ đề: Pháp luật và tuân thủ

Quyết định (bước 70 P7, "Hoá đơn điện tử: SePay eInvoice"; theo D-20261121-vn-providers và D-20261121-provider-accounts):

1. Module `einvoice_sepay` (crate `autoxide-einvoice-sepay`, hạng thử nghiệm) cài `EInvoiceProvider` với tên nhà cung
   cấp `sepay`. Tên module là `einvoice_sepay` chứ không phải `sepay_einvoice`: kernel từ chối hai module mà tên này
   là tiền tố của tên kia (tiền tố bảng sẽ lẫn với module `sepay`). Tài khoản là của từng khách:
   `einvoice_sepay.connection.api.client_id`, `…client_secret` (bí mật), `…provider_account_id` (không đặt thì chỉ tự
   lấy khi tài khoản SePay có đúng một tài khoản nhà cung cấp đang hoạt động). Access token (`POST /v1/token`, Basic
   Authentication, sống 24 giờ) giữ trong bộ nhớ theo từng khách.
2. **Gửi**: `POST /v1/invoices/create` với `is_draft: false` (xuất và phát hành luôn); hoá đơn thay thế →
   `/v1/invoices/replace`, điều chỉnh → `/v1/invoices/adjust` (`adjustment_type` theo dấu tiền của các dòng).
   `reference_code` = ID chứng từ: SePay chặn mã đã có hoá đơn (`409 EINVOICE_DOCUMENT_EXISTED`) nên gửi lại không tạo
   hoá đơn thứ hai — bộ nối coi 409 là "đã gửi" và hỏi trạng thái theo mã tham chiếu. Mẫu `1` cho hoá đơn GTGT, `2` cho
   hoá đơn bán hàng; **ký hiệu lấy từ SePay** (API chi tiết tài khoản), không dùng ký hiệu nội bộ.
3. **Trạng thái**: `GET /v1/invoices/create/check/{tracking_code}` — `Pending` thì chờ (SePay tự thử lại, không gửi lại),
   `Failed` thì từ chối kèm `message`, `Success` thì đọc chi tiết hoá đơn: có mã cơ quan thuế / đã tiếp nhận → chấp
   nhận; lỗi ký hay không đủ điều kiện cấp mã → từ chối; cơ quan thuế chưa phản hồi → chờ.
4. **Giới hạn ghi rõ**: chỉ đồng Việt Nam; chỉ hoá đơn GTGT và hoá đơn bán hàng; thuế suất 0 / 5 / 8 / 10 % và không
   chịu thuế; không có huỷ hoá đơn (tài liệu SePay không có API huỷ hoá đơn đã phát hành — xử lý sai sót bằng thay thế
   / điều chỉnh); **số hoá đơn do nhà cung cấp cấp** chỉ có sau khi phát hành — trả về qua
   `EInvoiceStatus::Accepted::number` và lưu ở `provider_number` của chứng từ (D-20261122-einvoice-provider-number); số
   nội bộ của chứng từ có thể khác.
5. Địa chỉ API là cấu hình của người vận hành: `AUTOXIDE_EINVOICE_SEPAY_API_URL` (mặc định
   `https://einvoice-api.sepay.vn`) và `AUTOXIDE_EINVOICE_SEPAY_SANDBOX_API_URL` (mặc định
   `https://einvoice-api-sandbox.sepay.vn`); khách chọn bằng `einvoice_sepay.connection.api.environment`
   (D-20261122-provider-environments).

Căn cứ pháp lý (cần đối chiếu văn bản gốc / người có chuyên môn pháp lý):

- Luật Quản lý thuế số 38/2019/QH14, Điều 89–92 (hoá đơn điện tử), hiệu lực 01/7/2020; quy định về hoá đơn điện tử áp
  dụng bắt buộc từ 01/7/2022.
- Nghị định 123/2020/NĐ-CP ngày 19/10/2020 về hoá đơn, chứng từ (hiệu lực 01/7/2022), được sửa đổi, bổ sung bởi Nghị
  định 70/2025/NĐ-CP ngày 20/3/2025 (hiệu lực 01/6/2025): nội dung hoá đơn (Điều 10), thời điểm lập (Điều 9), xử lý
  hoá đơn có sai sót bằng điều chỉnh / thay thế (Điều 19).
- Thông tư 78/2021/TT-BTC ngày 17/9/2021 hướng dẫn Nghị định 123/2020 (ký hiệu mẫu số, ký hiệu hoá đơn); Thông tư
  32/2025/TT-BTC ngày 31/5/2025 (hiệu lực 01/6/2025) hướng dẫn sau Nghị định 70/2025 — cần đối chiếu phần nào của
  Thông tư 78 còn hiệu lực.
- Người bán chịu trách nhiệm về nội dung hoá đơn; nhà cung cấp dịch vụ hoá đơn điện tử ký số và truyền tới cơ quan thuế.
  Autoxide chỉ chuyển dữ liệu chứng từ đã lập, không tự sửa số tiền hay thuế.

Nguồn kỹ thuật (đọc ngày 2026-11-21): https://developer.sepay.vn/vi/einvoice-api/v1/tong-quan và các trang con (token,
tài khoản nhà cung cấp, xuất hoá đơn, thay thế, điều chỉnh, theo dõi trạng thái, chi tiết, tải, xử lý lỗi) — liệt kê ở
đầu `modules/einvoice_sepay/src/lib.rs`. Chưa kiểm với dịch vụ thật.

Lý do: API của SePay xử lý bất đồng bộ (trả mã tracking, tự thử lại tới khoảng 2 giờ) — khớp với luồng "gửi rồi hỏi
trạng thái" của `invoices`; mã tham chiếu duy nhất do bên gửi đặt cho phép gửi lại an toàn.

Đã cân nhắc: tạo nháp rồi phát hành ở bước hai (thêm một vòng hỏi trạng thái mà không thêm an toàn — `invoices` chỉ gửi
chứng từ đã phát hành nội bộ); dùng ký hiệu nội bộ của chứng từ (SePay từ chối ký hiệu không thuộc tài khoản hoặc sai
năm); tự huỷ bằng cách xoá nháp (chỉ áp cho hoá đơn nháp, không phải hoá đơn đã phát hành).
