## D-20261128-address-key-rule · Luật `address-key` đọc lời gọi nhiều dòng và các cách dựng khoá hay gặp — 2026-11-28 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: luật `address-key` của `cargo xtask check` (D-20261125-address-keys) đọc trọn lời gọi `format!(…)` và
`.hit(…)` kể cả khi xuống dòng (bỏ qua ngoặc trong chuỗi và comment), và chặn: `format!` dựng khoá (chuỗi mẫu không có
khoảng trắng, không phải URL) có `ip:{`, có chỗ điền mang tên địa chỉ (`ip`, `addr`, `*_ip`, `*_addr`, `ip_*` trừ
`ip_hash*`) hay nhận biến địa chỉ làm đối số (không tính lời gọi như `local_addr()`, đối số qua `ip_hash::` /
`address_key(` là đã gom); `<địa chỉ>.to_string()` trong đối số của `.hit(…)`; `let <tên có key> = <địa chỉ>.to_string()`.
Đánh dấu `// address-key: <lý do>` ở dòng ngay trên hay bất kỳ dòng nào của lời gọi.

Lý do: P17 — luật cũ chỉ bắt `format!("…ip:{…")` trên một dòng. Kiểm theo kiểu (khoá có kiểu riêng vào `Throttle::hit`)
chặt hơn nhưng đổi chữ ký công khai `Throttle::hit(&str)` và sửa mọi module — để tới khi có lý do đổi hợp đồng
(cần `#[deprecated]` trước).

Đã cân nhắc: chặn mọi `format!` có biến địa chỉ (bắt nhầm câu thông báo, URL); kiểm theo kiểu ngay (phá vỡ hợp đồng).
