## D-20261120-step45-preorder-advance · Trả trước một phần cho hàng đặt trước: một cài đặt điều kiện, phép chia thuần, phần còn lại qua link của `ledger`, giữ lại theo điều kiện đã chụp — 2026-11-20 — agent quyết

Chủ đề: Thương mại

Bối cảnh: mục "Trả trước một phần cho hàng đặt trước" của bước 45 (D-232, chủ dự án quyết; đặc tả 45 mục cùng tên).
Code lệch đặc tả ở vài chỗ cần ghi lại: chính sách của `orders` là cài đặt theo nút (`OrgSetting`, kế thừa theo cây) chứ
không phải bảng `orders_policies`; `ledger` ghi mọi khoản thu là Có `receivable.customer`.

Quyết định:

1. **Một cài đặt `orders.preorder_advance`** (đối tượng JSON) thay cho các cột `preorder_payment = partial`,
   `preorder_advance_*`, `preorder_cancel_retention_*`, `preorder_balance_due_hours`, `preorder_balance_overdue_action` của
   đặc tả: không đặt = trả đủ lúc đặt (mặc định); đặt = trả trước một phần. Kiểm bằng `Terms::parse` khi lưu cài đặt —
   gom mọi chỗ sai vào một thông báo của cài đặt; mã `orders.preorder_advance_invalid` của đặc tả không cần (lỗi của
   khuôn cài đặt chung). Chỉ áp khi người mua trả online; COD vẫn thu khi giao như cũ.
2. **Phép chia thuần** `advance::split` (test thuộc tính 2000 đơn ngẫu nhiên): phí giao (sau giảm, gồm thuế) chia theo tiền
   hàng bằng `Money::allocate`; trả trước = tỉ lệ × phần đặt trước làm tròn xuống (hay số cố định), kẹp [sàn, trần], không
   vượt phần đặt trước; `upfront = total − phần đặt trước + advance`, `balance = total − upfront` (phép trừ — tổng thu
   bằng giá đơn từng đồng); trả trước bằng cả phần đặt trước thì coi như trả đủ.
3. **Công bố trước khi đặt**: giỏ trả `advance {upfront, balance, balance_due_hours, balance_methods, retention_bp,
   retention_max, lines[{line_key, amount, commit_at}]}` (gom một khối thay vì rải vào từng dòng). `accept_preorder_terms`
   là đồng ý cả điều kiện này; điều kiện và SHA-256 chụp vào `orders_advance_plans` cùng transaction tạo đơn.
4. **Dòng tiền** (D-187): khoản lúc đặt có `purpose = advance`, phần còn lại `purpose = balance`, mỗi lần thử một
   `reference` riêng; đều qua `ledger` như mọi khoản thu. **Không mở tài khoản `orders.customer_advance`**: `ledger` đã ghi
   Có `receivable.customer` khi thu — số dư Có đó chính là nghĩa vụ với người mua tới khi giao (bút toán doanh thu lúc
   giao Nợ `receivable.customer` như mọi đơn). Phần giữ lại khi huỷ ghi Nợ `receivable.customer` / Có
   `orders.cancellation_income` (mã tham chiếu `orders:<id>:retention`).
5. **Phần còn lại**: nghe `inventory.preorder_allocated`, mọi dòng đặt trước còn hiệu lực đã chia đủ → kế hoạch
   `balance_requested` (`UPDATE … WHERE status = advance_paid`: event giao lại không mở link thứ hai), event
   `orders.preorder_balance_requested`, thư cho người mua, tạo khoản và link ở `ledger` ngay trong bộ nghe (gọi lại an
   toàn) thay vì job riêng `orders.request_preorder_balance`. Người mua mở lại link hết hạn bằng
   `POST /me/orders/{id}/payments`, nhân viên bằng `POST /{id}/payment-link`; lần thử mới tăng `balance_attempt` bằng
   `UPDATE … WHERE balance_attempt = <cũ>` (hai lần bấm cùng lúc chỉ một link mới). Chưa trả xong thì không yêu cầu
   vận đơn cho hàng đặt trước (409 `orders.balance_unpaid`); hàng sẵn vẫn đi trước.
6. **Quá hạn**: job giờ `orders.expire_preorder_balances` → `balance_overdue`, event, huỷ các dòng đặt trước (kể cả phần
   đã chia hàng — `inventory` trả qua `orders.preorder_cancelled`) theo `overdue_action` đã chụp. **Huỷ**: người mua huỷ
   sau mốc chốt giữ `min(advance của dòng × retention_bp, retention_max)`; người bán huỷ, ngừng bán, dời ngày → hoàn đủ.
   Huỷ dòng đặt trước của đơn chưa trả phần còn lại chỉ hoàn phần trả trước của dòng (không hoàn nhầm tiền hàng sẵn).
7. **Chưa làm**: thu phần còn lại bằng COD (`preorder_balance_cod`) và `advance_invoice_timing = on_advance` — hoá đơn vẫn
   theo `invoice_timing` của đơn (lập một lần cho cả đơn).

Căn cứ: Bộ luật Dân sự 2015 (91/2015/QH13, hiệu lực 01/01/2017) Điều 328 — đặt cọc có chế tài, khác trả trước một phần;
Luật Bảo vệ quyền lợi người tiêu dùng 19/2023/QH15 (hiệu lực 01/7/2024): công bố điều kiện giao dịch trước khi giao kết;
NĐ 123/2020/NĐ-CP sửa bởi NĐ 70/2025/NĐ-CP Điều 9 — thời điểm lập hoá đơn với khoản thu trước. **Cần đối chiếu văn bản gốc
/ người có chuyên môn pháp lý** trước khi chạy thật, nhất là phần giữ lại khi người mua huỷ.

Hệ quả: `orders_lines.advance_minor` / `preorder_commit_at`, bảng `orders_advance_plans`; `get_order` (REST và MCP) trả khối
`advance`.
