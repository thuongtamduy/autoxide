## D-20261119-step42-lots-serials · Lô, hạn dùng và serial của `inventory`: giữ ở mức dòng tồn, chọn lô lúc xuất, hàng không gắn lô được phép — 2026-11-19 — agent quyết

Chủ đề: Thương mại

Quyết định:

1. **Lô** (`inventory_lots`) thuộc một dòng tồn; tồn của lô chỉ đổi bằng `rows::adjust` (`stock::move_lot`). Tổng tồn các
   lô ≤ `on_hand`: phần còn lại là hàng không gắn lô (vd chênh lệch kiểm kê dương — không biết lô). Giữ chỗ ở mức dòng tồn;
   lô chọn lúc hàng ra: nêu lô thì chỉ lô đó; không nêu thì các lô chưa hết hạn theo hạn dùng sớm nhất trước (FEFO), rồi
   tới hàng không gắn lô — không bao giờ lấy vào lô đã hết hạn (`inventory.lot_unavailable`). Mỗi phần lô một dòng sổ.
2. Thêm hàng của biến thể theo dõi lô phải nêu lô (và hạn dùng nếu có). **Job hằng ngày `inventory.expire_lots`** trừ lô
   quá hạn khỏi `on_hand` và `available` (dòng sổ `expiry`, lý do `expired`, event `inventory.lot_expired`); khoá dòng lô
   nên chạy lại không trừ hai lần.
3. `expiry_state` (`ok` / `expiring` / `expired`) tính lúc đọc từ `expires_on` và cài đặt kế thừa theo nút
   `inventory.expiring_days` (mặc định 30); `GET /lots?expiry_state=` lọc bằng khoảng ngày trên index.
4. **Serial** (`inventory_serials`, duy nhất theo biến thể): thêm hàng / nhập lại kèm đủ serial (`in_stock` ở kho); hàng ra
   (bán, điều chỉnh giảm, chuyển đi) phải nêu đủ serial đang `in_stock` ở kho đó, đổi trạng thái bằng
   `UPDATE … WHERE status = 'in_stock'` nên một serial không bán hai lần dù hai lần xuất chạy cùng lúc.
5. **Chuyển kho** giữ lô và serial: lúc gửi, lô lấy FEFO ghi vào dòng phiếu (JSON `lots`), serial đi đường
   (`in_transit`); lúc nhận (một phần được), phần nhận chia theo thứ tự lô lúc gửi và tạo lại lô cùng mã, cùng hạn ở kho nhận.
6. Kiểm kê: chênh lệch âm trừ lô FEFO; chênh lệch dương thành hàng không gắn lô. Kiểm kê không đổi trạng thái serial.

Lý do: giữ chỗ không biết lô nào sẽ được giao (nhặt hàng ở kho mới biết); cho phép hàng không gắn lô thay vì bịa mã lô cho
hàng thừa khi kiểm kê. Không có quy định chung bắt buộc FEFO — đây là mặc định an toàn để không bán hàng quá hạn (cần người có
chuyên môn xác nhận cho từng ngành).

Đã cân nhắc: giữ chỗ theo lô — đơn bị khoá vào một lô trước khi biết lô nào còn ở kệ; bắt mọi hàng phải có lô — kiểm kê thừa
không có mã lô để ghi.
