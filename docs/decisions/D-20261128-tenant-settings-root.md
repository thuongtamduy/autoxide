## D-20261128-tenant-settings-root · Đặt / bỏ cấu hình cấp khách cần `kernel.settings.manage` ở nút gốc — 2026-11-28 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: `PUT` / `DELETE /api/v1/kernel/settings/{name}` và tool `kernel.set_setting`, `kernel.delete_setting` gọi
`Principal::require_at_root(&SETTINGS_MANAGE)` (kernel, `security/access/decision.rs`): người gọi phải có
`kernel.settings.manage` qua một grant ở **nút gốc** của khách (hay toàn quyền), và không bị rào dưới gốc (API key gắn
nhánh). Có permission mà chỉ qua membership ở nhánh, hay API key gắn nhánh → 403 `org_out_of_scope`. Nhân sự nền tảng
vào khách bằng `kernel.tenants.operate` giữ grant ở gốc của khách nền tảng: được coi như ở gốc. Đường của nhân sự nền
tảng (`/kernel/tenants/{id}/settings…`, `kernel.tenants.manage`) không đổi. Đọc danh sách cấu hình giữ nguyên.

Audit: không thêm gì — mỗi lần đặt / bỏ đã ghi event `kernel.setting_changed` (người làm, tên cấu hình) cùng
transaction, module `audit` nhận mọi event. Event vẫn KHÔNG mang giá trị (D-258: event đi cả webhook ra ngoài).

Lý do: permission không `scoped` có grant ở nút nào cũng thành grant toàn khách (D-303 mục 3), nên quản lý một nhánh có
role chứa `kernel.settings.manage` đổi được cấu hình của cả khách như `kernel.business_model` (bật luật định danh người
bán của sàn, D-20261127-seller-kyc-listing). Cấu hình khách không thuộc nút nào, nên chỉ người làm ở gốc được đổi.

Đã cân nhắc: đánh dấu `kernel.settings.manage` là `scoped` rồi `require_at(root)` — đổi nghĩa của role đã lưu (grant ở
nhánh vốn có độ sâu `tenant`) và không chặn được grant độ sâu `tenant` ở nhánh; kiểm trong service — đường nhân sự nền
tảng dùng chung service với principal không có permission này.
