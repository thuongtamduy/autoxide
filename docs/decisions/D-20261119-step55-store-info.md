## D-20261119-step55-store-info · Bước 55: thông tin cửa hàng công khai — cài đặt `organizations.brand_*`, endpoint `/public/storefront` — 2026-11-19 — agent quyết

Chủ đề: Đa khách, tổ chức, gói

Quyết định — cách làm mục 4 của bước 55 (thông tin cửa hàng), khác đặc tả ở ba chỗ:

1. **Tên cài đặt** theo luật của kernel (`<module>.<tên>`, không dấu chấm thêm): `organizations.brand_name`,
   `organizations.brand_logo`, `organizations.brand_icon`, `organizations.contact_phone`,
   `organizations.contact_email`, `organizations.social_links` — thay cho `storefront.*` của đặc tả. Tên thương hiệu
   một giá trị, không theo ngôn ngữ (tên thương hiệu thường không dịch); cần thì thêm cài đặt sau.
2. **Không thêm endpoint quản trị riêng** `GET/PUT /units/{id}/storefront`: các cài đặt trên sửa và đọc qua
   endpoint cài đặt kế thừa sẵn có (`GET /units/{id}/settings` trả giá trị, nút nguồn, có kế thừa không) — một cách
   làm cho mỗi việc.
3. **Chân trang pháp lý** (thông tin chủ sở hữu, logo Bộ Công Thương) không ghép vào `/public/storefront`:
   `compliance` đăng ký sau `organizations` nên `organizations` không gọi được nó; website gọi thêm
   `GET /api/v1/compliance/public/owner` và `/public/badge` (đã có từ bước 37). Tài liệu endpoint nêu rõ.

Công khai: `GET /api/v1/organizations/public/storefront` (website theo `site` / tên miền) trả thương hiệu đã kế
thừa, hồ sơ địa điểm của bước 34 (không lưu hai chỗ, D-237), múi giờ và "đang mở cửa" (ngày đặc biệt thắng giờ
thường, theo ngày địa phương của nút), `Cache-Control: public, max-age=60`. Logo / biểu tượng là ID ảnh công khai
của `media` (`organizations` đăng ký trước `media`, không gọi được nó).

Lý do: theo luật đặt tên của kernel, không làm hai đường cho cùng một việc, giữ thứ tự đăng ký module.
