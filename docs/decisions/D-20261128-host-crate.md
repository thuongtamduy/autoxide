## D-20261128-host-crate · Crate `autoxide-host`: log, bộ thu số đo và xuất trace ra khỏi kernel; phần còn lại ở lại có lý do — 2026-11-28 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định: Tạo crate `crates/host` (`autoxide-host`), chỉ `autoxide` được phụ thuộc (`cargo xtask check`, luật
`dependency`: vai `Host`; kernel, testkit, module không được khai, kể cả ở `[dev-dependencies]`). Chuyển sang đó đúng phần
mà đo bằng chỗ gọi (28/11/2026, `main` `42c69f254`) thấy **không module, testkit hay ví dụ nào dùng và không phần nào khác
của kernel gọi tới**:

- `logging` (bộ ghi log của tiến trình) → `autoxide_host::logging`;
- `metrics::Metrics` (bộ thu Prometheus) và route `/metrics` (`http::metrics_routes`) → `autoxide_host::metrics::{Metrics, routes}`;
- phần gửi đi của `telemetry` (`TraceExport`, `TelemetryError`) → `autoxide_host::telemetry`.

Ở lại kernel: `telemetry::remote_context` (lớp HTTP của kernel đọc `traceparent` ở mỗi request) và `http::metrics::record_request`
(macro `metrics`, không cần bộ thu). Kernel bỏ bốn thư viện thường (`opentelemetry-otlp`, `opentelemetry_sdk`,
`tracing-subscriber`, `metrics-exporter-prometheus`; hai cái sau chỉ còn ở `[dev-dependencies]` cho test). Đây là đổi phá
vỡ Rust API trước 1.0, ghi ở changelog mục Removed (D-20261125-api-break-before-1-0, D-20261126-contract-tiers: "đường dẫn đổi
thẳng trước 1.0"); 0 module trong repo dùng các đường đó.

Chưa chuyển (danh sách của D-20261126-contract-tiers), vì chính kernel còn gọi — chuyển bây giờ phải đảo chiều phụ thuộc hay
viết lại phần đăng ký module:

| Phần | Ai trong kernel gọi |
| --- | --- |
| `backup` | job sao lưu định kỳ của module nội bộ `kernel` (`ops/builtin.rs`, `backup/scheduled.rs`), `job/backlog.rs`, rollback migration (`private_dir`) |
| `transfer` | chuyển khách giữa placement (`tenancy/relocate`, `moves`, `exports`, `erasure`), `module/selection.rs` |
| `builtin`, `module_catalog`, `user_flags` (route của kernel) | `module/registry` dựng `KernelModule` và gắn `/me`; `http`, `tenancy`, `entitlement` dùng `builtin::NAME` / permission của nó |
| `Migrator`, `Database::connect` | tenancy (migration theo placement), backup, testkit |
| `tls` | module `ai`, `auth`, `files`, `webhooks` gọi `install_crypto_provider` |
| `setup` | module `content` khai bước cài đặt |

Muốn chuyển nhóm `builtin`/`backup` thì việc đầu tiên là cho `Modules` nhận module nội bộ từ bên ngoài (host đăng ký
`KernelModule` như module thường) — ghi thành mục lộ trình riêng khi làm, không làm lén trong một PR tách file.

Lý do: tách theo chỗ gọi thật, không theo danh sách dự kiến: thứ chuyển được thì chuyển hết và gate giữ không cho quay lại;
thứ còn bị kernel gọi mà cố chuyển thì phải dựng lại phần đăng ký module, trùng việc phiên song song đang làm ở
`module`/`StartContext`, và lợi ích (bớt hợp đồng) chưa bằng rủi ro. Bỏ bộ xuất OTLP và bộ thu Prometheus khỏi kernel làm
~40 module không còn biên dịch chúng.

Đã cân nhắc: để `autoxide_kernel::logging`… làm `pub use` trỏ sang host — không được, kernel không phụ thuộc host (vòng
phụ thuộc); để kernel tự dựng lại bộ ghi log cho test — chỉ hai bài test cần, mỗi bài dựng bộ ghi JSON năm dòng bằng
`tracing-subscriber` ở `[dev-dependencies]`; đưa `remote_context` sang host — lớp HTTP của kernel cần nó ở mỗi request.
