## D-20261120-release-profile · Bản release: `strip = "symbols"`, `lto = "thin"`, `codegen-units = 1` — 2026-11-20 — agent quyết

Chủ đề: Vận hành và hiệu năng

Quyết định: `[profile.release]` thêm `strip = "symbols"`, `lto = "thin"`, `codegen-units = 1`; giữ `overflow-checks = true`
(D-179). Không đổi `opt-level`, `panic`.

Lý do — số đo (20/11/2026, cùng mã nguồn `main`, build sạch `cargo build --release -p autoxide` với 4 job trên máy 4 nhân dùng
chung; độ trễ đo như D-20261120-step39-hot-paths: oha 32 kết nối 8 giây, 2 lượt mỗi bản xen kẽ, cửa hàng 20 người bán × 10
sản phẩm, Postgres 16 sau `ANALYZE`; chi tiết ở `docs/BENCHMARKS.md` "Cấu hình bản release"):

| Cấu hình | Binary | Build sạch |
| --- | ---: | ---: |
| gốc (chỉ `overflow-checks`) | 377 MB | 25.9 phút |
| + `strip = "symbols"` | 290 MB (−23%) | 25.6 phút |
| + `lto = "thin"`, `codegen-units = 1` (chọn) | 198 MB (−47%) | 21.2 phút (−18%) |
| + `lto = "fat"`, `codegen-units = 1` | 181 MB (−52%) | 40.3 phút (+56%) |

Tốc độ (req/s so với gốc): thin nhanh hơn ở mọi đường đo, +1% (danh sách sản phẩm SQLite) đến +16% (`GET /auth/me`
Postgres); fat ngang thin trong dao động (−2% đến +21%), không đáng gấp đôi thời gian build. Phần lớn binary là mã máy
(`.text` 230 MB của 377 MB) nên strip chỉ bớt một phần; LTO bỏ mã trùng giữa các crate. Image Docker giảm đúng phần binary
(máy đo không có Docker): image 0.15.0 là 627 MB với binary 377 MB → khoảng 450 MB.

Đánh đổi: bỏ bảng symbol thì backtrace (khi đặt `RUST_BACKTRACE`) mất tên hàm; thông báo panic (lớp HTTP đổi panic thành
lỗi 500 có log) vẫn có file và dòng vì nằm trong mã. App không đọc backtrace ở đâu. `cargo bench` (profile `bench` kế thừa
`release`) cũng dùng cấu hình này.

Đã cân nhắc: `strip = "debuginfo"` (giữ tên hàm cho backtrace) — bản release vốn gần như không có debug info, chỉ bớt 19 MB;
LTO fat — xem bảng; `opt-level = "s"` / `"z"` — đổi tốc độ lấy cỡ, ngoài phạm vi (cỡ đã giảm gần nửa mà vẫn nhanh hơn);
`panic = "abort"` — lớp HTTP cần bắt panic để trả 500 thay vì sập cả máy chủ.
