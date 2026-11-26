## D-20261126-migration-rewrite-proof · Viết lại migration đã commit sang `Plan` chỉ khi SQL sinh ra giống từng byte trên cả hai database — 2026-11-26 — agent quyết

Chủ đề: Database, dữ liệu, job, event

Quyết định: Ngoại lệ có điều kiện cho luật "migration đã commit thì không sửa": được viết lại **mã Rust** của migration đã
commit sang API `Plan` của kernel (D-20261126-kernel-data-layer) khi và chỉ khi SQL nó sinh ra trên SQLite và Postgres
giống từng byte với trước. Cách giữ: trước khi chuyển, `cargo xtask regen` chụp SQL của mọi migration của mọi module ra
`xtask/migration-sql/<module>.<database>.sql`; test so file chụp; PR chuyển không được đổi file chụp. Tên migration (khoá
trong bảng lịch sử và trong manifest migration theo bản phát hành mà lệnh rollback của G3 dùng) không bao giờ
đổi; PR viết lại đi sau các PR của G3 chạm bộ chạy migration. Đổi nội dung schema vẫn phải là migration mới.

Lý do: không viết lại thì module giữ `sea-query` mãi vì các migration cũ, và lời hứa "thư viện không níu module" không đạt.
Luật cũ bảo vệ database đã chạy migration khỏi lệch schema; câu SQL giống từng byte cho cùng schema nên mục đích của luật
vẫn giữ, và có máy kiểm thay vì lời hứa.

Đã cân nhắc: giữ migration cũ trên DSL sea-query (module không bỏ được thư viện); gộp migration cũ thành một migration
"nền" mới (phá database đã cài, phải có đường nâng riêng). Thiết kế đầy đủ, API mẫu và thứ tự PR: `docs/research/kernel-contract-1-0.md`.
