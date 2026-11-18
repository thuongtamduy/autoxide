## D-20261118-step44-shipping-rates · `shipping`: cân quy đổi số nguyên, bậc phí theo vùng tỉnh, phương thức kế thừa theo cây, thuế của phí giao do `orders` tính theo `tax_category` — 2026-11-18 — agent quyết

Chủ đề: Thương mại

Bối cảnh: đặc tả bước 44 mục "Tính phí" để agent chốt cách tính cân quy đổi, cách chọn bậc và cách trả dòng thuế của
phí giao.

Quyết định:

1. **Cân quy đổi tính bằng số nguyên:** một món = D × R × C (mm³) / `volumetric_divisor` gram, làm tròn lên (mm³ / 6000
   = cm³ × 1000 / 6000 / 1000 kg). Tính trên `i128` để không tràn. Cân tính phí = max(tổng cân thật, tổng cân quy đổi);
   chỉ món `requires_shipping` được tính.
2. **Chọn bậc:** vùng có tỉnh đích thắng vùng cả nước (`province_code IS NULL`); trong vùng, bậc có
   `min ≤ cân < max` (max NULL = vô hạn) và cạnh dài nhất ≤ `max_side_mm`. Phí = phí bậc + `step_amount` cho mỗi
   `step_grams` (hay phần lẻ) vượt `min_weight_grams`. Bậc chồng nhau (cùng vùng, cùng `max_side_mm`) bị từ chối lúc
   lưu (`shipping.rate_overlap`); bậc khác `max_side_mm` cho hàng cồng kềnh là hợp lệ.
3. **Kế thừa theo cây:** phương thức đang bật của nút bán và mọi tổ tiên (`OrgTree::ancestors` của kernel, không cần
   `organizations`), nút gần hơn thắng cùng `code`. Chỉ phương thức cùng loại tiền với `order_value` được xét.
4. **Phí trong báo giá đã gồm thuế GTGT** (giá bán cho người tiêu dùng gồm thuế); lựa chọn mang `tax_category` của
   phương thức (mặc định `vat.standard`) để `orders` tách dòng thuế của phí giao bằng engine thuế khi lập hoá đơn —
   một chỗ tính thuế cho cả đơn, không tính hai lần (Luật Thuế GTGT 48/2024/QH15 — cần đối chiếu văn bản gốc).
5. **`option_key`** = 16 byte đầu SHA-256 của (phương thức, số tiền, loại tiền, điểm nhận, đích, cân tính phí, dịch vụ):
   phí hay đích đổi → khoá đổi → `confirm_option` trả `shipping.option_changed`.
6. **Nhận tại cửa hàng:** mỗi điểm nhận một lựa chọn; phí là `handling_fee` của phương thức (thường 0); không cần đích
   hay cân nặng. Điểm nhận phải là nút của phương thức hay nút dưới nó.
7. **Phương thức `carrier`** không dùng được tới khi có tài khoản hãng (mục 3): nằm trong `unavailable`.

Hệ quả: bước 45 gọi `ShippingRates::quote` lúc tính giỏ và `confirm_option` lúc đặt; dòng thuế của phí giao do bước 45
tính từ `tax_category`.
