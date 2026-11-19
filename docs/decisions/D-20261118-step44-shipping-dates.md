## D-20261118-step44-shipping-dates · `shipping`: ngày giao dự kiến theo lịch giao, giờ sẵn sàng nhận tại cửa hàng, ước tính công khai đọc biến thể qua `Catalog` — 2026-11-18 — agent quyết

Chủ đề: Thương mại

Bối cảnh: đặc tả bước 44 mục 6–7 và việc website bán hàng (D-200, D-201, D-223…D-227). Ngày giao phải là ngày tuyệt đối
theo múi giờ, ngày nghỉ và giờ chốt là dữ liệu. Ước tính công khai cần cân nặng của biến thể, mà tiêu chí xong cũ của đặc
tả ghi "`shipping` không phụ thuộc `autoxide-catalog`".

Quyết định:

1. **Lịch giao** (`shipping_calendars`, `shipping_calendar_days`): múi giờ IANA (mặc định `Asia/Ho_Chi_Minh`), ngày làm việc
   trong tuần (mặc định thứ Hai – thứ Sáu), ngày riêng `closed` (nghỉ lễ) hay `special_hours` (giờ chốt riêng). Khách nhập
   ngày nghỉ khi có thông báo chính thức; không viết cứng ngày lễ nào. Phương thức không gắn lịch dùng lịch chuẩn: thứ Hai
   – thứ Bảy, giờ Việt Nam.
2. **`eta`**: hàng rời kho ngày làm việc đầu tiên còn kịp giờ chốt (`cutoff_minute` của ngày riêng, không có thì của
   phương thức; không có giờ chốt = kịp cả ngày); `earliest_date` / `latest_date` = cộng `estimated_days` (ít nhất, nhiều
   nhất) ngày làm việc; `same_day` = 0 ngày. `order_by` = giờ chốt hôm nay khi còn kịp. Hàm tính thuần (`eta.rs`), không
   chạm database, có test riêng.
3. **Nhận tại cửa hàng:** `ready_at` = lúc đặt + `prep_hours` của điểm nhận, lùi tới giờ mở cửa kế tiếp theo lịch của
   phương thức; giờ mở cửa tạm là 08:00–21:00 vì giờ mở cửa trong hồ sơ công khai của nút (bước 34) chưa có trong
   `autoxide_organizations::api` — đã báo phiên điều phối; khi có hàm đọc thì chuyển sang. `hold_days` của phương thức trả
   cùng lựa chọn cho bước 45.
4. **Ước tính công khai** `GET /api/v1/shipping/public/estimate?variant_id&quantity&province_code` chạy đúng báo giá của
   giỏ một dòng (cùng bảng phí, cùng miễn phí, cùng `eta`), đích chỉ có tỉnh (không kiểm danh mục xã), trả `estimate: true`,
   `Cache-Control: public, max-age=60`. Cân nặng, kích thước, nút bán và giá (để xét miễn phí) đọc qua
   `autoxide_catalog::api::Catalog` — theo giao việc của chủ dự án ("bước 44 cần trait `Catalog`"); thay câu "không phụ
   thuộc `autoxide-catalog`" trong tiêu chí xong của đặc tả 44: `shipping` vẫn không đọc bảng của `catalog` và báo giá
   cho `orders` vẫn nhận số liệu do bên gọi đưa vào. Không cài `catalog` → `shipping.estimate_unavailable`.

Hệ quả: website bán hàng hiện phí và ngày giao theo tỉnh trên trang sản phẩm, kèm giờ chốt trong ngày của nơi bán.
