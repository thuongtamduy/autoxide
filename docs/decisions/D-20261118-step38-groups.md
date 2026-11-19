## D-20261118-step38-groups · Nhóm nút ngoài cây cho báo cáo: số nút bị bỏ ở chi tiết nhóm, `group_by=group` cần `group_ids` — 2026-11-18 — agent quyết

Chủ đề: Đa khách, tổ chức, gói

Quyết định: Bước 38 mục nhóm nút (D-232) làm theo đặc tả 38 (Luồng 5), với các điểm chốt sau.

1. **Bảng của `organizations`**: `organizations_group` (PK `tenant_id, id`; unique `tenant_id, name_folded` — tên không
   phân biệt dấu, hoa thường) và `organizations_group_member` (PK `tenant_id, group_id, org_id`; index `tenant_id,
   org_id`), migration `m0007_groups`. Không foreign key giữa hai bảng hay sang bảng nút: xoá nhóm xoá thành viên trong
   service, xoá / lưu trữ nút không xoá thành viên. Tách / gộp: nhóm `copy_on_split` (chép khi tách, nhập khi gộp),
   thành viên `ByOrg` theo `org_id`.
2. **Nhóm không cấp quyền**: quản lý cần `organizations.groups.manage` (không `scoped` — chỉ cấp ở độ sâu `tenant`);
   đọc cần `organizations.units.read`. `member_count` và danh sách thành viên chỉ gồm nút trong phạm vi đọc
   (`Principal::scope_condition`); số nút ngoài phạm vi là `excluded_org_count` của **`GET /groups/{id}` và `GET /groups`**
   — không đặt trong trang thành viên như đặc tả, vì response chỉ có ba hình dạng (`Data`, `Page`, `Problem`; CLAUDE.md).
3. **Thêm thành viên**: tối đa 200 nút mỗi lệnh, 1 000 nút mỗi nhóm; `INSERT … ON CONFLICT DO NOTHING` theo khoá chính
   nên gọi lại / gọi đồng thời không tạo trùng; lệnh thêm chạm dòng nhóm trước (khoá dòng trên Postgres) để trần 1 000
   đếm đúng khi đồng thời. Nút không thuộc khách → `organizations.group_member_invalid` (kiểm qua bảng nút của chính
   module).
4. **`reporting`** lấy `dyn OrgGroups` (tuỳ chọn): `group_ids` ở `/summary`, `/facts` và tool MCP; thành viên GIAO với phạm
   vi `reporting.read` tính trước bộ lọc `org_ids` của client; nhóm chỉ lọc số liệu của chính khách người xem (bỏ khách
   con). `excluded_org_count` ở `totals` (nút không trùng của mọi nhóm đã hỏi) và ở từng nhóm khi `group_by=group`;
   vắng khi không hỏi nhóm. `group_by=group` bắt buộc có `group_ids` (trait không liệt kê nhóm) — thiếu thì 422; nhóm
   không có → `reporting.group_not_found` (404). Một nút ở hai nhóm được tính ở cả hai, tổng chung tính một lần.

Lý do: giữ đúng ba hình dạng response và luật sở hữu bảng; `OrgGroups::members` một hàm là đủ cho `reporting`, không
cần thêm hàm liệt kê nhóm vào hợp đồng.

Đã cân nhắc: trang thành viên có thêm trường `excluded_org_count` (hình dạng response thứ tư); `group_by=group` không
`group_ids` thì lấy mọi nhóm (cần thêm hàm vào `OrgGroups`, và người một nhánh sẽ nhận một danh sách nhóm dài vô ích);
foreign key thành viên → nhóm (chính sách tách / gộp phải giữ cặp bảng đi cùng nhau).
