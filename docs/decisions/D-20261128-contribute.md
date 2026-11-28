## D-20261128-contribute · `contribute::<dyn Trait>` là đường đăng ký điểm mở rộng của kernel; getter cũ đếm bằng baseline — 2026-11-28 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định: `StartContext::contribute::<dyn T>(…)` nhận cả đóng góp vào registry của kernel: kernel thử ép `Arc<T>` về
từng trait của nó (`PersonalData`, `PostingVerifier`, `WorkSource`, `FinderSource`, `SuggestSource`, `SummaryMetric`,
`ExportSource`, `BulkAction`, `CodeResolver`, `SetupStep`, `ConnectionProbe`, `ReferenceSource`) và gửi vào đúng registry như
hàm đăng ký cũ (`crates/kernel/src/module/contribute.rs`). Dữ liệu ban đầu, dữ liệu mẫu và bản sao sandbox đóng góp bằng
`dyn TenantSeed`, `dyn SampleData` (trait mới, tách khỏi `TenantSeed` vì `contribute` chọn chỗ nhận theo kiểu) và
`dyn SandboxCopy`, hook của bước chuyển khi tách / gộp khách bằng `dyn SwitchHook` (thay `Relocation::on_switch`, đường
cũ vẫn chạy); registry gom sau khi mọi module khởi động, cùng với hàm cũ `Module::seed_tenant`/`sample_data`/
`sandbox_copy` (module làm cả hai cách thì khởi động lỗi). Ba hàm chuyển tiếp `StartContext::seed_tenant`, `sample_data`,
`sandbox_copy` (thêm ở #616, chưa phát hành, không ai gọi) **gỡ luôn**: hết chữ ký hạng ổn định nhận kiểu thử nghiệm
`SandboxCopy` — `contribute` là hàm generic, kiểu thử nghiệm chỉ do module tự chọn khi gọi. Getter cũ (`outbox()`,
`sync_log()`, `entitlements()`… — mọi getter có handle trong `KernelHandle`) và hàm đăng ký cũ ở trên giữ chạy, không
`#[deprecated]` (D-20261127-transition-paths); `cargo xtask check` đếm chỗ gọi `context.<tên cũ>(` trong mã module (bỏ thân
`fn dormant`) theo luật `start-context-legacy`, baseline trong `xtask/module-contract-legacy.txt`, chỉ giảm. `notes` dùng
đường mới.

Lý do: thiết kế (§5) đặt `contribute` thay ~20 hàm đăng ký riêng; làm trước khi chuyển cụm module 1–2 để module không chuyển
hai lần. Định tuyến theo `TypeId` giữ nguyên hành vi (kiểm tên, đăng ký trùng) vì gọi đúng hàm `register` cũ.

Đã cân nhắc:
- Cất mọi đóng góp ở `Apis` rồi registry gom sau khi khởi động: registry như `PersonalDataExport`, `SuggestSources` được đọc
  (và kiểm lỗi) trong lúc khởi động — đổi thời điểm là đổi hành vi.
- Đưa các đăng ký có khoá (`resource_type(kind, …)`, `badge_source(kind, codes, …)`, `block_data(name, …)`,
  `taggable(kind, permission)`, `usage_counter(limit, …)`, `search_documents`, `search_backend`) vào `contribute`: chúng gắn
  một khoá với cài đặt, không phải "thêm một cài đặt của trait"; ép vào `contribute` phải thêm hàm `kind()` vào từng trait
  (phá vỡ). Chúng **giữ** là hàm đăng ký có tên (lệch bảng §5 ở `badge_source`).
- Đếm cả `database()`, `permissions()`, `translations()`, `events()`: trả tham chiếu, không phải handle; `database()` đã có
  luật `raw-database`.

Thay: phần "Chuyển tiếp, sẽ thay: `StartContext::seed_tenant`, `sample_data`, `sandbox_copy`" của D-20261128-manifest-defaults
