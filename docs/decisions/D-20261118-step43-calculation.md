## D-20261118-step43-calculation · Khuyến mại: thứ tự tính, cộng dồn, làm tròn và chia tiền — 2026-11-18 — agent quyết

Chủ đề: Thương mại

Quyết định:

1. **Thứ tự cố định** (bộ tính thuần `modules/promotions/src/engine`, không database): ứng viên → lớp `product` → lớp
   `order` (trên phần còn lại sau lớp `product`) → lớp `shipping` → trần pháp luật → sàn giá `block` → chia người trả
   tiền. Trong mỗi lớp xếp theo `priority` giảm dần rồi `id`; mọi danh sách dòng xếp theo `line_key`, nên giỏ xáo trộn
   thứ tự dòng ra cùng kết quả và cùng `fingerprint`.
2. **Cộng dồn**: hai chương trình cộng được khi CẢ HAI cho phép (`exclusivity`, `combines_with`); `combines_with` mặc
   định là cả ba lớp. Chế độ `priority` (mặc định) áp lần lượt; `best_for_buyer` so tập cộng dồn được với từng chương
   trình độc quyền đứng riêng (tuyến tính, tất định). Mỗi lớp tối đa `MAX_VOUCHERS_PER_CLASS` voucher cho mỗi nút sở hữu
   chương trình (bên trả tiền).
3. **Làm tròn**: phần trăm tính trên TỔNG dòng (không trên đơn giá rồi nhân) bằng `rust_decimal`, `HalfUp`
   (`MidpointAwayFromZero`) về đơn vị nhỏ nhất. Khoản giảm cấp đơn, `max_discount`, combo và phần của người trả tiền chia
   bằng `Money::allocate` (tổng khớp từng đồng). Trần pháp luật mỗi dòng = giá tham chiếu × số lượng × `max_bp` / 10 000,
   làm tròn xuống; vượt thì cắt khoản áp sau trước, đánh dấu `capped_by_law`.
4. **Thiếu giá tham chiếu** (sản phẩm chưa có giá ngay trước khuyến mại): không áp được trần nên không giảm; khoản đó vào
   `Evaluation.withheld` với lý do `promotions.reason.no_reference_price`. Trần của một dòng nhiều chương trình: giá tham
   chiếu tại `starts_at` sớm nhất, `max_bp` thấp nhất trong các chương trình đã áp (cách an toàn).
5. Lượt flash sale, ngân sách, lượt tổng mà bản `CompiledPromotions` trong bộ nhớ thấy (có thể cũ tới
   `COMPILED_TTL_SECONDS`) chỉ để hiển thị; `reserve` mới là nơi quyết định, cộng bộ đếm có trần trong một transaction.

Lý do: cùng giỏ phải luôn ra cùng kết quả (dấu vân tay `orders` gửi lại khi giữ); làm tròn trên tổng dòng không lệch khi
nhân số lượng; cắt khoản áp sau trước giữ nguyên chương trình ưu tiên cao.

Đã cân nhắc: tìm tổ hợp tối ưu toàn cục (bùng nổ tổ hợp, khó đoán với người bán); làm tròn `Down` cho chiết khấu (lệch so
với hoá đơn khi khách tính tay theo phần trăm).
