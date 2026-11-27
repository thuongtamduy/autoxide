## D-20261126-purpose-cap-strict · Trần mục đích xử lý của `compliance` là trần chính xác, đếm và chèn trong một transaction ghi — 2026-11-26 — agent quyết

Chủ đề: Database, dữ liệu, job, event

Quyết định: thay câu "hai request tạo cùng lúc có thể vượt một dòng" ở mục 4 của D-20261126-list-pages-done. Trần
`AUTOXIDE_COMPLIANCE_PURPOSES_TENANT_MAX` (409 `compliance.purpose_limit_reached`) nay chính xác: `create_purpose` đếm
mục đích riêng của khách và chèn dòng mới trong **cùng một** transaction ghi (`Database::begin_write`), sau khi khoá
theo tên `compliance.purposes:<tenant_id>` bằng `autoxide_kernel::rows::lock_key`. Postgres: `pg_advisory_xact_lock`
giữ tới hết transaction, request thứ hai chờ rồi đếm lại thấy dòng vừa thêm. SQLite: `BEGIN IMMEDIATE` của
`begin_write` đã giữ quyền ghi cả database nên `lock_key` không làm gì mà vẫn đúng. Sửa và ghi đè mục đích mặc định
không đi qua bước đếm (như trước). Test `tran_muc_dich_dung_khi_tao_dong_thoi`: 8 request đồng thời với trần 3 thì đúng
3 thành công, 5 nhận 409, chạy trên cả hai database.

Lý do: `lock_key` là công cụ có sẵn của kernel (D-181) cho đúng trường hợp "không có dòng ổn định để khoá" — khách
chưa có mục đích nào thì `SELECT … FOR UPDATE` không có dòng để khoá. Chi phí là một câu lệnh mỗi lần tạo, ở chỗ ghi
hiếm. Trần chính xác dễ giải thích cho người dùng hơn trần mềm.

Đã cân nhắc: giữ trần mềm và chỉ ghi tài liệu — loại vì làm đúng rẻ; `SELECT … FOR UPDATE` trên một dòng của khách
(vd dòng `kernel_tenants`) — loại: đọc bảng của kernel từ module trái luật sở hữu bảng, và khoá dòng chung của khách
chặn cả việc khác. Trần lý do của kernel (`AUTOXIDE_REASONS_TENANT_MAX`, `kernel.reason_limit_reached`) cũng đếm rồi ghi
không trong transaction — cùng loại trần mềm, chưa sửa trong đợt này (ghi lại để đợt sau làm theo cùng cách).
