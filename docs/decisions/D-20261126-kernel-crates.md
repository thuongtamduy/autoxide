## D-20261126-kernel-crates · Tách kernel theo lớp sau mặt tiền `autoxide-kernel`; phần chỉ binary dùng sang `autoxide-host` — 2026-11-26 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định: Đích là đồ thị một chiều `autoxide-macros` → `autoxide-values` → `autoxide-data` → `autoxide-web` →
`autoxide-platform` → `autoxide-kernel` (mặt tiền: `pub use` mọi đường hiện có + `Module`, `Manifest`, `StartContext`),
và `autoxide-host` (server, bộ chạy migration, backup, transfer, logging/metrics/telemetry, TLS, route của kernel) chỉ
`autoxide` dùng. Module và testkit chỉ khai `autoxide-kernel`; đường `autoxide_kernel::x` không đổi. Làm: đo đồ thị
`use crate::…` giữa các nhóm (`cargo xtask kernel-deps`), cắt vòng, tách từ dưới lên mỗi crate một PR (chỉ di chuyển code),
đo `cargo build --timings` trước/sau. Ranh giới cuối (vd `tenancy` thành crate riêng) quyết bằng mục mới sau khi đo.

Lý do: kernel ~86.000 dòng là một đơn vị biên dịch; sửa một file đa khách biên dịch lại cả phần giá trị, database, HTTP. Mặt
tiền giữ hợp đồng cho người ngoài trong khi bên trong chia nhỏ; `autoxide-host` đưa phần không phải hợp đồng ra khỏi hợp đồng
(D-20261126-contract-tiers).

Đã cân nhắc: module khai từng crate con (lộ cấu trúc nội bộ thành hợp đồng); tách theo nhóm thư mục hiện có
(`contracts`/`data`/`ops`/`security`/`values` — có vòng phụ thuộc, không theo lớp). Thiết kế đầy đủ, API mẫu và thứ tự PR: `docs/research/kernel-contract-1-0.md`.
