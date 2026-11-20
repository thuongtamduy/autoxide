## D-20261120-step47-stored-value · Thẻ quà tặng và số dư cửa hàng của `customers` (bước 47 mục 4): tắt hai lớp, khép kín trong khách phát hành, không đường rút tiền, chỉ đếm mã sai — 2026-11-20 — agent quyết

Chủ đề: Thương mại

Bối cảnh: đặc tả `docs/steps/47-customers.md` (mục "Thẻ quà tặng và số dư cửa hàng (TẮT mặc định)"), D-194, D-187
(core không giữ tiền của người mua). Thẻ trả trước dùng nhiều nơi hay rút được ra tiền có thể thành phương tiện thanh
toán / ví điện tử cần giấy phép; thẻ chỉ dùng mua hàng của chính bên phát hành thường được coi là khép kín.

Quyết định:

1. **Tắt hai lớp, mặc định tắt.** `AUTOXIDE_CUSTOMERS_STORED_VALUE_ENABLED=false` (người vận hành) **và** tính năng
   của khách `customers.gift_cards` / `customers.store_credit` (gói, bước 35). Thiếu một lớp → mọi endpoint 403
   `customers.stored_value_disabled` (kiểm trước khi đụng dữ liệu, kể cả trước khi `/me` tạo hồ sơ); biến môi trường
   tắt thì tender `gift_card` / `store_credit` không được cắm vào `orders` và job dọn không làm gì. `autoxide doctor`
   cảnh báo khi đã bật.
2. **Khép kín:** thẻ và số dư chỉ trả cho đơn của chính khách (tenant) phát hành. **Không có** endpoint, tool MCP hay
   tender nào rút ra tiền, chuyển sang người khác, nạp bằng tiền hay đổi tiền mặt; test kiểm danh sách đường dẫn
   OpenAPI và danh sách tool. Gắn thẻ vào tài khoản (`/me/gift-cards/attach`) chỉ để người mua thấy thẻ của mình,
   không chuyển quyền dùng.
3. **Giới hạn theo cửa hàng (`org_scope`) chưa làm:** `Tender::hold` chưa biết đơn thuộc nhánh nào, nên cột để dành
   (luôn rỗng) và API không nhận trường này — tránh một giới hạn trông như có mà không được kiểm.
4. **Mã thẻ:** 16 ký tự ngẫu nhiên từ bảng 31 ký tự không nhập nhằng (~79 bit, lấy mẫu loại bỏ để không lệch), chỉ
   lưu SHA-256 + 4 ký tự cuối; mã chỉ trả một lần lúc phát hành; phát hành lại cùng `reference` trả thẻ cũ không kèm
   mã ("trùng thì bỏ qua" nên hai request cùng lúc chỉ ra một thẻ).
5. **Chống dò mã: chỉ đếm lần tra SAI**, theo người tra (`throttle` của kernel), quá
   `AUTOXIDE_CUSTOMERS_GIFT_CARD_LOOKUPS_PER_HOUR` lần sai trong giờ thì chặn cả tra đúng (429
   `customers.lookup_throttled`). Đếm cả lần đúng thì người mua dùng thẻ nhiều lần bị chặn oan. Vì bộ đếm theo người,
   **trả bằng thẻ cần đăng nhập** (`customers.stored_value_need_account`): khách vãng lai dùng chung một bộ đếm thì
   một người đoán mã sẽ khoá mọi khách vãng lai.
6. **Trả đơn** theo khung `Tender` của bước 47 mục 3: giữ = ghi chỗ giữ trước ("trùng thì bỏ qua" theo đơn + loại)
   rồi trừ số dư bằng một câu có điều kiện không âm (`rows::adjust`), nên trả đồng thời không vượt số dư (test đồng
   thời). `orders.order_confirmed` → xác nhận; `orders.order_cancelled`, hết hạn giữ (job) → trả lại. Thẻ đã huỷ / hết
   hạn trong lúc đang giữ: phần trả lại không về thẻ mà ghi thành thu nhập khác.
7. **Số dư cửa hàng** chỉ phát sinh từ bồi thường của nhân viên (`POST /{id}/store-credit/adjustments`, có
   `reference` và lý do, gọi lại không cộng hai lần, trừ không xuống dưới 0). Chưa làm: người mua chọn nhận hoàn tiền
   dạng số dư, hoàn đơn trả bằng thẻ về lại thẻ, bán thẻ qua `orders` (`customers_gift_card_products`) — phát hành hiện
   tại là ở quầy với `reference` của chứng từ bán.
8. **Mệnh giá tối đa** là cấu hình `AUTOXIDE_CUSTOMERS_GIFT_CARD_MAX_MINOR` (giới hạn rủi ro của người vận hành, không
   phải con số của văn bản pháp luật nên không cần ngày hiệu lực); hạn dùng 1–120 tháng hay không hạn.
9. **Sổ cái** (qua job `customers.post_ledger` như điểm thưởng): phát hành Nợ `buyer.prepayments` / Có
   `gift_card.liability`; dùng Nợ `gift_card.liability` (hay `store_credit.liability`) / Có `buyer.prepayments`; huỷ, hết
   hạn: Nợ `gift_card.liability` / Có `stored_value.breakage`; bồi thường: Nợ `store_credit.expense` / Có
   `store_credit.liability` (trừ thì ngược lại). Mọi thay đổi số dư có một dòng trong `customers_stored_value_entries`
   (chỉ thêm).

Căn cứ: NĐ 52/2024/NĐ-CP về thanh toán không dùng tiền mặt (ví điện tử, phương tiện thanh toán; hiệu lực 01/07/2024) —
ranh giới "thẻ khép kín chỉ dùng tại bên phát hành" và thuế GTGT khi bán / khi dùng thẻ (khuyến nghị: bán thẻ là nhận
trước, xuất hoá đơn khi dùng thẻ mua hàng) **cần đối chiếu văn bản gốc / người có chuyên môn pháp lý** trước khi bật.

Hệ quả: người vận hành không vô tình bật một sản phẩm giữ tiền; khi bật, cửa hàng phát hành thẻ ở quầy, người mua trả
một phần hay toàn bộ đơn bằng thẻ / số dư, và nghĩa vụ còn lại luôn có trong sổ cái.
