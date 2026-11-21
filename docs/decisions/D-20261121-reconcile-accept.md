## D-20261121-reconcile-accept · Đối soát: `match` chỉ khi đúng số, số lệch cần `accept_received`; event mang số thực thu — 2026-11-21 — agent quyết

Chủ đề: Tiền, thuế, chứng từ

Quyết định:

1. Xử lý mục chờ bằng `match` chỉ ghi "đã thu" khi số đối tác thực thu **bằng** số phải thu. Lệch (thiếu hay thừa) thì
   từ chối (`ledger.amount_differs`, 409); thanh toán giữ `needs_review` tới khi người có `ledger.reconcile` xác nhận
   riêng bằng hành động mới `accept_received` (REST `POST /exceptions/{id}/resolve` và tool MCP `resolve_exception`).
2. `match` / `accept_received` chỉ nhận: mục chờ loại `amount_mismatch`, `unmatched_credit`, `unknown_in_statement`,
   `missing_in_statement`; `payment_id` đúng thanh toán mục chờ nêu (nếu có nêu); thanh toán còn chờ tiền (`pending`,
   `expired`, `needs_review`, không thì `ledger.payment_state`); cùng tiền tệ với số đã thu. Số đã thu lấy từ mục chờ
   (`received_minor`, `received`, `amount`), không có thì là số phải thu (mục "thiếu trong sao kê").
3. Event `ledger.payment_succeeded` mang `amount` = số **thực thu** (trước đây: số phải thu). Đường thường (webhook,
   đối soát, biến động số dư) chỉ phát khi hai số bằng nhau nên không đổi gì; chỉ `accept_received` phát số khác.

Lý do: trước đây `match` bỏ qua phép so số tiền và event luôn mang số phải thu, nên `orders` cộng đủ tiền vào
`paid_minor` cho một khoản chuyển thiếu. Ghi số lệch là việc của người, phải là hành động riêng có tên và ghi vào mục
chờ (`resolution = accept_received`) để kiểm lại được.

Đã cân nhắc:
- Bỏ hẳn đường ghi số lệch: khoản thiếu vài đồng (phí ngân hàng) không bao giờ đóng được, chỉ còn hoàn ngoài.
- Thêm trường mới (`received`) vào event, giữ `amount` = số phải thu: mọi bên nghe đang cộng `amount`, giữ nghĩa cũ thì
  lỗi vẫn còn ở bên nào quên đọc trường mới.
