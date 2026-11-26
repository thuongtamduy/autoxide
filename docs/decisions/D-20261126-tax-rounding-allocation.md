## D-20261126-tax-rounding-allocation · Thuế giá đã gồm thuế: phân bổ phần lệch làm tròn không bao giờ ra thuế âm — 2026-11-26 — agent quyết

Chủ đề: Tiền, thuế, chứng từ

Quyết định: Ở chế độ giá đã gồm thuế (`modules/tax/src/engine.rs`) mọi kết quả phải giữ bốn bất biến, ở cấp nhóm
và cấp dòng: (I1) mọi thuế ≥ 0; (I2) thành phần có thuế danh nghĩa 0 (0%, không chịu thuế, đơn giá × số lượng = 0)
ra đúng 0; (I3) giá chưa thuế + Σ thuế = số tiền mỗi dòng, tổng nhóm mỗi cột = tổng các dòng; (I4) giá chưa thuế ≥ 0.
Cách tính:

1. **Nhóm** (`inclusive_group`, `settle`): giá chưa thuế `n = HalfUp(tách ngược chính xác)` như trước; thuế mỗi tầng
   tính xuôi `HalfUp(thuế suất × cơ sở đã làm tròn)` như trước (người đọc hoá đơn tự kiểm được từng dòng thuế). Lệch
   `D = (tổng − n) − Σ thuế`: `D > 0` cộng vào tầng chịu thuế cuối nếu thuế chính xác của nó dương, không thì tầng sau
   cùng có thuế chính xác dương; `D < 0` trừ ở tầng chịu thuế cuối tới 0 rồi các tầng đứng trước gần nhất còn ≥ 1.
   Chứng minh đủ chỗ: mọi thuế chính xác = 0 ⇒ `n` = tổng ⇒ `D = 0`; `tổng − n ≥ 0` (giá chưa thuế chính xác ≤ tổng,
   tổng là số nguyên) nên khi Σ thuế vượt nó luôn còn tầng ≥ 1.
2. **Dòng** (`inclusive_lines`): giá chưa thuế và từng thuế chia theo tỉ lệ số tiền dòng bằng `Money::allocate`; cột
   thuế cuối có tổng khác 0 lấy phần còn lại của dòng (như trước). Ô âm sửa bằng hoán đổi 2×2 (dòng có ô âm nhận một
   đơn vị từ cột khác của chính nó, dòng có ô cột cuối ≥ 1 đổi ngược lại): tổng dòng, tổng cột giữ nguyên, không ô nào
   xuống dưới 0, tổng phần âm giảm 1 mỗi lượt. Luôn có ô để đổi vì giá chưa thuế của dòng ≤ số tiền dòng và cột cuối
   cộng lại ≥ 0.

**Tính chất giữ tương thích:** khi cách cũ (dồn hết phần dư vào tầng chịu thuế cuối, ở dòng lấy phần còn lại) đã thoả
I1 và I2 thì kết quả mới trùng từng đồng — kiểm bằng mô hình phân số chính xác trên 55.100 ca ngẫu nhiên. Số chỉ đổi ở
những chứng từ cách cũ ra thuế âm hay ra thuế khác 0 cho tầng 0% / không chịu thuế: chuỗi có tầng 0% hay không chịu
thuế đứng sau tầng có thuế (`[k%, 0%]`), chuỗi từ hai tầng có thuế mà tầng cuối làm tròn không đủ bù, và nhóm nhiều dòng
với từ hai tầng có thuế khi một dòng từng nhận thuế âm.

Lý do: tổng tiền thuế trên hoá đơn phải khớp giá thanh toán và từng dòng thuế phải là số không âm ghi được lên hoá đơn
điện tử (NĐ 123/2020/NĐ-CP sửa bởi NĐ 70/2025/NĐ-CP; TT 32/2025/TT-BTC về nội dung hoá đơn — quy định chi tiết về cách
làm tròn từng dòng thuế khi giá đã gồm thuế **cần đối chiếu văn bản gốc / người có chuyên môn**). Hai lần sửa trước
chỉ đổi tầng nhận phần dư và vẫn có phản ví dụ (vd `[65%, 10%, 10%]` tổng 5 → tầng cuối −1; `[1%, 1%]` hai dòng 26 → một
dòng −1). Giữ nguyên số của mọi ca cũ đã đúng để chứng từ lưu rồi tính lại không lệch.

Đã cân nhắc: tính thuế mỗi tầng từ giá trị chính xác rồi phân phần lệch theo phần lẻ lớn nhất (largest remainder) — sát
tỉ lệ hơn nhưng đổi số của chuỗi nhiều tầng thông thường (vd TTĐB + GTGT) và thuế ghi trên hoá đơn có thể khác "thuế
suất × cơ sở in trên hoá đơn"; bài làm tròn ma trận đầy đủ (luồng trên đồ thị) — đúng nhưng nặng, không cần khi hoán
đổi 2×2 đã đủ chứng minh; phân bổ tuần tự từng dòng theo phần còn lại — đổi số của mọi nhóm nhiều dòng.

Thay: phần "tầng cuối nhận phần dư (dòng: tổng dòng − phần còn lại)" của D-341
