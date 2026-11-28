## D-20261128-job-all-tenants · `JobContext::all_tenants` là nguồn thứ hai của handle quét mọi khách — 2026-11-28 — agent quyết

Chủ đề: Kiến trúc module và kernel

Quyết định: ngoài `ControlPlane::all_tenants` (lấy từ `StartContext`), job của cả một database (`JobContext::tenant` là
`None`: lịch dọn dẹp, `JobQueue::enqueue_database`) lấy handle quét mọi khách của **database job đang chạy** bằng
`JobContext::all_tenants("lý do")`. Hai đường cùng một hàm dựng nội bộ (`AllTenants::counted`): cùng số đo
`kernel_all_tenant_queries{module}` (module = tiền tố của loại job, đã kiểm lúc đăng ký) và cùng log lý do. Job của một
khách (`tenant` là `Some`) gọi hàm này là dùng nhầm: bản debug dừng ngay (`debug_assert!`), bản release ghi cảnh báo
(`tracing::warn!`) rồi vẫn trả handle như job của cả database — không đổi chữ ký thành `Result` cho một lỗi lập trình.

Lý do: ở chế độ nhiều database (khách có database riêng, bước 33), lịch dọn dẹp chạy một lần trên **từng** database;
`ControlPlane` chỉ trỏ database điều khiển, nên không quét được bảng của khách nằm ở database riêng. Trước đây các job
này dùng `job.database` thô với SeaORM — chính là lối vượt bộ lọc khách mà lớp `db` muốn đóng. Handle lấy từ
`JobContext` chỉ chạm database của job, có số đo và lý do như đường `ControlPlane`.

Đã cân nhắc: `DatabaseRegistry` trả `AllTenants` cho từng database (module phải tự lặp database, trong khi worker đã
chạy job trên từng database); để job tự gọi `ControlPlane::all_tenants` (sai database ở khách có database riêng); trả
lỗi khi job của một khách gọi (phải đổi kiểu trả về thành `Result` cho mọi nơi gọi đúng).

Thay: phần "`Db`/`AllTenants` chỉ lấy từ `StartContext`" của D-20261126-kernel-data-layer
