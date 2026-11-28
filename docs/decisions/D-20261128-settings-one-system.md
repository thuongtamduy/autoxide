## D-20261128-settings-one-system · Một hệ cài đặt theo khách: một dịch vụ của kernel cho cả cấp khách và cấp nút, luật chéo lúc lưu, một cache — 2026-11-28 — agent quyết

Chủ đề: Đa khách, tổ chức, gói

Quyết định: Hai cơ chế cài đặt theo khách — cài đặt cấp khách (`TenantSetting`, bảng `kernel_tenant_settings`) và cài đặt
theo cây tổ chức (`OrgSetting`, giá trị ở bảng `organizations_setting`) — thành **một hệ do kernel giữ**: một dịch vụ
(`SettingsService` của kernel, công bố cho module bằng kiểu `tenancy::SettingSystem`), một cache, một sổ luật chéo. Hai
**phạm vi** của cùng một hệ, không phải hai hệ:

1. **Quy tắc chọn phạm vi** (ghi để agent sau khỏi phân vân): cấu hình kỹ thuật và kết nối đối tác của cả khách (địa chỉ
   site, máy chủ mail, khoá nhà cung cấp, link mẫu, ngôn ngữ của từng module…) là cài đặt **cấp khách** — có secret, có
   nhóm, đặt bởi `kernel.settings.manage` ở gốc. Chính sách nghiệp vụ có thể khác nhau theo cửa hàng/chi nhánh (múi giờ,
   tiền tệ, chính sách đổi trả, đăng ký tài khoản…) là cài đặt **theo cây** — kế thừa từ nút cha, đặt bởi
   `organizations.settings.write` phủ nút. Một khoá chỉ ở một phạm vi; tên khoá không trùng giữa hai phạm vi (kiểm lúc dựng
   `Modules`).
2. **Đọc qua một chỗ, có cache.** `SettingSystem::effective(tenant, nút, khoá)` / `effective_all` trả giá trị hiệu lực
   của cài đặt theo cây (nút gần nhất trên chuỗi tổ tiên đặt nó, không thì mặc định). Kernel không đọc bảng của module
   (luật kiến trúc 1): `organizations` cắm kho giá trị của nó vào hệ bằng trait `OrgSettingStore`
   (`SettingSystem::use_org_store`), kernel chỉ gọi `stored(tenant)` (một câu: mọi giá trị của khách). Cache theo khách
   giữ giá trị đã đặt và chuỗi tổ tiên của từng nút đã hỏi, sống `AUTOXIDE_TENANCY_CACHE_SECONDS` như cache cài đặt cấp
   khách; lần đọc trúng cache không chạy câu nào (trước: hai câu mỗi lần đọc — chuỗi tổ tiên và giá trị — kể cả mỗi lần
   làm mới phiên đăng nhập). `OrgDirectory::setting` của `organizations` (40 chỗ đọc ở 14 module) đọc qua hệ nên mọi chỗ
   đọc được cache ngay, không phải sửa; module mới đọc thẳng `SettingSystem`, không phụ thuộc `organizations`.
3. **Cache được làm mới khi ghi.** Ghi trên máy chủ này (đặt/bỏ cài đặt cấp khách qua kernel, cài đặt theo cây qua
   `organizations` gọi `SettingSystem::changed` sau commit) xoá cache của khách ngay; máy chủ khác thấy trong thời gian sống
   của cache (như cài đặt cấp khách trước nay). Dời nút trong cây làm chuỗi tổ tiên đã cache cũ đi tối đa chừng đó thời
   gian. Kiểm quyền theo cây (`require_at`) không qua cache này, **nhưng** vài cài đặt theo cây là chính sách có liên
   quan quyền — `auth.support_access`, `auth.staff_registration`, `auth.customer_registration`,
   `catalog.price_change_requires_second_person`, `conversations.require_verified_account` — trước đọc thẳng database,
   nay qua cache như mọi cài đặt cấp khách (vd `auth.step_up`) đã qua từ trước: trên máy chủ nhận lần lưu, giá trị mới
   có hiệu lực ngay; trên máy chủ khác trễ tối đa `AUTOXIDE_TENANCY_CACHE_SECONDS`. Chấp nhận cùng mức với cài đặt cấp
   khách; khách cần hiệu lực tức thì trên nhiều máy chủ thì đặt thời gian sống thấp. Lần đọc trượt cache không chèn bản
   cũ đè lần ghi vừa xảy ra (số thế hệ theo khách: bản đọc chỉ được giữ khi không có lần ghi nào trong lúc đọc). Tách / gộp
   khách (chuyển dòng cài đặt giữa hai khách) xoá cache của cả hai khi lượt chạy kết thúc.
4. **Luật chéo lúc lưu** (`SettingRule`). Module đăng ký trong `start` một luật nêu các khoá nó ràng buộc (của một hay cả
   hai phạm vi) và một hàm kiểm nhận `SettingsView` (giá trị **sau** thay đổi đang lưu: cấp khách dạng chữ, theo cây là
   giá trị hiệu lực tại nút đang đổi — đổi cấp khách thì tại nút gốc). Lưu một khoá có luật mà vi phạm → 422
   `validation_failed`, trường `value`, câu nói rõ phải sửa gì; dữ liệu không đổi. Luật chạy trong transaction ghi, sau một
   khoá theo khách (`kernel.settings:<khách>`, advisory lock trên Postgres; SQLite vốn một người ghi), nên hai lần lưu đồng
   thời không cùng lọt luật rồi để lại cặp giá trị sai. Phần cấp khách của view đọc trong transaction; phần theo cây khi lưu
   cấp khách đọc giá trị đã commit (luật chéo hai phạm vi có thể lọt nếu một lần lưu theo cây đang chạy song song — chỗ đọc
   lúc dùng vẫn giữ đường lùi như cũ). Luật không xem được giá trị secret. Giá trị đã lưu từ trước mà nay vi phạm vẫn đọc như
   cũ (đường lùi lúc dùng giữ nguyên); chỉ lần lưu mới bị chặn. Luật đầu tiên: `menus.default_locale` ∈ `menus.locales`,
   `content.default_locale` ∈ `content.locales` (danh sách chưa đặt thì so với danh sách của máy chủ). Chỉ kiểm giá trị **đã đặt**: đổi bộ ngôn ngữ thì bỏ mặc định, đổi danh sách, đặt lại mặc định — luôn có đường
   đi qua các trạng thái hợp lệ.

Đường nâng cấp: chuyển bảng giá trị theo cây vào kernel (một bảng `kernel_…`) sau này chỉ đổi chỗ cắm `OrgSettingStore`,
không đổi chỗ đọc. `OrgDirectory::setting` thành đường cũ khi module đọc đã chuyển sang `SettingSystem` (theo các cụm của
D-20261128-library-layer-plan). Module ngoài repo: chỉ thêm (kiểu mới, hàm mới); hợp đồng cũ (`TenantSettings`,
`OrgSetting`, `OrgDirectory`) giữ nguyên.

Lý do: mục lộ trình 71 và ba mục "Chỗ trong core làm cấu hình theo khách khó" (hai cơ chế song song không quy tắc, không
kiểm chéo lúc lưu, cài đặt theo cây không cache). Gộp hai bảng ngay là chuyển dữ liệu giữa module (kernel không ghi được bảng
của `organizations`) và đổi hai API đang dùng; một dịch vụ với hai phạm vi cho đúng điều chủ dự án cần — một chỗ đọc, một
chỗ kiểm, một cache — mà không phá dữ liệu hay hợp đồng.

Đã cân nhắc: chỉ ghi quy tắc chọn phạm vi (không giải quyết cache và kiểm chéo); gộp về một bảng của kernel ngay (chuyển
dữ liệu xuyên module, đổi API `organizations`); cache riêng trong `organizations` (thêm một cơ chế cache thứ hai, trái
"một cách làm cho mỗi việc"); lưu nhiều khoá một lần để kiểm chéo (endpoint mới; đường "bỏ – đổi – đặt lại" đủ cho mọi
chuyển đổi, để sau nếu agent cần).
