## D-20261118-step41-price-rules · `catalog`: bảng giá suy ra, làm tròn giá đẹp, đổi giá hàng loạt — 2026-11-18 — agent quyết

Chủ đề: Thương mại

Quyết định:

1. **Bảng giá suy ra**: giá = `giá gốc × (10000 + adjust_bp) / 10000` (làm tròn nửa lên tới đơn vị nhỏ nhất) `+
   adjust_minor`, không âm, rồi làm tròn theo quy tắc. Job `catalog.derive_prices` xếp cùng transaction khi bảng gốc có
   dòng giá mới (và khi tạo bảng suy ra, cho mọi dòng hiện có), đi theo lô 500 dòng gốc, mỗi lô một transaction; dòng
   ghi có `reference = derived:<id dòng gốc>` nên chạy lại không nhân đôi. Giá gạch suy ra cùng cách, chỉ giữ khi cao
   hơn giá. Dòng tay ghim giá khi đang hiệu lực lúc dòng gốc bắt đầu. Bảng suy ra phải cùng tiền tệ với bảng gốc.
2. **Làm tròn giá đẹp**: mức `k × step + ending` (`ending` 0 = bội số của `step`), `up` / `down` / `nearest` (cách
   đều thì lên); không bao giờ âm (xuống dưới mức đầu thì lấy mức đầu ≥ 0). Chỉ số nguyên.
3. **Đổi giá hàng loạt**: phạm vi là hợp của danh mục (gồm danh mục con), sản phẩm, biến thể; rỗng hết = mọi biến thể
   đang có giá trong bảng; tối đa 50.000 biến thể. Giá cũ là giá **của chính bảng đó** lúc `valid_from` (không theo
   thứ tự chọn bảng). `percent` / `amount` bỏ qua biến thể chưa có giá; `set` thì không. Giá mới dưới sàn `block`
   không được ghi (bản xem trước báo số lượng). Duyệt cần `catalog.prices.approve`; cài đặt nút
   `catalog.price_change_requires_second_person` (mặc định tắt, đọc qua `OrgDirectory` của `organizations`; chưa cài
   module đó thì coi như tắt) bật thì người tạo không tự duyệt (403 `catalog.self_approval`). Job ghi theo lô 500
   biến thể, `reference = bulk:<lần đổi>:<biến thể>`, rồi đặt `applied`.

Lý do: D-222 yêu cầu đọc giá không tính công thức và lịch sử giá là dòng thật (trần 50% của khuyến mại cần "giá ngay
trước khuyến mại"). Lấy giá cũ trong chính bảng thay vì giá giải ra để lần đổi chỉ phụ thuộc bảng nó sửa.

Đã cân nhắc: tính giá suy ra lúc đọc — đường nóng phải tính, lịch sử không có dòng thật; bảng suy ra nhiều tầng — khó
biết giá từ đâu ra, D-222 chỉ cho một tầng.
