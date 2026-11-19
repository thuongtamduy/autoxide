## D-20261118-step43-legal-data · Khuyến mại: hạn mức pháp luật là dữ liệu, chặn khi chưa thông báo — 2026-11-18 — agent quyết

Chủ đề: Pháp luật và tuân thủ

Quyết định:

1. Trần giảm giá, giới hạn số ngày, quy tắc thông báo nằm ở bảng mặt điều khiển `promotions_legal_limits`,
   `promotions_notice_rules` (có `effective_from` / `effective_to`, căn cứ, nguồn, ghi chú "cần đối chiếu"), nạp bằng
   `autoxide task promotions.load_legal_data` từ gói `modules/promotions/data/vn/`. Không có hạn mức hiệu lực thì
   `activate` báo `promotions.no_legal_data`, không tự cho qua; `autoxide doctor` cảnh báo. Lúc tính giỏ mà dữ liệu luật
   đã bị xoá thì không cắt trần (chương trình đã qua kiểm khi bật).
2. Gói "vn" (đối chiếu ngày 13/11/2026 qua nguồn tổng hợp — cần đối chiếu văn bản gốc / người có chuyên môn pháp lý,
   D-186): `price_reduction` thường 50% giá ngay trước khuyến mại (NĐ 81/2018 Đ7; TT 39/2025/TT-BCT), 45 ngày / chương
   trình và 90 ngày / năm tới 30/11/2024, 120 ngày / năm từ 01/12/2024 (NĐ 128/2024); chương trình tập trung quốc gia
   100%; `gift_value` 50% (NĐ 81/2018 Đ6); `free_shipping` 100%, chưa rõ có tính vào trần — ghi chú cần đối chiếu.
3. **Chặn `activate`** khi quy tắc nói phải thông báo mà chưa ghi nhận đã nộp cho từng nơi nhận
   (`promotions.notice_required`); miễn phí giao mặc định cũng phải thông báo (cách an toàn, D-225). Nơi nhận
   `province_of_store` lấy từ cài đặt kế thừa `promotions.notice_province` của nút (chưa đặt → `province:unknown`, vẫn
   ghi nhận được).
4. Số ngày giảm giá trong năm đếm theo ngày giờ Việt Nam, cộng các chương trình giảm giá cùng nút, chung mục tiêu, chưa
   huỷ.

Lý do: luật số hai — quy định hay đổi là dữ liệu có ngày hiệu lực; NĐ 239/2026 giao Bộ Công Thương quy định hạn mức nên
số sẽ đổi mà không cần sửa code.

Đã cân nhắc: chỉ cảnh báo khi chưa thông báo (dễ chạy chương trình trái luật); viết cứng 50% (trái luật số hai).
