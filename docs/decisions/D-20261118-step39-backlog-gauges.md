## D-20261118-step39-backlog-gauges · Gauge tồn đọng của job, event và pool kết nối — 2026-11-18 — agent quyết

Chủ đề: Vận hành và hiệu năng

Quyết định: Bước 39 mục A3 (số đo tồn đọng). Kernel thêm `autoxide_kernel::job::backlog`:

- `backlog(database, name, job_modules, event_modules, now)` chỉ ĐỌC (kết nối đọc, SQLite chỉ có một kết nối ghi) và
  trả `BacklogSnapshot`: mỗi module `jobs_pending` (chưa xong, chưa dead-letter, kể cả hẹn giờ),
  `jobs_oldest_pending_seconds` (job TỚI HẠN chờ lâu nhất; job hẹn giờ tương lai không tính), `jobs_dead`,
  `events_pending`, `events_oldest_pending_seconds`, `events_dead`; mỗi pool (`read` / `write`) số kết nối đang dùng,
  rảnh, tối đa — đọc thẳng từ pool của sqlx, không truy vấn. Tuổi tính bằng cách đọc dòng đầu theo thứ tự (không
  `MIN(...)`): kiểu trả về của hàm gộp trên cột thời gian khác nhau giữa SQLite và Postgres.
- `publish(snapshots, max_database_labels)` đặt gauge `jobs_*`, `events_*`, `database_connections{pool,state}`,
  `database_connections_max{pool}` với nhãn `database` và `module`. Quá `AUTOXIDE_METRICS_MAX_DATABASE_LABELS` (mặc
  định 50) database thì phần còn lại gộp vào `database="other"` (cộng số đếm, lấy tuổi lớn nhất) — số chuỗi có trần
  dù mỗi khách có database riêng.
- `BacklogMonitor` (dựng bằng `Modules::backlog_monitor`) chạy mỗi `AUTOXIDE_METRICS_BACKLOG_SECONDS` (mặc định 30)
  ở tiến trình chạy việc nền (`all` / `worker`) và chỉ khi bật `/metrics`: tiến trình chỉ phục vụ API không đặt cùng
  gauge (Prometheus sẽ thấy chuỗi trùng). Nhiều tiến trình worker vẫn cho nhiều chuỗi khác `instance` — luật cảnh báo
  dùng `max without(instance)`.
- Đặt ở `job/` (không ở `ops/` như đặc tả phác) vì đọc bảng của job queue và outbox; tên công khai
  `autoxide_kernel::job::backlog`.
- `cargo xtask check` thêm luật `metric-labels`: macro số đo không được có nhãn `tenant`, `tenant_id`, `org_id`,
  `user_id`.

Còn lại của A3 (đặc tả bước 39, thứ tự làm mục 6): `GET /api/v1/kernel/ops/backlog`, tool MCP, permission
`kernel.ops.read`, `autoxide jobs stats` — làm sau, dùng chung `backlog`.

Lý do: bộ đếm cộng dồn (`jobs_failed_total`…) không cho thấy hàng đợi đang KẸT; người vận hành cần thấy trước khi khách
thấy (đặc tả bước 39).

Đã cân nhắc: nhãn theo khách (cấm: số chuỗi tăng theo số khách); chạy gauge ở mọi tiến trình (chuỗi trùng); `MIN(...)`
trên cột thời gian (kiểu trả khác nhau giữa hai database).
