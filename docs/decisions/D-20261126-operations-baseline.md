## D-20261126-operations-baseline · Vận hành và cam kết nền trước 1.0: phiên bản nền tảng, sổ tay vận hành, thử hỏng, mở rộng hợp đồng, đường chuyển 0.x — 2026-11-26 — chủ dự án quyết

Chủ đề: Phạm vi và mục tiêu

Quyết định: thêm nhóm G14 "Vận hành và cam kết nền" vào bước 71 và vào điều kiện ra 1.0 (D-20261126-road-to-1-0):

1. Cam kết nền tảng: chính sách bản Postgres, bản SQLite tối thiểu, hệ điều hành và kiến trúc CPU; CI chạy ma trận bản
   Postgres; `doctor` cảnh báo bản ngoài danh sách.
2. Bộ đồ nghề vận hành: sổ tay xử lý sự cố (EN + VI), mẫu bảng theo dõi và luật cảnh báo, quy trình ứng phó lỗ hổng bảo mật
   có xin mã CVE.
3. Thử hỏng có chủ đích trong CI trên cả hai database (mất kết nối, đầy ổ đĩa, chuyển máy dự phòng, tiến trình bị giết giữa
   chừng): tự hồi phục, không mất hay nhân đôi event / job, không hỏng dữ liệu.
4. Mở rộng danh sách hợp đồng được cam kết ở D-20261126-release-1-0-readiness: CLI (tên lệnh, tham số, mã thoát) và định
   dạng file sao lưu, file xuất dữ liệu (có số phiên bản định dạng; bản 1.x đọc được file của mọi bản 1.y trước đó).
5. Đường chuyển 0.x → 1.0 cho người viết module: hướng dẫn từng bước và công cụ tự sửa code cũ, dùng chính nó cho module
   trong repo.

Ngoài năm mục này không thêm phạm vi 1.0 mới; việc phát hiện thêm trong lúc làm thì sửa trong mục đang làm hay ghi sau 1.0.

Lý do: chủ dự án hỏi còn gì cần trước 1.0 ngoài G1–G13. Rà lại lộ trình thấy năm chỗ hở mà người dùng doanh nghiệp và
người vận hành cần ngay khi quyết định dùng: biết chạy trên bản nào, xử lý sự cố ra sao, hệ thống chịu hỏng thế nào, script
và file sao lưu của họ có còn dùng được ở bản sau không, và người đã viết module theo kiểu cũ chuyển sang 1.0 bằng cách nào.

Đã cân nhắc: để sổ tay vận hành và thử hỏng sau 1.0 (người dùng đầu tiên gặp sự cố mà không có hướng dẫn, và lỗi hồi phục
lộ ra trên dữ liệu thật); chỉ viết hướng dẫn chuyển mà không có công cụ tự sửa (khoảng 6.000 chỗ gọi trong repo và mọi
module ngoài phải sửa tay).
