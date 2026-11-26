## D-20261126-kernel-crates · Tách kernel theo lớp sau mặt tiền `autoxide-kernel`; phần chỉ binary dùng sang `autoxide-host` — 2026-11-26 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định (chủ dự án giao phiên điều phối quyết, 26/11/2026): Trước 1.0 **chỉ tách `autoxide-host`** (server, bộ chạy
migration, backup, transfer, logging/metrics/telemetry, TLS, route của kernel — chỉ `autoxide` dùng) ra khỏi `autoxide-kernel`.
Tách phần còn lại của kernel theo lớp (đích tham khảo: `autoxide-values` → `autoxide-data` → `autoxide-web` →
`autoxide-platform` → mặt tiền `autoxide-kernel`) **dời sau 1.0**: quy tắc chốt trước là "tiết kiệm < 20% hoặc vướng orphan rule
→ dời", và số đo của spike (c) là 6–9%. Mặt tiền giữ đường `autoxide_kernel::x`, nên tách sau 1.0 không phá vỡ hợp đồng. Muốn
làm lại thì đo lại theo cùng cách và ghi mục mới.

Số đo (spike (c), 26/11/2026, máy cloud 4 CPU / 15 GB, `main` `af74d2fb5`, profile dev, build đã ấm, thêm một dòng comment vào
file rồi chạy lại):

| Lệnh | Sửa file | Tổng | Phần `autoxide-kernel` | Phần ~40 module + binary |
| --- | --- | --- | --- | --- |
| `cargo build -p autoxide` | `tenancy/scope.rs` | 455 s | 145 s (phân tích 110 s, sinh mã 34 s) | ~345 s sau khi kernel phân tích xong |
| `cargo build -p autoxide` | `values/slug.rs` | 455 s | 143 s | như trên |
| `cargo check -p autoxide` | `tenancy/scope.rs` | 210 s | 56 s | ~154 s |
| `cargo check -p autoxide` | `access/mod.rs` | 212 s | 57 s | ~155 s |
| `cargo check -p autoxide` | `http/mod.rs` | 215 s | 58 s | ~157 s |

Ước tính sau tách: dòng kernel theo lớp đích là values 7.499, data 5.214, web 10.064, host 5.235, platform 57.962 (tổng 85.974).
Sửa một file của `platform` (tenancy, access) chỉ biên dịch lại phần ~67% đó, rút ~38 s của 455 s khi build (~8%) và ~19 s của
210 s khi check (~9%); sửa `http` (lớp web) biên dịch lại web + platform, rút ~6%; sửa `values` biên dịch lại cả chuỗi, không
rút gì. Module vẫn biên dịch lại hết vì mọi module phụ thuộc mặt tiền — phần này chiếm 70–75% thời gian.

Lý do: `autoxide-host` đưa phần không phải hợp đồng ra khỏi hợp đồng (D-20261126-contract-tiers) và không vướng gì. Tách theo
lớp vướng orphan rule (`impl IntoResponse for Error`, `impl From<DbErr> for Error`, `impl ToSchema for Page`), phụ thuộc ngược
(`http`/`mcp` dùng `access`/`tenancy`), và lợi ích nhỏ vì mọi module phụ thuộc mặt tiền nên sửa một file kernel vẫn biên dịch lại
mọi module (kiểm độc lập lần 1, 26/11/2026).

Đã cân nhắc: module khai từng crate con (lộ cấu trúc nội bộ thành hợp đồng); tách theo nhóm thư mục hiện có
(`contracts`/`data`/`ops`/`security`/`values` — có vòng phụ thuộc, không theo lớp). Thiết kế đầy đủ, API mẫu và thứ tự PR: `docs/research/kernel-contract-1-0.md`.
