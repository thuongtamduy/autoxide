## D-20261118-step42-transfers-counts · Chuyển kho và kiểm kê của `inventory`: giá vốn đi theo hàng, kiểm kê áp chênh lệch vào tồn hiện tại — 2026-11-18 — agent quyết

Chủ đề: Thương mại

Quyết định:

1. **Chuyển kho**: `draft` → `ship` (trừ `available` rồi `on_hand` ở kho gửi bằng `transfer_out` — không lấy hàng đã giữ
   cho đơn; giá vốn bình quân của kho gửi lúc đó ghi vào dòng phiếu; cộng `incoming` ở kho nhận) → `receive` một phần hay
   hết (`transfer_in` có giá vốn của dòng, nên giá vốn bình quân của kho nhận tính đúng) → `received`; `close` khi còn hàng
   chưa tới: phần đó bỏ khỏi `incoming` (mất trên đường — ghi điều chỉnh có lý do nếu cần); `cancel` chỉ khi còn nháp.
   Gửi cần `inventory.transfer` ở kho gửi, nhận / đóng ở kho nhận. Phiếu đi theo kho gửi khi tách khách.
2. **Kiểm kê khi đang bán**: mỗi dòng đếm lưu `expected_at_count` = `on_hand` lúc nhập số đếm; áp dụng ghi dòng sổ `count`
   = `counted − expected_at_count` vào `on_hand` HIỆN TẠI, không ghi đè `on_hand` bằng số đếm. Lý do của dòng lấy từ danh
   sách `inventory.adjustment`, mặc định `count_variance`. Nhập số đếm cần `inventory.count`; áp dụng cần
   `inventory.adjust`. Nhập CSV (`sku` hay mã vạch, `counted`, `reason_code`) gửi trong thân JSON `{csv}`; lỗi nêu từng dòng.
3. **Số chứng từ** `CK-<năm>-<6 số>` / `KK-<năm>-<6 số>` cấp bằng `numbering` theo khách và năm, trong transaction của phiếu
   (unique theo khách nên không theo kho).

Lý do: hàng bán xen giữa lúc đếm và lúc áp dụng đã có dòng sổ riêng; ghi đè bằng số đếm sẽ tính trừ hai lần. Giá vốn theo
hàng giữ báo cáo lãi gộp theo kho đúng khi hai kho nhập giá khác nhau.

Đã cân nhắc: khoá kho khi kiểm kê — dừng bán cả cửa hàng; dãy số theo kho — trùng số giữa các kho của cùng khách.
