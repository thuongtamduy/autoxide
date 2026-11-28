## D-20261128-handle-bridge-by-cluster · Chỗ module chạy câu SeaORM trên handle của `db` chuyển sang `Record` theo cụm, không trong PR gỡ cầu — 2026-11-28 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định: cầu `ConnectionTrait` trên handle của `db` (`Read`, `Tx`) ở lại tới khi các chỗ module đang chạy câu
SeaORM tự dựng trên handle đã chuyển sang `Record`; việc chuyển đó làm trong PR cụm của từng module (thứ tự cụm ở
D-20261128-library-layer-plan mục 7), không trong PR làm `db::Executor`. Danh sách chỗ còn dùng cầu (28/11/2026): `auth`
`oauth/` (gồm `fetch_page` trên handle ở `grants.rs`), `ledger` `reconcile.rs` / `payment_refunds.rs` / `simulate.rs`,
`compliance` `phone_locks.rs`, `conversations` `voice/recordings.rs`, `orders` `payment_attempts.rs`. Không thêm chỗ dùng
mới. Khi chỗ cuối đã chuyển: xoá hai `impl ConnectionTrait` của `Read`/`Tx` trước, rồi mới thêm `impl Executor` riêng cho
chúng (còn cầu thì hai impl đó xung đột với impl chung của `Executor`).

Lý do: thử gỡ cầu ở #615 thì vỡ ở năm module trên. Đưa các chỗ đó về kết nối cũ (`tenant.database()`) phải nâng baseline
`raw-database`, mà baseline chỉ được giảm; chuyển chúng sang `Record` ngay là chuyển một phần của năm module đang được các
phiên khác sửa song song, PR to và dễ xung đột. Các câu đó vẫn tự lọc khách bằng `.scoped(tenant)`, nên hoãn không mở lỗ
rò dữ liệu; rủi ro là chỉ còn lời nhắc (lộ trình, comment), không gate nào bắt mẫu này.

Đã cân nhắc: chuyển sang `Record` trong cùng PR (mục 5 cũ — xem lý do trên); nâng baseline `raw-database` cho các chỗ đó
(trái luật baseline chỉ giảm); giữ cầu vô thời hạn (handle hứa lọc khách mà không giữ được lời hứa).

Thay: phần "Module đang chạy câu SeaORM trên handle chuyển các câu đó sang `Record` trong cùng PR" ở mục 5 của D-20261128-library-layer-plan
