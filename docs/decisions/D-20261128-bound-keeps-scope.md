## D-20261128-bound-keeps-scope · Luật tự động hoá bị giới hạn theo cả phạm vi lúc lưu, không chỉ theo tên permission — 2026-11-28 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: kernel thêm `Principal::bounded_by(tenant, limit, catalog)`: giữ của người gọi đúng phần mà bản chụp `limit`
cũng có — theo từng permission, với mỗi cặp grant (nút, độ sâu) của hai bên, grant này nằm trong grant kia thì giữ grant
hẹp hơn, không lồng nhau (hai nhánh rời) thì bỏ. Vùng trong cây hoặc lồng nhau hoặc rời nhau, nên "grant hẹp hơn của hai
grant lồng nhau" đúng là phần giao. Permission không `scoped` có hiệu lực toàn khách bất kể grant: có ở cả hai bên là
giữ grant hiện tại; tên danh mục không biết coi là `scoped` (an toàn hơn). Chỉ đọc cây tổ chức khi một bên có grant
`subtree` ở nút khác nút của bên kia. `automations` dùng hàm này thay `Principal::only` khi dựng quyền chạy luật từ
`run_as_limit`: luật không bao giờ có hơn quyền lúc tạo / sửa / bật, kể cả khi người `run_as` được nới phạm vi sau đó.
Luật cũ (cột NULL) giữ cách cũ như trước. Hàm mới nhận `&TenantContext`, không lộ kiểu SeaORM ra hợp đồng công khai.

Lý do: P16 — `Principal::only` giữ grant HIỆN TẠI của tên permission, nên người quản lý chi nhánh A lưu luật rồi được
nâng lên quyền toàn khách thì luật cũng chạy toàn khách. Bản chụp đã lưu cả grant (`PrincipalSnapshot`), không cần
migration.

Đã cân nhắc: chụp quyền lúc tạo và chạy hẳn bằng bản chụp (người bị hạ quyền thì luật vẫn giữ quyền cũ — sai chiều);
làm phần giao trong `automations` (phải dựng lại `Principal` và mất các thuộc tính riêng như rào nút, lệnh cấm).

Thay: phần "Giới hạn theo TÊN permission" ở mục 3 của D-20261126-automation-owner-revoked
