## D-20261126-migration-rewrite-proof · Viết lại migration đã commit sang `Plan` chỉ khi SQL sinh ra giống từng byte trên cả hai database — 2026-11-26 — agent quyết

Chủ đề: Database, dữ liệu, job, event

Quyết định: Ngoại lệ có điều kiện cho luật "migration đã commit thì không sửa": được viết lại **mã Rust** của migration đã
commit sang API `Plan` của kernel (D-20261126-kernel-data-layer) khi và chỉ khi **schema** nó tạo ra không đổi. Cách giữ lâu
dài: test `migration-schema` chạy, trên SQLite và Postgres, `up` của từng migration rồi `up → down → up`, chụp catalog schema đã
chuẩn hoá (bảng, cột, kiểu, NULL, mặc định, index, ràng buộc; sắp xếp, bỏ tên tự sinh) sau mỗi bước vào
`xtask/migration-schema/<module>.<database>.txt`; PR viết lại không được đổi file chụp. So từng câu SQL chỉ dùng lúc viết lại để
soát, không là test lâu dài (renderer của `Plan` không bị trói vào đầu ra của sea-query). Migration chỉ có DDL được đóng băng
bằng `plan.sql_for(backend, …)` sinh máy móc từ câu đã chụp. Bước dữ liệu giữ nguyên code Rust thành `DataStep`, có test dữ liệu
mẫu trước/sau trên cả hai database. Helper migration dùng chung của kernel chuyển một lần ở kernel; `down` gọi `up` của
migration cũ hơn thì chuyển cả cặp. Tên migration (khoá trong bảng lịch sử và trong manifest migration theo bản phát hành mà
lệnh rollback của G3 dùng) không bao giờ đổi; PR viết lại đi sau các PR của G3 chạm bộ chạy migration, và luật an toàn migration
của G3 chuyển sang đọc `Plan` khi `Plan` có. Đổi nội dung schema vẫn phải là migration mới.

Lý do: không viết lại thì module giữ `sea-query` mãi vì các migration cũ, và lời hứa "thư viện không níu module" không đạt.
Luật cũ bảo vệ database đã chạy migration khỏi lệch schema; so catalog schema sau `up` và sau `up → down → up` kiểm đúng điều
đó trên cả hai database. Kiểm lần 1 (26/11/2026) bác bản "giống từng byte": ~64/236 file có bước dữ liệu hay code riêng từng
database, chụp câu trên database trống không chứng minh gì cho bước dữ liệu, và trói renderer vào đầu ra của sea-query mãi.

Đã cân nhắc: so câu SQL từng byte (bản đầu của mục này — bị loại như trên); giữ migration cũ trên DSL sea-query (module không bỏ được thư viện); gộp migration cũ thành một migration
"nền" mới (phá database đã cài, phải có đường nâng riêng). Thiết kế đầy đủ, API mẫu và thứ tự PR: `docs/research/kernel-contract-1-0.md`.
