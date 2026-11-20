## D-20261120-step39-checkout-batch · Đặt hàng ghi mọi dòng đơn và dòng thuế bằng `INSERT` nhiều dòng — 2026-11-20 — agent quyết

Chủ đề: Vận hành và hiệu năng

Quyết định: `POST /orders/checkout` và đơn bán tại quầy (POS) ghi dòng đơn rồi dòng thuế bằng `insert_many` (hàm
`checkout::insert_lines`), mỗi bảng một câu cho mỗi lô 200 dòng, thay vì một câu mỗi dòng và mỗi dòng thuế. Giữ bằng
test `orders` `tests::statements` (giỏ 2 dòng và 7 dòng: cùng số câu ở mọi bước mua).

Lý do: đếm bằng công cụ của D-20261120-step39-hot-paths, đặt hàng tăng một câu mỗi dòng (26 câu với 1 dòng, 31 câu với
6 dòng; có thuế thì hai câu mỗi dòng), tất cả nằm trong transaction ghi — trên SQLite là lúc giữ quyền ghi duy nhất, mọi
lệnh ghi khác xếp hàng chờ. Sau: 26 câu dù 1 hay 6 dòng, trên cả SQLite và Postgres. Mọi trường của hai kiểu dòng đều
`Set` nên `INSERT` nhiều dòng ghi đúng giá trị như trước (SeaORM điền `NULL` cho trường `NotSet` khi gộp — không có trường
nào như vậy). Lô 200 dòng × ~40 cột = 8 000 tham số, dưới giới hạn của SQLite (32 766) và Postgres (65 535).

Đã cân nhắc: một câu cho mọi dòng không chia lô — giỏ tối đa `AUTOXIDE_ORDERS_CART_MAX_LINES` dòng cộng quà tặng, cấu
hình nâng được nên có thể vượt giới hạn tham số; đo độ trễ đặt hàng qua máy chủ thật — cần dựng kho, vận chuyển, thanh
toán qua API, để lần đo đặt hàng riêng; số câu trong transaction ghi là thước đo trực tiếp của thời gian giữ quyền ghi.
