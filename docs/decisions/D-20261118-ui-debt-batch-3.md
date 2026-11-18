## D-20261118-ui-debt-batch-3 · Trả nợ giao diện đợt 3: công cụ kế toán, tổng quan tuân thủ, hồ sơ định danh, sổ loại đối tượng — 2026-11-18 — agent quyết

Chủ đề: Hợp đồng cho giao diện

Quyết định: Tiếp D-382, theo lệnh chủ dự án "trả nợ tiếp", thêm các API dưới đây trên module đã xong. Chỉ thêm, không phá
vỡ (D-059).

1. **`ledger`:** `POST /bank-accounts/{id}/vietqr-preview` (`ledger.read`) dựng chuỗi VietQR mẫu để quét thử, không ghi gì
   (nội dung mặc định là một mã chuyển khoản mẫu cùng dạng mã thật). Đường dẫn gắn vào tài khoản thay cho
   `/ledger/vietqr/preview` của đề bài: mẫu luôn là của một tài khoản. `GET /books/{id}/integrity` kiểm lại cả chuỗi
   băm của sổ trên một bức ảnh nhất quán (cùng hàm của `autoxide doctor`); chỉ đọc, tốn tỉ lệ số bút toán. Khoá kỳ đã tự
   kiểm (`ledger.period_order`, `ledger.period_not_ended`) nên không thêm bước "kiểm trước khi khoá" riêng.
2. **`invoices`:** `POST /documents/{id}/refresh` (`invoices.issue`) hỏi nhà cung cấp ngay, như lịch hỏi định kỳ; chứng
   từ không ở `submitting` trả nguyên — không thêm mã `invoices.document_state`. `GET /vat-summary` cộng thuế GTGT
   **đầu ra** theo loại chứng từ, tiền tệ, thuế suất từ dòng của chứng từ đã có số trong kỳ (`issued`, `submitting`,
   `accepted`, `adjusted`; điều chỉnh giảm trừ đi; bị thay, huỷ, bị từ chối thì bỏ) — số tham khảo để đối chiếu tờ
   khai, **cần người có chuyên môn kế toán / thuế đối chiếu cách tính**. Đầu vào chờ chứng từ mua (bước 42).
3. **`compliance`:** `GET /overview` — số việc chờ của năm khu vực, mỗi khu vực chỉ điền khi người gọi có quyền đọc khu
   vực đó (còn lại `null`), mỗi số một câu `COUNT`. Hồ sơ định danh thêm `subject_name` (tên tài khoản / tên nút trước
   khi xác minh, qua sổ đối tượng `auth.user`, `organizations.unit` — không đọc bảng module khác; loại chưa có bộ tra
   thì bỏ qua), `missing_documents` (`draft` / `needs_info` và chưa gửi giấy tờ), `review_note` (từ chối / thu hồi nhận
   `note` ≤ 1000 ký tự; duyệt thì xoá). Migration `m0019_kyc_review_note`.
4. **Sổ đối tượng:** `organizations` đăng ký loại `organizations.unit` (tên nút, không công khai). Kernel thêm
   `GET /kernel/records/types` (loại kèm `has_children`, chỉ loại của module người gọi có `<module>.read`, như
   `usages`); tìm bản ghi của một loại dùng `GET /kernel/find?q=… is:<loại>` có sẵn.
5. **Bộ giả hoá đơn điện tử của testkit:** kết quả từ chối gắn với lần gửi, hỏi trạng thái không "tiêu" kết quả — trước
   đây hai lần hỏi đồng thời (job gửi và lịch hỏi) làm test `bi_tu_choi_gui_lai_cung_so_roi_duoc_chap_nhan` chập chờn
   trên Postgres.

Còn để sau (ghi trong bảng nợ): xoá dữ liệu cá nhân theo email ở mọi module, số lần tự xuất / tự xoá, hoàn tác thao tác
hàng loạt, sắp bình luận theo cột khác, GTGT đầu vào.

Lý do: các màn quản trị của `autoxide-web` đang dùng dữ liệu mẫu cho những chỗ này (`docs/prompts/web-build-api-gaps.md`).

Đã cân nhắc: đường dẫn `/ledger/vietqr/preview` của đề bài (mẫu luôn của một tài khoản nên gắn vào tài khoản); mã lỗi
`invoices.document_state` cho hỏi lại chứng từ không chờ (trả nguyên là đủ, gọi lại an toàn); bước "kiểm trước khi khoá"
riêng cho kỳ (khoá kỳ đã tự kiểm); tính GTGT đầu vào (chưa có chứng từ mua).
