## D-20261119-step42-preorders · Hàng đặt trước của `inventory`: bộ đếm suất có trần, job chia hàng về tính lại từ trạng thái — 2026-11-19 — agent quyết

Chủ đề: Thương mại

Quyết định:

1. **Suất** của biến thể `sale_mode = preorder` có `preorder_limit` (D-204, bước 41 khai) nằm ở
   `inventory_preorder_counters` (một dòng mỗi khách × biến thể). Giữ chỗ tăng `reserved` bằng một câu `UPDATE … WHERE
   reserved + q <= limit` (`rows::adjust` với trần là giới hạn), trong cùng transaction với giữ chỗ, trước khi trừ tồn;
   0 dòng thì `inventory.preorder_sold_out` (409). Giới hạn chép lại từ `catalog` mỗi lần giữ, nên người bán hạ giới hạn
   thì chỉ chặn đặt mới, không huỷ đơn đã giữ. Trả chỗ (huỷ, hết hạn, trả tay) trừ lại phần chưa xuất. Biến thể không có
   giới hạn thì không có bộ đếm.
2. **Hàng về**: mọi biến động làm `on_hand` của một dòng tồn tăng (nhập, chuyển đến, trả về, điều chỉnh tăng) xếp job
   `inventory.allocate_preorders` khi dòng tồn đó còn dòng đặt trước chưa đủ hàng. Job khoá dòng tồn, tính
   `hàng rảnh = on_hand − phần còn lại của dòng thường − phần đã chia chưa xuất của dòng đặt trước`, rồi chia cho các dòng
   đặt trước **theo thứ tự đặt** (ID dòng UUIDv7), cộng `allocated_quantity` và phát `inventory.preorder_allocated` cho
   từng phần.
3. Job **tính lại từ trạng thái** thay vì khoá theo ID biến động: chạy lại, chạy thừa hay hai job cùng dòng tồn (khoá
   dòng tồn xếp hàng chúng) không chia hai lần.
4. Khi tách khách (relocation), bộ đếm suất chép sang khách mới (`copy_on_split`) vì suất là của biến thể, không của
   tổ chức.

Lý do: đọc rồi ghi để kiểm giới hạn sẽ bán vượt suất khi nhiều người đặt cùng lúc (test 100 người, giới hạn 10 → đúng 10,
trên cả SQLite và Postgres). Tính lại từ trạng thái đơn giản hơn giữ sổ "biến động nào đã chia" và tự sửa nếu job lỗi giữa
chừng.

Đã cân nhắc: đếm suất bằng `SUM` các dòng đặt trước lúc giữ — phải khoá cả bảng dòng giữ chỗ; chia hàng ngay trong
transaction nhập kho — làm transaction nhập kho dài và phụ thuộc số đơn đang chờ.

Thay: không.
