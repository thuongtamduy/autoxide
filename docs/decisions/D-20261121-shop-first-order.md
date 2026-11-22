## D-20261121-shop-first-order · Thứ tự sau bước 70 P1–P3 xếp theo kịch bản "cửa hàng bán online chạy thật" — 2026-11-21 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: chủ dự án giao agent xếp lại thứ tự sau P1–P3 của bước 70 theo một kịch bản dùng thật. Agent chọn kịch
bản **một cửa hàng bán online ở Việt Nam** (khớp cách giới thiệu ở D-20261121-community-first và các nhà cung cấp đã
chọn ở D-20261121-vn-providers). Thứ tự:

1. **Bán được hàng thật.** Bộ nối nhà cung cấp (bước 70 P7); hai mục còn mở của bước 45 không phụ thuộc bước 67 (tạo
   tài khoản từ số điện thoại đã xác thực qua tra đơn vãng lai; báo giá giỏ trả thêm điểm tối đa và cách trả dùng
   được); rồi bước 70 P8 — một luồng bán hàng mẫu chạy trọn trong test (sản phẩm → giỏ → đơn → thanh toán → vận chuyển
   → hoá đơn → tin nhắn, qua máy chủ giả của nhà cung cấp, trên cả hai database) kèm hướng dẫn "mở một cửa hàng".
2. **Mặt tiền cửa hàng.** Bước 67, phần cửa hàng cần trước: sổ khối và các khối của `catalog`, `promotions`, `orders`
   (các mục "chờ sổ khối" của 41, 43, 45); sau đó phần còn lại của 67 (AI tạo trang, bộ giao diện).
3. **Phân quyền (54)**, mục cửa hàng cần trước: xác thực lại cho việc nhạy cảm, quyền theo trường dữ liệu; rồi các mục
   còn lại.
4. **Quản trị nâng cao (52).**
5. **Bản 1.0**: bước 70 P6 (rà soát bảo mật nội bộ) rồi P4 (cam kết tương thích).

Mục còn mở nhưng phụ thuộc bước tạm dừng hay mốc thời gian không chặn 1.0 và giữ nguyên chỗ: P17 của 51 (chờ 56),
banner gian hàng người bán của 55 (chờ 46), gỡ menu cũ của 68 (sau 2027-05-14), cache quyền của 54 (chỉ làm khi số đo
cho thấy cần).

Lý do: phần nào có người dùng thật thì hoàn thiện trước; chỗ vướng khi chạy trọn một luồng bán hàng cho biết nên sửa
gì tiếp. Chưa có sản phẩm thật nào chạy trên Autoxide, nên luồng mẫu trong test là thứ gần nhất với "dùng thật" mà
agent tự làm được. Chủ dự án có sản phẩm cụ thể khác (website nội dung, SaaS nhiều khách) thì ghi mục mới thay mục này.

Đã cân nhắc: giữ thứ tự chung (đuôi module → 54 → 52 → 67) — loại: trang dựng từ khối nằm cuối trong khi các khối
thương mại của 41, 43, 45 đều chờ nó. Xếp theo website nội dung hay SaaS — loại: không khớp các nhà cung cấp chủ dự
án vừa chọn (thanh toán, vận chuyển, hoá đơn).

Thay: phần "sau P1–P3" (mục 3) của D-20261121-community-first
