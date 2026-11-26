## D-20261126-tax-per-unit-by-quantity · Thuế theo đơn vị chia về dòng theo số lượng, giữ đúng số cố định; kiểm trần thuế suất của gói — 2026-11-26 — agent quyết

Chủ đề: Tiền, thuế, chứng từ

Quyết định:

1. Thuế theo đơn vị của một nhóm chia về dòng theo **số lượng của dòng** (phần nguyên dưới + phần lẻ lớn nhất, trọng số
   0 nhận 0), không theo số tiền dòng — cả hai chế độ giá. Chuỗi có thuế theo đơn vị thì mọi cột (giá chưa thuế, thuế
   theo phần trăm) cũng chia theo giá trị chính xác của chính dòng (chưa gồm thuế: tính xuôi từ số tiền dòng; đã gồm
   thuế: tách riêng dòng theo số lượng của nó) — vd thuế theo phần trăm đứng SAU thuế theo đơn vị chia theo
   số tiền + đơn giá × số lượng của dòng. Chuỗi không có thuế theo đơn vị giữ nguyên `Money::allocate` theo số tiền.
2. Giá đã gồm thuế, chia về dòng: cột lấy phần còn lại của dòng là thuế cuối có tổng khác 0 **không theo đơn vị**,
   không có thì giá chưa thuế. Ô âm sửa từng đơn vị: trước hết hoán đổi 2×2 với đúng lựa chọn của
   D-20261126-tax-rounding-allocation (cột cho có ô lớn nhất, dòng nhận có ô phần còn lại lớn nhất — heap, O(log n) mỗi
   lượt), dòng nhận phải được mang cột cho (thuế theo đơn vị chỉ ở dòng có số lượng > 0); không được thì tìm đường tăng
   (BFS trên đồ thị dòng × cột). Đường tăng đầy đủ: có cách chia hợp lệ thì luôn có đường (tách hiệu giữa lời giải và
   trạng thái hiện tại thành chu trình xen kẽ — chứng minh ở `engine/lines.rs`), nên chỉ báo `range` khi thật sự không
   có cách chia nào (thuế theo đơn vị không vừa số tiền các dòng có số lượng). Chuỗi không có thuế theo đơn vị: hoán đổi
   2×2 luôn đủ, số trùng từng đồng bản D-20261126-tax-rounding-allocation (đối chiếu 30.000 ca với mô hình).
3. Giá đã gồm thuế, cấp nhóm: phần lệch làm tròn ưu tiên các tầng theo phần trăm (từ cuối về đầu); tầng theo đơn vị
   chỉ nhận khi không còn tầng nào khác — thuế theo đơn vị là số cố định (đơn giá × số lượng), hoá đơn ghi đúng số đó.
   Chuỗi không có thuế theo đơn vị: thứ tự như cũ.
4. Giá đã gồm thuế: dòng có số tiền thấp hơn thuế theo đơn vị nằm trong nó (giá chưa thuế của riêng dòng làm tròn âm)
   bị từ chối `range` ở `lines[i].amount`, như cả nhóm trước đây.
5. Gói thuế: từ chối `per_unit` âm, thuế suất ngoài 0–1000%, thuế suất khấu trừ thay ngoài 0–100%.

Lý do: thuế theo đơn vị (vd thuế bảo vệ môi trường theo lít, TTĐB theo đơn vị) phát sinh theo số lượng hàng của từng
dòng; chia theo số tiền làm dòng số lượng 0 mang thuế theo đơn vị, và dồn phần dư làm tròn vào nó làm tổng khác đơn
giá × số lượng — hoá đơn ghi sai. Chia giá chưa thuế theo số tiền khi có thuế theo đơn vị làm phần còn lại của dòng lệch
lớn, nên chia theo giá trị chính xác của dòng. Hoán đổi 2×2 thuần có thể kẹt dù có lời giải (cần dời qua ba dòng) — từ
chối chứng từ hợp lệ là sai. Trần thuế suất chặn lỗi nhập liệu làm tràn số (2,4e20% từng trả `money.overflow`).

Đã cân nhắc: làm tròn thuế theo đơn vị riêng từng dòng rồi cộng lại thành tổng nhóm — trái nguyên tắc làm tròn một lần
trên tổng nhóm (D-341) và đổi tổng chứng từ khi số lượng lẻ; giải toàn bộ bằng max-flow từ đầu — đúng nhưng bỏ mất
tỉ lệ của dòng và đổi số của chuỗi không có thuế theo đơn vị; đường tăng chỉ chạy khi hoán đổi 2×2 không được.
