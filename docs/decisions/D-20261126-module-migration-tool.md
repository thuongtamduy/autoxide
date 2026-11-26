## D-20261126-module-migration-tool · Công cụ `migrate-module` sửa code module 0.x sang hợp đồng 1.0 bằng sửa văn bản theo vị trí cú pháp — 2026-11-26 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định: Đường chuyển 0.x → 1.0 cho người viết module (71-G14 mục 5, D-20261126-operations-baseline) gồm hướng dẫn
`docs/guides/migrating-to-1.0.md` + `.vi.md` và công cụ `autoxide migrate-module <đường dẫn>` (trong công cụ dòng lệnh độc lập
của 71-G6; trong repo là `cargo xtask migrate-module`). Công cụ phân tích cú pháp bằng `syn` (bật `span-locations` của
`proc-macro2`) rồi sửa **văn bản gốc theo vị trí**, giữ comment và định dạng, chạy `rustfmt` sau cùng. Luật máy móc áp thẳng
(`Cargo.toml`, `use`, derive, `#[utoipa::path]` → `#[endpoint]`, router → `routes!`, `Module` → `manifest()` + `start`, entity →
`Record`, mẫu truy vấn SeaORM thường gặp); chỗ không chắc chèn `// TODO(autoxide-migrate): …` và in danh sách. Bảng "cũ → mới"
của hướng dẫn sinh từ chính bảng luật. Test là cặp file trước/sau cho từng luật và chạy lại không đổi gì. Mọi PR chuyển cụm
module trong repo chạy công cụ trước; CI chạy nó trên bản chụp 0.x của `examples/external-module` và đòi biên dịch + test xanh.
Đích: ≥ 80% chỗ gọi của một module điển hình được sửa tự động.

Lý do: ~6.000 chỗ gọi trong repo và mọi module ngoài phải chuyển; công cụ dùng cho cả hai thì được thử trên mã thật trước khi
người ngoài dùng. Sửa theo vị trí giữ nguyên comment (người viết module mất comment là mất tài liệu của họ).

Đã cân nhắc: rust-analyzer SSR (cần rust-analyzer chạy được trên dự án người dùng, khó tự động hoá trong CI, khó cho người
mới); in lại cả file bằng `prettyplease` (mất comment); chỉ viết hướng dẫn (chủ dự án yêu cầu có công cụ). Thiết kế: §17 của
`docs/research/kernel-contract-1-0.md`.
