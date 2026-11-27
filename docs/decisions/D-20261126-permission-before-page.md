## D-20261126-permission-before-page · Handler danh sách kiểm quyền trước tham số trang, gate `permission-first` giữ luật — 2026-11-26 — agent quyết

Chủ đề: HTTP, REST, OpenAPI

Quyết định: tiếp mục 6 của D-20261126-list-pages-done. Mọi handler danh sách gọi kiểm quyền **trước** khi đọc tham số
trang (`PageRequest::try_from`, `….into_request`): người gọi thiếu quyền gửi `limit=0` nhận 403, không phải 422.
Đợt này sửa 28 handler (`auth`, `bookings`, `catalog`, `compliance`, `content`, `files`, `invoices`, `ledger`,
`menus`, `orders`); handler gọi đúng kiểm mà service sẽ làm đầu tiên (`principal.require(&P)?`, `require_any` cho
`bookings` vì service nhận một trong ba permission, `has` + `require` cho khoá API của mọi người dùng để người duyệt
không bị hỏi step-up của quyền quản lý; `passkeys` dời `check_admin` lên trước). Service vẫn tự kiểm (phạm vi, bản
ghi), REST và MCP dùng chung service như cũ.

Luật `permission-first` của `cargo xtask check` (`xtask/src/permission_first.rs`): hàm có `#[utoipa::path]` đọc trang
mà trước đó chưa có câu lệnh trọn nhắc `principal` và có `?` / `return Err` thì vi phạm, nếu operation khai
`x-permissions` khác rỗng (trừ `x-buyers: true`) hoặc thân hàm gọi `principal.require…(…)?` sau chỗ đọc trang. Miễn
trừ: `// permission-first: <lý do>` trên `fn`. Không có file danh sách cho phép: cây hiện tại không còn vi phạm.

Lý do: lớp `x-permissions` của kernel (D-20261126-permission-gate) chỉ hỏi "có permission ở một nút nào đó" và không hỏi
step-up; người qua được lớp đó mà service vẫn từ chối (permission cần xác nhận lại, danh sách "một trong" mà service
cần một cái cụ thể, route dựng không kèm lớp này trong test) thấy 422 trước 403 khi handler đọc trang trước. Kiểm ở
handler làm thứ tự đúng ngay cả không có lớp đó, và gate giữ cho handler mới.

Đã cân nhắc: chuyển việc đọc trang vào service — loại: đổi chữ ký của rất nhiều service và tool MCP; một hàm kernel
công khai "có một trong các permission, không step-up" cho handler — loại: thêm hợp đồng công khai cho việc mà các kiểm
sẵn có đã làm được; luật tìm theo tên handler `list_*` — loại: bỏ sót `account_lines`, `staff_reviews`. Giới hạn của
luật: đọc trang trong hàm phụ khác hai mẫu trên, và handler `x-permissions: []` để service kiểm sau khi đọc trang,
không được nhận ra.
