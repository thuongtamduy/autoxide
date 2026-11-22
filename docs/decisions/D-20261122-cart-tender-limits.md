## D-20261122-cart-tender-limits · Báo giá giỏ trả tối đa dùng được của từng tender (`tender_limits`, số điểm tối đa) — 2026-11-22 — agent quyết

Chủ đề: Thương mại

Bối cảnh: phần còn lại của mục "Website bán hàng" ở bước 45: báo giá giỏ trả thêm "số điểm tối đa dùng được". Điểm thưởng
đã có ở `customers` (bước 47, D-20261120-step47-loyalty) dưới dạng `Tender` loại `points` cắm vào `orders`; `orders`
chưa hỏi được trước khi đặt.

Quyết định:

1. `orders::api::Tender` thêm `limit(tenant, buyer, max) -> Option<TenderLimit>` **có cài đặt mặc định `None`** (không
   bắt bên cài cũ sửa, D-038). `TenderLimit { amount, units }` (`#[non_exhaustive]`, dựng bằng `new`).
2. Giỏ (`GET /carts/{id}` và mọi response giỏ) có `tender_limits: [{kind, max_amount, max_units}]`, hỏi mọi tender đã cắm
   theo thứ tự cắm với trần là tổng còn phải trả của giỏ (cùng trần checkout dùng); `orders` kẹp kết quả về [0, tổng].
   Tổng giỏ bằng 0 thì không hỏi. Tên chung thay vì `max_points`: thẻ quà tặng / số dư cắm cùng cách.
3. `customers` cài cho `points`: cùng luật `hold_points` kiểm — số dư dùng được (đang giữ không tính), trần
   `max_redeem_ratio_bp` của đơn, giá trị điểm hiện hành; dưới `min_redeem_points` thì 0. Khách vãng lai, chưa có
   chương trình hay khác tiền tệ → không báo. Thẻ quà tặng cần mã nên không báo; số dư (`store_credit`) để sau.
   Chỉ đọc, không giữ chỗ: số có thể đổi tới lúc đặt; checkout vẫn kiểm lại.

Hệ quả: website hiện "dùng tối đa N điểm" trên trang giỏ mà không phải đoán luật đổi điểm.
