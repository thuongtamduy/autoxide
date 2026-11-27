## D-20261126-einvoice-no-cancel · Hoá đơn điện tử đã gửi cơ quan thuế: luật vùng thuế quyết có huỷ được không — 2026-11-26 — agent quyết

Chủ đề: Tiền, thuế, chứng từ

Quyết định: Có được huỷ hoá đơn điện tử đã gửi cơ quan thuế hay không là **dữ liệu của gói thuế** (quy tắc
`einvoice.cancel_after_submission`, `{"allowed": bool}`, có ngày hiệu lực), không viết cứng. Gói `vn` v3: `allowed: false`
từ 01/6/2025. `invoices` hỏi `TaxEngine::einvoice_cancel_forbidden` (cài đặt mặc định: không cấm — D-038) trước khi huỷ
chứng từ thuộc dãy có nhà cung cấp hoá đơn điện tử; bị cấm thì `409 invoices.einvoice_not_cancellable` kèm căn cứ, hướng
sang điều chỉnh giảm hay thay thế, **không gọi nhà cung cấp**. Hoá đơn bị cơ quan thuế từ chối (chưa có hoá đơn hợp lệ)
huỷ tại chỗ, không báo nhà cung cấp. Core không gửi thông báo sai sót mẫu 04/SS: tài liệu chỉ người dùng nộp ở cổng của
nhà cung cấp hay cơ quan thuế khi cần.

Căn cứ (rà pháp lý vòng 1, LG-9): NĐ 123/2020/NĐ-CP Điều 19 sửa bởi NĐ 70/2025/NĐ-CP (khoản 13 Điều 1, hiệu lực
01/6/2025): hoá đơn có sai sót xử lý bằng điều chỉnh hay thay thế; quy định huỷ hoá đơn có mã chưa gửi người mua bằng mẫu
04/SS bị bãi bỏ; 04/SS còn dùng để thông báo một số sai sót (vd sai tên, địa chỉ người mua)
([MISA](https://amis.misa.vn/?p=80258), [einvoice.vn](https://einvoice.vn/tin-tuc/huong-dan-xu-ly-hoa-don-dien-tu-co-sai-sot-theo-nghi-dinh-70-2025-nd-cp)).
Nguồn thứ cấp — **cần đối chiếu văn bản gốc**: các trường hợp còn được huỷ (vd huỷ giao dịch) và khi nào bắt buộc 04/SS.

Sau kiểm độc lập: quy tắc tra theo **ngày thuế lúc huỷ** (`TaxEngine::tax_date(người bán, bây giờ)`), không theo ngày lập
— chặt hơn: hoá đơn lập trước 01/6/2025 cũng không huỷ được sau mốc (**cần đối chiếu quy định chuyển tiếp của NĐ
70/2025**). Trạng thái được so lại trong transaction đã khoá; đổi giữa chừng thì `invoices.not_cancellable`, gọi lại.
`plans` huỷ kỳ thu phí gặp `invoices.einvoice_not_cancellable` thì lập điều chỉnh giảm toàn bộ (`reference`
`plans:<kỳ>:void`, chạy lại không lập hai lần) rồi phát hành. Hoá đơn điều chỉnh, thay thế (cả khi sửa nháp) tính thuế
theo ngày của hoá đơn gốc.

Kiểm độc lập #575 (vòng 3): quy tắc cấm huỷ áp cho **mọi** trường hợp. NĐ 70/2025 có thể vẫn cho huỷ khi hoá đơn lập lúc
thu tiền trước rồi dịch vụ bị huỷ — đúng ca huỷ kỳ thuê bao của `plans`: **cần đối chiếu văn bản gốc / người có chuyên
môn pháp lý**. Không đổi hành vi: điều chỉnh giảm toàn bộ là cách an toàn đang dùng (đúng luật trong mọi cách đọc).
Nhà cung cấp không có lệnh huỷ (SePay eInvoice — gặp khi người bán chưa có hồ sơ thuế nên quy tắc chưa áp) trả
`409 invoices.einvoice_cancel_unsupported`, cùng hướng điều chỉnh / thay thế; `plans` xử lý như `einvoice_not_cancellable`.

Lý do: trước đây `invoices` gọi `cancel` của nhà cung cấp, SePay luôn trả `not_supported`, người dùng nhận lỗi khó hiểu
và không biết phải làm gì. Từ chối sớm với căn cứ và hướng đi đúng luật thì rõ hơn, và đóng luôn khe hở huỷ khi job gửi
hoá đơn chưa chạy (trạng thái `issued`).

Đã cân nhắc: để nhà cung cấp tự từ chối — mỗi nhà cung cấp một thông báo, không có căn cứ; viết cứng "Việt Nam không cho
huỷ" trong `invoices` — trái Luật số hai (quy định là dữ liệu có ngày hiệu lực); làm 04/SS qua nhà cung cấp ngay — SePay
chưa có API được tra, và chưa rõ trường hợp nào bắt buộc.
