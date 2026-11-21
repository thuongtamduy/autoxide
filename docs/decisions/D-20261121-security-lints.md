## D-20261121-security-lints · Hai luật gate chặn lỗi phân quyền tái phát: `tenant-scope` và `mcp-principal`, kèm danh sách chỗ cũ chờ sửa — 2026-11-21 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. **`tenant-scope`** (`xtask/src/tenant_scope.rs`): trong `src/` của kernel và module, mỗi lời gọi
   `<entity>::{find, find_by_id, update_many, delete_many, delete_by_id}(` trên entity cấp khách (crate
   có `impl TenantScoped for` nó) phải có `.scoped(` trong cùng biểu thức — hoặc tự lọc cột `TenantId`,
   hoặc khoá chính bắt đầu bằng khách (`find_by_id((tenant…, id))`), hoặc gán vào `let q = …;` rồi `q….scoped(`
   trong cùng khối. Chỗ cố ý quét mọi khách (job dọn dữ liệu…) ghi `// tenant-scope: <lý do>` ngay trên
   câu lệnh.
2. **`mcp-principal`** (`xtask/src/mcp_principal.rs`): `Tool::new(…)` có closure bỏ qua tham số
   `Principal` (tên `_…` hay không dùng) mà `.requires(P)` với `P` là permission crate đó giới hạn theo
   phạm vi (`&P` trong đối số `scope_condition(` / `require_at(`) thì bị chặn, trừ khi ghi
   `// mcp-principal: <lý do>` trên `Tool::new`.
3. Cả hai đọc mã nguồn dạng văn bản (đã che comment, chuỗi, phần `#[cfg(test)]` — `xtask/src/source.rs`),
   không dùng `syn`. Chú thích miễn trừ phải có lý do (ít nhất vài ký tự sau dấu `:`).
4. Chỗ vi phạm có sẵn lúc luật vào gate ghi ở `xtask/security-baseline.txt`: `<luật> <số chỗ> <file>`
   kèm số dòng và lý do trong comment. Gate chặn khi file có **nhiều** chỗ hơn số ghi; ít hơn hay hết
   thì chỉ cảnh báo nhắc hạ số / xoá dòng.

Lý do: đợt rà soát bảo mật tháng 11/2026 tìm thấy nhiều lần hai lỗi cùng dạng: quên `.scoped(&tenant)`
(lộ dữ liệu giữa các khách) và tool MCP rộng tay hơn REST cùng permission (trái luật kiến trúc 8).
Trình biên dịch không bắt được cả hai; review bằng mắt đã sót. Danh sách chỗ cũ ghi theo **số chỗ
mỗi file** chứ không theo số dòng vì ba phiên khác đang vá chính các file đó: dòng xê dịch không được
làm gate của họ đỏ, và sửa bớt chỉ cảnh báo để họ không phải chạm file của xtask.

Đã cân nhắc:
- Phân tích bằng `syn`: chính xác hơn về cú pháp nhưng vẫn không biết kiểu (alias, generic), thêm thư
  viện cho xtask; luật văn bản cùng cách với `boundaries.rs` / `tenancy.rs` đủ bắt dạng lỗi đã gặp.
- Lint của clippy (dylint): phải dựng toolchain riêng, nặng cho gate nhanh.
- Bắt mọi truy vấn trên mọi entity (không phân biệt `TenantScoped`): ~480 chỗ báo, phần lớn là bảng mặt
  điều khiển — quá nhiều báo nhầm. Tên entity mơ hồ (cùng tên, chỉ một phần cài `TenantScoped`) bị bỏ
  qua: thà sót còn hơn báo nhầm.
- Đối chiếu tool MCP với đúng handler REST tương ứng: không có liên kết máy đọc được giữa hai bên;
  dùng permission làm cầu nối là đơn giản và giải thích được.

Giới hạn đã biết: không theo truy vấn truyền qua hàm khác, không thấy `E::find()` generic, không kiểm
`Entity::update(model)` / `Entity::delete(model)`; permission truyền qua biến và handler MCP là hàm đặt
tên riêng không được nhận ra; luật 2 chỉ xét `principal` có được dùng, không xét dùng đúng chỗ.
