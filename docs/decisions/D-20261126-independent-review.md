## D-20261126-independent-review · Kiểm độc lập bắt buộc cho thay đổi chạm kernel, bảo mật, tiền và thuế — 2026-11-26 — chủ dự án quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. Pull request chạm `crates/kernel`, xác thực / phân quyền / đa khách, tiền, thuế, chứng từ hay migration xoá dữ liệu
   phải có một lượt kiểm độc lập trước khi merge: người kiểm (agent khác, không phải tác giả) chỉ đọc code, nhận hợp đồng
   và bất biến cần giữ chứ không nhận kết luận của tác giả, cố dựng phản ví dụ; lỗi chặn được sửa và kiểm lại phần thay đổi.
2. Mô tả pull request nêu kết quả lượt kiểm (đạt / còn lỗi chặn) và các điểm để sau.
3. Hỗ trợ bằng công cụ (mục 71-G9): mutation testing cho kernel và phần tiền/thuế; bất biến tiền, thuế, quyền thành
   property test chạy hằng đêm.

Lý do: ngày 26/11/2026, gần như mọi pull request lớn đều bị lượt kiểm độc lập bắt lỗi thật trước khi merge (thuế âm một
đồng ở ba biến thể, quét toàn bảng không trần, bỏ qua bước xác thực lại, test không thể đỏ, cấp quyền hai lớp lặp). Cách
kiểm này đang chỉ có khi phiên điều phối chủ động gọi; đưa thành luật để không phụ thuộc trí nhớ của ai.

Đã cân nhắc: chỉ dựa vào CI (CI không bắt lỗi thiết kế và test sai); kiểm độc lập mọi pull request (tốn hạn mức cho thay
đổi tài liệu và việc nhỏ không chạm phần rủi ro).
