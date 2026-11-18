## D-20261118-step42-stock-model · Mô hình tồn của `inventory`: `on_hand` / `available`, sổ chỉ thêm, giá vốn bình quân làm tròn nửa lên, nút giữ tồn — 2026-11-18 — agent quyết

Chủ đề: Thương mại

Quyết định:

1. **Hai số trên mỗi dòng tồn (biến thể × kho)**: `on_hand` = tổng sổ biến động của dòng; `available` = `on_hand` −
   chỗ đang giữ. `reserved` không lưu, suy ra `on_hand − available`. Cả hai chỉ đổi bằng `rows::adjust` trong
   `modules/inventory/src/stock.rs` (một câu `UPDATE … SET c = c + Δ WHERE …` có điều kiện). `on_hand` không bao giờ
   âm (`Bounds::NON_NEGATIVE`); `available` được âm khi hàng ra khỏi kho lúc đã giữ hết (vd hàng hỏng) hay khi sản
   phẩm cho bán khi hết — chỉ chặn tràn.
2. **Sổ `inventory_movements` chỉ thêm**: không câu `UPDATE` / `DELETE` nào trên bảng này; sửa sai bằng một dòng
   điều chỉnh mới có lý do. Test của module kiểm điều này trên mã nguồn.
3. **Giá vốn bình quân gia quyền theo dòng tồn** (biến thể × kho): chỉ hàng vào CÓ giá (nhập hàng, chuyển kho tới,
   điều chỉnh tăng có `unit_cost` — vd tồn đầu kỳ) mới tính lại `(tồn × giá cũ + nhập × giá nhập) / (tồn + nhập)`,
   tính bằng `i128`, **làm tròn nửa lên xa số 0** về đơn vị nhỏ nhất. Tồn ≤ 0 trước khi nhập thì giá vốn = giá nhập.
   Hàng ra, trả về, tìm thấy không có giá không đổi giá vốn. Khác tiền tệ khi đang có tồn → `inventory.currency_mismatch`.
   Dòng tồn bị khoá (`rows::lock_one`) trước khi đọc giá cũ. Giá vốn đổi → event `inventory.cost_changed` (`catalog`
   nghe cho sàn giá, D-222).
4. **Nút giữ tồn**: nút gốc của khách (cửa hàng đơn lẻ giữ tồn ở gốc) và nút loại `warehouse` hay `store`
   (`OrgDirectory::unit`). Không có `organizations` thì không kiểm. Nút chưa từng có tồn thì tính vào hạn mức
   `inventory.warehouses` và cần tính năng `inventory.multi_warehouse` khi khách đã có kho khác.
5. **Event gộp theo dòng tồn trong mỗi transaction**: một `inventory.level_changed` (kèm `before` / `after`) mỗi dòng,
   `low_stock` khi `available` đi từ trên xuống ≤ `reorder_point`, `out_of_stock` khi đi từ trên 0 xuống ≤ 0.
6. **Lệnh điều chỉnh có bảng riêng** `inventory_adjustments` (`reference` duy nhất theo khách, D-184) để gửi lại an
   toàn; dòng sổ trỏ về nó bằng `source_type = adjustment`.

Lý do: tách `available` khỏi `on_hand` cho phép giữ chỗ bằng một câu có điều kiện (không bán quá tồn dù nhiều request
cùng lúc) mà vẫn đúng sổ kế toán; số học nguyên `i128` tránh thêm `rust_decimal` vào crate và không tràn với mọi số
`i64`; cửa hàng đơn lẻ không phải dựng nút kho mới chỉ để có tồn.

Đã cân nhắc: chỉ một số `quantity` và tính chỗ giữ lúc đọc — giữ chỗ phải đọc rồi ghi, cần khoá; giá vốn theo cả
khách thay vì theo kho — không đúng khi hai kho nhập giá khác nhau (báo cáo lãi gộp theo cửa hàng sai); `rust_decimal`
cho giá vốn — kết quả như nhau, thêm một phụ thuộc.
