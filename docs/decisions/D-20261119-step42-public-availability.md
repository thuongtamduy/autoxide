## D-20261119-step42-public-availability · Tình trạng hàng công khai của `inventory`: tính như giữ chỗ, bỏ qua biến thể ẩn, cache 5 giây — 2026-11-19 — agent quyết

Chủ đề: Thương mại

Quyết định:

1. "Tồn bán được ở cửa hàng" tính đúng như khi giữ chỗ: các kho nguồn của cửa hàng (`inventory_sources`, hay chính cửa
   hàng); `allow_split` bật thì cộng các kho, không thì lấy kho nhiều nhất. Combo = số bộ ráp được từ thành phần theo
   dõi tồn. Biến thể không theo dõi tồn luôn `in_stock`. `available` âm (bán khi hết, đặt trước) tính là 0.
2. Mức: `≤ 0` → `preorder` nếu bán theo `preorder`, không thì `out`; `< PUBLIC_LOW_THRESHOLD` → `low` kèm số; còn lại
   `in_stock` không số (D-200). Biến thể bán theo `backorder` hết hàng hiện `out` (vẫn đặt được, nhưng không hứa có sẵn).
3. Biến thể không có, chưa bán hay đang ẩn bị **bỏ qua** thay vì báo lỗi: người lạ không dò được ID nào tồn tại.
4. `Inventory::in_stock_at` (bộ lọc của `catalog`) dùng cùng hàm, "còn hàng" = `in_stock` hoặc `low`.
5. `ETag` là dấu vân tay của câu trả lời (kèm ID khách); `Cache-Control: public, max-age=5`. Không dùng phiên bản sổ
   biến động làm `ETag`: giữ chỗ đổi `available` mà không ghi sổ.

Lý do: khách thấy "còn hàng" thì giữ chỗ phải được — hai đường phải tính giống nhau.

Đã cân nhắc: cộng mọi kho bất kể `allow_split` — sẽ báo còn hàng cho đơn mà giữ chỗ từ chối.

Thay: không.
