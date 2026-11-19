## D-20261119-step42-preorder-cancel · `inventory` trả riêng phần đặt trước bị huỷ của một đơn còn hiệu lực, chống trùng bằng bảng tham chiếu — 2026-11-19 — agent quyết

Chủ đề: Thương mại

Quyết định:

1. `inventory` nghe `orders.preorder_cancelled` (bằng `Subscription::optional`; `after`: `order_id`, `lines[{line_key,
   quantity}]`, cùng `reason`, `amount` mà inventory không đọc). Mỗi dòng trả `quantity` đơn vị của dòng đơn `line_key`
   (combo nhân `per_unit` cho từng thành phần) khỏi chỗ giữ: cộng lại `available`, cộng `released_quantity`, dòng đặt
   trước trả suất (`preorders::give_back`). Chỗ giữ vẫn mở; `release` cả đơn sau đó chỉ trả phần còn lại. Huỷ quá phần
   còn lại thì chỉ trả phần còn; đơn không có chỗ giữ hay chỗ giữ đã đóng thì bỏ qua.
2. **Chống trùng bằng bảng** `inventory_line_releases` (khoá duy nhất `tenant_id, reference`; `reference` =
   `orders.preorder_cancelled:<ID event>`), chèn trong cùng transaction với phần trả và sau khi khoá chỗ giữ. Không so
   tổng đã huỷ theo (`order_id`, `line_key`): event chỉ mang phần huỷ của lần đó (không cộng dồn), nên hai lần huỷ
   bằng nhau và một event giao lại trông giống hệt nhau nếu chỉ nhìn số.
3. Relocation: bảng đi theo chỗ giữ (`follow` `inventory_reservations`).

Lý do: event được giao ít nhất một lần (D-054); bảng tham chiếu là cách duy nhất phân biệt "giao lại" với "huỷ thêm
lần nữa" khi payload không cộng dồn. Khoá duy nhất cũng chặn hai lần giao chạy đồng thời, trên cả SQLite và Postgres.

Đã cân nhắc: đổi `Inventory::release` (đã chốt chữ ký, không đổi); xin `orders` gửi số cộng dồn — được thì bỏ được bảng,
nhưng buộc `orders` giữ thêm trạng thái chỉ để phục vụ inventory.

Thay: không.
