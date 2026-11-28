## D-20261128-manifest-defaults · Đường chuyển của manifest: hàm cũ đọc manifest, kernel bọc module sau `start` — 2026-11-28 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định: Cài đặt đường chuyển của D-20261126-module-manifest theo chiều ngược với §5.2 điểm 1 của tài liệu thiết
kế. `Module::manifest()` mặc định trả manifest "cũ" (không tên, không khai gì); mỗi hàm khai báo cũ của trait (`name`,
`permissions`, `events`, `migrations`…) mặc định **đọc từ `self.manifest()`**. Module kiểu mới chỉ cài `manifest`
(+ `configure`, `start`, `dormant`); module cũ ghi đè hàm cũ như trước. Thứ đăng ký trong `start`
(`StartContext::subscribe`, `routes`, `tools`, `job`, `task`, `check`, `prompt`, `mcp_resource`, `client_config`,
`seed_tenant`, `sample_data`, `sandbox_copy`) được giữ trong `StartContext`; sau `start` registry bọc module kiểu mới
trong một vỏ (`module/started.rs`) trả các thứ đó qua đúng các hàm "gọi sau start" mà kernel đang đọc. Module chưa cài
`manifest` mà gọi các hàm đăng ký mới thì khởi động lỗi; module cài `manifest` mà còn cài hàm cũ thì `cargo xtask
check` chặn (`module-contract-mixed`). Module còn theo đường cũ đếm ở `xtask/module-contract-legacy.txt`
(`module-contract-legacy`, chỉ giảm). Handle của kernel lấy bằng `StartContext::kernel::<H>()` (trait niêm phong
`KernelHandle`, mỗi handle cài bằng đúng getter cũ). `migrations`/`upgrades` của manifest là con trỏ hàm để dựng lại
manifest rẻ. Chưa làm ở bước này: `contribute::<dyn Trait>` thay ~20 hàm đăng ký riêng (`work_source`,
`personal_data`…), hook của `relocation`, feature `unstable` (manifest có sẵn `Manifest::unstable` không gắn `cfg`).

Lý do: kernel, app và testkit đọc các hàm cũ ở khoảng 200 chỗ (137 chỗ chỉ riêng `.name()`); đổi hết sang đọc
manifest ngay bây giờ chạm cùng file với hai phiên song song (lớp thư viện, event) mà không thêm gì cho người viết
module. Theo chiều này, kernel không phải sửa chỗ đọc nào, không có vòng gọi vô hạn giữa hai mặc định (manifest mặc
định không đọc hàm cũ), và khi baseline rỗng thì gỡ hàm cũ rồi đổi chỗ đọc sang manifest trong một PR như kế hoạch.

Đã cân nhắc: đúng §5.2 (manifest mặc định dựng từ hàm cũ, kernel đọc manifest): phải đổi mọi chỗ đọc và lưu manifest
theo từng module ngay; vỏ bọc mọi module ngay từ lúc đăng ký: vỏ phải chuyển tiếp đủ ~50 hàm, quên một hàm thì lặng
lẽ trả rỗng — vỏ hiện chỉ áp cho module kiểu mới và chỉ cài các hàm "sau start".

Thay: phần "`manifest()` có cài đặt mặc định dựng từ 45 hàm cũ" của D-20261126-module-manifest
