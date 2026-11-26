## D-20261126-kernel-crates · Tách kernel theo lớp sau mặt tiền `autoxide-kernel`; phần chỉ binary dùng sang `autoxide-host` — 2026-11-26 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định: Trước mắt **chỉ tách `autoxide-host`** (server, bộ chạy migration, backup, transfer, logging/metrics/telemetry,
TLS, route của kernel — chỉ `autoxide` dùng) ra khỏi `autoxide-kernel`. Tách phần còn lại của kernel theo lớp (đích tham khảo:
`autoxide-values` → `autoxide-data` → `autoxide-web` → `autoxide-platform` → mặt tiền `autoxide-kernel`) **chờ chủ dự án quyết**
dựa trên số đo của spike (c) (thời gian build lại khi sửa một file kernel, trước và ước tính sau tách) và bản rà orphan rule /
vòng phụ thuộc — ở §15.3 của tài liệu thiết kế. Mục này nằm trong lộ trình 1.0 do chủ dự án duyệt nên agent không tự dời.
Nếu tách: module và testkit chỉ khai mặt tiền, đường `autoxide_kernel::x` không đổi.

Lý do: `autoxide-host` đưa phần không phải hợp đồng ra khỏi hợp đồng (D-20261126-contract-tiers) và không vướng gì. Tách theo
lớp vướng orphan rule (`impl IntoResponse for Error`, `impl From<DbErr> for Error`, `impl ToSchema for Page`), phụ thuộc ngược
(`http`/`mcp` dùng `access`/`tenancy`), và lợi ích nhỏ vì mọi module phụ thuộc mặt tiền nên sửa một file kernel vẫn biên dịch lại
mọi module (kiểm độc lập lần 1, 26/11/2026).

Đã cân nhắc: module khai từng crate con (lộ cấu trúc nội bộ thành hợp đồng); tách theo nhóm thư mục hiện có
(`contracts`/`data`/`ops`/`security`/`values` — có vòng phụ thuộc, không theo lớp). Thiết kế đầy đủ, API mẫu và thứ tự PR: `docs/research/kernel-contract-1-0.md`.
