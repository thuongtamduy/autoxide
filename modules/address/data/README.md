# Danh mục đơn vị hành chính

`vn-2025/`: danh mục Việt Nam hiệu lực từ 01/7/2025 (QĐ 19/2025/QĐ-TTg) — 34 tỉnh / thành (28 tỉnh, 6 thành phố
trực thuộc trung ương), 3.321 đơn vị cấp xã (2.621 xã, 687 phường, 13 đặc khu). Lấy từ
`https://provinces.open-api.vn/api/v2/?depth=2` ngày 16/11/2026 (bản đăng lại danh mục của cơ quan thống kê; trang
gốc không truy cập được khi lấy), chuyển sang `units.csv` (`level,code,name,kind,parent_code`, mã giữ số 0 đầu) — D-318.
Lệnh `autoxide task address.import` không nêu `path` nhập bộ này; test `du_lieu_goc_du_34_tinh_3321_xa` kiểm số lượng.
