## D-20261128-shipping-load-ceiling · Trần cân nặng của vận chuyển; khoản thanh toán khác tiền tệ vào dead-letter — 2026-11-28 — agent quyết

Chủ đề: Thương mại

Quyết định: `shipping` nhận số liệu hàng có trần: một món ≤ 10 000 000 g, mỗi cạnh ≤ 10 000 000 mm, số lượng ≤
10 000 000 (như `catalog`); cân tính phí của một báo giá, một kiện và mọi kiện của một vận đơn ≤ 1 000 000 000 g
(`api::MAX_PARCEL_WEIGHT_GRAMS`, `api::MAX_SIDE_MM` công khai cho bộ nối hãng). Vượt trần → 422 (lỗi theo trường hay
`shipping.load_too_large`); phép tính trên đó dùng `i128` rồi so trần, phí bậc tràn → `money.overflow` — không còn
bão hoà về `i64::MAX`. Bộ nối GHN tự kiểm tổng của nó (`ghn.parcel_too_large`). Khoản `ledger.payment_succeeded`
khác tiền tệ của khoản thanh toán của đơn: `orders` trả lỗi `orders.payment_received_currency_mismatch` (event thử
lại rồi vào dead-letter), ghi log lỗi + metric `orders_payment_currency_mismatch_total` + luật cảnh báo; người vận
hành hoàn qua `ledger` (docs/MONEY.md).

Lý do: bão hoà tránh panic nhưng biến đầu vào vô lý thành phí / cân vô nghĩa gửi đi — người gọi (agent) không biết
mình sai. 1 000 tấn là xa trên mọi lô hàng thương mại điện tử mà vẫn đủ nhỏ để cộng vài kiện không tràn `i64`.
Khoản ngoại tệ giữ cách vào dead-letter vì đó là tín hiệu bền đã có cảnh báo; nuốt lỗi (trả Ok) sẽ mất dấu khoản
tiền phải hoàn.

Đã cân nhắc: đặt trần theo giới hạn thương mại của từng hãng (GHN…) — loại, số đó do hãng đổi và hãng tự từ chối;
báo `shipping.load_too_large` thành lý do "không dùng được" của từng phương thức — loại, quá trần là lỗi đầu vào
chung mọi phương thức; với khoản ngoại tệ thêm trạng thái mới cho khoản thanh toán — loại trước 1.0, đổi hợp đồng API.
