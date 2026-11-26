## D-20261126-tax-per-unit-by-quantity · Thuế theo đơn vị chia về dòng theo số lượng; kiểm trần thuế suất của gói — 2026-11-26 — agent quyết

Chủ đề: Tiền, thuế, chứng từ

Quyết định:

1. Thuế theo đơn vị của một nhóm chia về dòng theo **số lượng của dòng** (phần nguyên dưới + phần lẻ lớn nhất, trọng số
   0 nhận 0), không theo số tiền dòng. Chuỗi có thuế theo đơn vị thì mọi cột (giá chưa thuế, thuế theo phần trăm) cũng
   chia theo giá trị chính xác của chính dòng (tách riêng dòng theo số lượng của nó). Chuỗi không có thuế theo đơn vị
   giữ nguyên `Money::allocate` theo số tiền — số không đổi.
2. Giá đã gồm thuế: cột lấy phần còn lại của dòng là thuế cuối có tổng khác 0 **không theo đơn vị**, không có thì giá
   chưa thuế. Ô âm sửa bằng hoán đổi 2×2 như D-20261126-tax-rounding-allocation, thêm điều kiện dòng nhận phải được mang
   cột cho (thuế theo đơn vị chỉ ở dòng có số lượng > 0). Không có thuế theo đơn vị thì luôn hoán đổi được (chứng minh ở
   `engine/lines.rs`); có thì nếu kẹt báo `range` như giá chưa thuế âm (500.000 ca property test không lần nào kẹt).
   Vòng hoán đổi dùng con trỏ dòng nhận chỉ tiến cho mỗi cột: O(số dòng × số cột + số đơn vị phải dời).
3. Giá đã gồm thuế: dòng có số tiền thấp hơn thuế theo đơn vị nằm trong nó (giá chưa thuế của riêng dòng làm tròn âm)
   bị từ chối `range` ở `lines[i].amount`, như cả nhóm trước đây.
4. Gói thuế: từ chối `per_unit` âm, thuế suất ngoài 0–1000%, thuế suất khấu trừ thay ngoài 0–100%.

Lý do: thuế theo đơn vị (vd thuế bảo vệ môi trường theo lít, TTĐB theo đơn vị) phát sinh theo số lượng hàng của từng
dòng; chia theo số tiền làm dòng số lượng 0 mang thuế theo đơn vị — hoá đơn ghi sai. Giá chưa thuế chia theo số tiền
khi có thuế theo đơn vị làm phần còn lại của dòng lệch lớn (hàng trăm đồng phải hoán đổi), nên chia theo giá trị chính
xác của dòng. Trần thuế suất chặn lỗi nhập liệu làm tràn số (2,4e20% từng trả `money.overflow`).

Đã cân nhắc: làm tròn thuế theo đơn vị riêng từng dòng rồi cộng lại thành tổng nhóm — trái nguyên tắc làm tròn một lần
trên tổng nhóm (D-341) và đổi tổng chứng từ khi số lượng lẻ; giải bài làm tròn ma trận có ràng buộc ô bằng luồng — chứng
minh đủ cho mọi trường hợp nhưng nặng, trong khi trường hợp kẹt chỉ có ở dữ liệu thuế theo đơn vị vượt số tiền.
