## D-20261128-subject-request-fulfil · Hạn thực hiện yêu cầu của chủ thể dữ liệu theo ngày thường — 2026-11-28 — agent quyết

Chủ đề: Pháp luật và tuân thủ

Quyết định: Sổ yêu cầu của chủ thể dữ liệu tính `fulfil_due_at` = lúc nhận + `fulfil_within_days[loại]` của quy tắc
`privacy.subject_requests` (phiên bản 3, đơn vị `fulfil_unit`: `days` — ngày thường; `working_days` dùng lịch ngày làm
việc như `cases.deadlines`). Yêu cầu quá hạn khi còn `received` mà qua hạn trả lời (`due_at`), hay chưa xong mà qua hạn
thực hiện. Hạn kéo dài khi có bên xử lý thứ ba chưa áp dụng. Thân quy tắc không phải JSON là lỗi.

Căn cứ: Luật 91/2025/QH15 và NĐ 356/2025/NĐ-CP Điều 5 theo bản phân tích (rút đồng ý / hạn chế / phản đối 15 ngày;
xem, sửa 10 ngày; xoá 20 ngày) — nguồn chưa thống nhất ngày làm việc hay ngày thường và con số khi có bên thứ ba; cần
đối chiếu văn bản gốc / người có chuyên môn pháp lý.

Lý do: ngày thường cho hạn ngắn hơn hoặc bằng ngày làm việc, không kéo dài — hướng an toàn cho chủ thể dữ liệu khi nguồn
chưa rõ; đơn vị là dữ liệu nên văn bản gốc xác nhận thì nhập phiên bản quy tắc mới, không sửa code. Yêu cầu đã nhận xử
lý (`in_progress`) đã đạt hạn trả lời, nên chỉ còn hạn thực hiện.

Đã cân nhắc: tính theo ngày làm việc ngay (hạn dài hơn, sai nếu luật tính ngày thường).
