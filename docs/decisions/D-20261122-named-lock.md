## D-20261122-named-lock · `rows::lock_key`: khoá theo tên tới hết transaction ghi; `catalog` tính sản phẩm tương tự dưới một khoá — 2026-11-22 — agent quyết

Chủ đề: Database, dữ liệu, job, event

Quyết định: kernel thêm `autoxide_kernel::rows::lock_key(&transaction, key)` — công cụ thứ ba của D-181 bên cạnh khoá dòng
(`lock` / `lock_one`) và `adjust`. Postgres: `pg_advisory_xact_lock` với số 64 bit lấy từ 8 byte đầu SHA-256 của `key`
(trùng số chỉ làm chờ thừa); SQLite: không làm gì vì `BEGIN IMMEDIATE` của `Database::begin_write` đã giữ quyền ghi cả
database. Khoá nhả khi transaction kết thúc. Tên có tiền tố module. Nhiều khoá trong một transaction thì lấy theo cùng
thứ tự. Test đồng thời (`money::khoa_theo_ten_tuan_tu_hoa_doc_roi_ghi`) chạy trên cả hai database, đỏ trên Postgres khi bỏ
khoá.

Dùng khi việc cần tuần tự hoá không gắn với một dòng ổn định: chưa có dòng nào, hay dòng bị xoá + chèn lại (đổi ID) giữa
chừng.

Áp dụng đầu tiên — `catalog` (thay cách khoá mọi dòng embedding của PR #350): job `catalog.index_embeddings` gọi mô hình
ngoài transaction, rồi trong MỘT transaction ghi lấy `lock_key("catalog.similar:<tenant>:<model>")`, xoá vector của hàng
thôi bán, ghi lại vector, đọc lại toàn bộ vector và ghi danh sách tương tự. Khoá theo dòng embedding còn hở: hai job của
cùng một sản phẩm xoá + chèn lại dòng (ID mới) nên câu `FOR UPDATE` của job kia bỏ qua dòng đã xoá, và lúc bảng rỗng không
có dòng nào để khoá — CI Postgres đỏ ngẫu nhiên vì vậy. Khoá theo tên cũng tránh việc mỗi job khoá hàng chục nghìn dòng
với catalog lớn.

Đánh đổi: các job lập index của cùng một khách đi lần lượt; job đang chờ giữ một kết nối ghi. Chấp nhận được: worker giới
hạn số job mỗi module, phần trong khoá chỉ là ghi vector và tính điểm trên bộ nhớ.
