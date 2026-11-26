## D-20261126-one-settings-system · Một hệ cài đặt theo khách ở kernel: phạm vi khách hoặc kế thừa theo cây, kiểm chéo lúc lưu, có cache — 2026-11-26 — agent quyết

Chủ đề: Đa khách, tổ chức, gói

Quyết định: `TenantSetting` và `OrgSetting` thành một kiểu `Setting` của kernel có `SettingScope::Tenant` hoặc
`SettingScope::Inherited`. Giá trị lưu ở `kernel_tenant_settings` thêm cột `org_id` (NULL = mức khách = nút gốc). Đọc: nút
gần nhất trên chuỗi tổ tiên (qua trait `OrgTree` do `organizations` cung cấp) → mức khách → mặc định → cấu hình chung; cache
theo khách như hiện nay. Lưu: checker của cài đặt + phạm vi + quyền tại nút. `organizations` chuyển dữ liệu của bảng mình
bằng `DataUpgrade` ghi qua API cài đặt của kernel; endpoint cũ thành lớp mỏng rồi deprecate (D-059). Khai bằng
`manifest.setting(…)` (D-20261126-module-manifest).

Lý do: hai hệ song song (một ở kernel có cache và secret, một ở `organizations` có kế thừa) cho cùng một nhu cầu "cấu hình
theo khách"; module phải biết dùng cái nào, giao diện phải gọi hai API, và cài đặt kế thừa không có cache lẫn secret.

Đã cân nhắc: chuyển hết sang `organizations` (kernel cần đọc cài đặt khi module đó tắt); giữ hai hệ, chỉ ghi tài liệu (không
đạt "một cách làm"). Thiết kế đầy đủ, API mẫu và thứ tự PR: `docs/research/kernel-contract-1-0.md`.

Thay: phần "giá trị lưu và phục vụ ở module `organizations`" của thiết kế cài đặt kế thừa bước 34 (ghi ở `tenancy/org_settings.rs`)
