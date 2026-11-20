## D-20261120-step47-loyalty · Điểm thưởng, hạng và voucher tặng của `customers` (bước 47 mục 3, 6): tích khi hoàn tất, lô có hạn, điểm là tender của `orders`, nghĩa vụ ở sổ của nút gốc — 2026-11-20 — agent quyết

Chủ đề: Thương mại

Bối cảnh: đặc tả `docs/steps/47-customers.md` (mục "Điểm thưởng và hạng", "Voucher cho người mua") để agent tự quyết
mốc tích điểm, hạn của điểm trả lại, và giả định `orders` đã dùng `Tender` lúc đặt hàng — thật ra `orders` (bước 45) chỉ
có điểm cắm `add_tender`, chưa gọi tender nào.

Quyết định:

1. **Tích khi `completed`** (đã qua hạn đổi trả, ít phải đảo), ngay trong transaction cập nhật tổng chi; lô mang
   `source_ref = orders:<id>:earn` (unique) nên event nhận lại không tích hai lần. Hạng xét TRƯỚC khi tích: đơn vừa hoàn
   tất được tích theo hạng mà chính nó giúp đạt. Điểm = ⌊tiền hàng đã trả / `earn_per_minor`⌋ × `points_per_unit` × hệ số
   hạng (phần vạn, số nguyên `i128`). Hệ số theo danh mục chưa làm (event của `orders` chưa mang danh mục của dòng).
2. **Hạn điểm theo giờ Việt Nam** (UTC+7, cố định): cuối ngày `points_valid_months` tháng sau lúc tích; lưu UTC.
3. **`orders` gọi tender lúc đặt hàng** (`CheckoutInput.tenders`, theo thứ tự gửi, mỗi loại một lần): giữ sau tồn và
   khuyến mại với `reference = orders:<id>`, trần là phần còn phải trả; lỗi thì trả mọi chỗ giữ. Mỗi tender là một khoản
   thanh toán `succeeded` phương thức `tender.<loại>` ghi trước khi tạo đơn; khoản thu ở đối tác chỉ còn phần còn lại; trả
   đủ bằng tender thì đơn vào thẳng `confirmed` / `paid`. `goods_paid` trong event trừ phần trả bằng tender (D-226).
   Lượt đặt treo được job trả mọi tender đã cắm. Mã lỗi mới `orders.tender_unavailable` (422).
4. **Giữ / xác nhận / trả điểm:** giữ trừ số dư dùng được bằng `rows::adjust` TRƯỚC (thất bại → 409
   `customers.insufficient_points`, chưa đụng lô), khoá lô theo `id` rồi tiêu lô hết hạn sớm nhất trước. Xác nhận khi
   nghe `orders.order_confirmed`, trả khi nghe `orders.order_cancelled` hay quá hạn (job 5 phút); mỗi bước đổi trạng thái
   chỗ giữ có điều kiện `status = 'held'`. Giá trị trừ cho người mua = điểm × giá trị điểm HIỆN HÀNH; luật đổi tối thiểu
   và trần % đơn là dữ liệu của quy tắc.
5. **Sổ cái ở sổ của nút gốc**, qua job `customers.post_ledger` đưa vào hàng đợi cùng transaction (`unique_key` = mã
   tham chiếu bút toán; `ledger` trả `Replayed` khi chạy lại). Tài khoản riêng mở bằng `open_account` với bên thường:
   `loyalty.liability` (Có), `loyalty.expense` (Nợ), `loyalty.breakage` (Có), `buyer.prepayments` (Có). Tích: Nợ chi phí /
   Có nghĩa vụ theo giá trị LÚC TÍCH lưu trên lô; đổi trả đơn: Nợ nghĩa vụ (giá trị lô) / Có `buyer.prepayments` (tiền
   được trừ), chênh lệch vào chi phí; hết hạn: Nợ nghĩa vụ / Có `loyalty.breakage`; điều chỉnh, đổi voucher, thu hồi: nghĩa
   vụ ↔ chi phí. Đối trừ `buyer.prepayments` với phải thu của đơn là việc của bút toán doanh thu của `orders` — chưa làm,
   ghi tồn đọng.
6. **Hoàn tiền sau khi đã tích:** thu hồi điểm của lô đơn theo tỉ lệ tiền hoàn, tối đa phần còn chưa dùng; phần đã tiêu
   không thu hồi (không làm số dư âm), đã nằm ở chi phí. Hoàn tiền phần đơn trả bằng điểm (trả điểm thành lô mới) chưa
   làm: `orders.refund_completed` chưa tách phần của tender — ghi tồn đọng.
7. **Điều chỉnh tay** (`customers.loyalty.adjust`, theo phạm vi của hồ sơ) có `reference` + lý do (D-184), cùng
   `reference` không làm hai lần; cộng là lô mới theo quy tắc hiện hành.
8. **Quy tắc, hạng, chương trình voucher là dữ liệu chỉ thêm** (`effective_from` không ở quá khứ, lệch đồng hồ tới 1 phút
   cho qua); quyền `customers.loyalty.manage` ở nút gốc (chương trình chung của khách).
9. **Voucher tặng** qua `Promotions::grant_voucher`, gọi ngoài transaction ghi: sinh nhật (job hằng ngày, ngày giờ Việt
   Nam, cột `birthday_md` có index để tìm như nhau trên hai database, `source_ref = birthday:<năm>`), sau khi mua
   (`purchase:<order_id>`, có tiền hàng tối thiểu), đổi điểm (`points:<mã>` — trừ điểm trước, cấp lỗi thì hoàn điểm thành
   lô mới cùng giá trị, hạn 1 tháng). Lỗi nghiệp vụ của `promotions` khi tặng chỉ ghi log; lỗi hạ tầng trả lên để chạy lại.

Căn cứ: Luật Kế toán 88/2015/QH13 (nghĩa vụ điểm là số liệu kế toán, giữ ít nhất 10 năm); Luật Bảo vệ quyền lợi người
tiêu dùng 19/2023/QH15 (điều kiện chương trình điểm công khai cho người mua — `GET /me/points` trả quy tắc hiện hành) —
**cần đối chiếu văn bản gốc / người có chuyên môn pháp lý**.

Hệ quả: người mua trả một phần đơn bằng điểm ở mọi kênh đặt hàng online; số dư không bao giờ âm dù nhiều đơn cùng lúc;
nghĩa vụ điểm trong sổ cái luôn bằng Σ điểm còn × giá trị lúc tích (test). Thẻ quà tặng / số dư (bước 47 mục 4) cắm vào
cùng đường tender.
