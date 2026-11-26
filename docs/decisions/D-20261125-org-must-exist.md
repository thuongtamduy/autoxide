## D-20261125-org-must-exist · Quyết định quyền theo đơn vị đòi đơn vị phải có trong cây của khách — 2026-11-25 — agent quyết

Chủ đề: Đa khách, tổ chức, gói

Quyết định: hàm quyết định duy nhất cho một bản ghi (`Reach::covers`, sau `Principal::check` / `require_at` /
`require_write_at`) chỉ cho phép khi đơn vị đích đã được chứng minh là của khách: nằm trong grant của người gọi, trong
nhánh của grant `subtree` / fence (tra closure như cũ), hoặc — khi thứ cho phép là quyền toàn khách hay `own` theo chủ bản
ghi — qua `OrgTree::contains` (nút gốc của khách, hay một nút có dòng closure của chính nó). Đơn vị không có trả cùng
lỗi như ngoài phạm vi: 403 `org_out_of_scope` (đọc: service trả 404 như cũ). Kernel thêm `OrgTree::contains` và
`OrgTree::ensure_exists` (404 `org_not_found`) cho chỗ nhận đơn vị ngoài một quyết định quyền: nhập / xuất catalog,
nháp sản phẩm từ ảnh, nháp chiến dịch của `content`, tạo giỏ công khai của `orders`, báo có hàng công khai của
`inventory`. Sandbox: `role_id` không có ở khách thì không uỷ quyền gì (trước đây rơi về vai trò `admin`).

Lý do: pentest 2026-11 (PT-8) — người có quyền toàn khách tạo được menu, sản phẩm, ca POS, cài đặt theo đơn vị… gắn
với ID đơn vị bịa hay của khách khác (201): grant toàn khách "bao" mọi ID mà không ai tra cây. Rà soát thấy khoảng 50
chỗ nhận `org_id` mà không tự kiểm; sửa ở hàm quyết định thì mọi chỗ đi qua `require_at` / `require_write_at` (REST và
MCP) được sửa một lần, module mới không phải nhớ. Đơn vị không bao giờ bị xoá khỏi cây (chỉ chuyển trạng thái), nên bản
ghi cũ không mất quyền đọc. Chi phí: một truy vấn closure theo khoá chính (tenant, descendant) cho người có quyền toàn
khách khi đích không phải nút gốc; khách chưa dựng cây (mọi bản ghi ở nút gốc) không thêm truy vấn nào.

Hệ quả cho code tự dựng `Principal` (job, module ở repo khác): `Principal` dùng để làm việc ở một khách phải mang khách
đó (`.in_tenant(tenant.id())`); trước đây quyền toàn khách không cần biết khách nên quên cũng chạy. Đã sửa job sao
chép vào sandbox và job dịch của `content`, cùng các test dựng `Principal` của khách mặc định rồi dùng ở khách khác.

Đã cân nhắc: sửa từng service (hơn 50 chỗ, lặp lại ở module sau); chỉ kiểm trong `ensure_writable` (bỏ sót chỗ chỉ dùng
`require_at`, vd mở ca POS); trả 404 `org_not_found` riêng cho trường hợp này (phải thêm biến thể vào enum công khai
`Decision`, đổi hợp đồng; 403 như người ngoài phạm vi cũng không lộ thêm gì).
