## D-20261118-step39-ops-backlog-api · Tồn đọng qua REST và MCP, chỉ cho nhân sự nền tảng — 2026-11-18 — agent quyết

Chủ đề: Vận hành và hiệu năng

Quyết định: Phần còn lại của bước 39 mục A3.

- `GET /api/v1/kernel/ops/backlog` và tool MCP `kernel.ops_backlog` (read-only) gọi cùng một hàm
  (`job::backlog::status`, luật kiến trúc 8): với mỗi database, tồn đọng job / event theo module (cùng số liệu với gauge
  của D-20261118-step39-backlog-gauges), hai pool kết nối, và lúc xong của bản backup theo lịch tốt mới nhất.
- Permission mới `kernel.ops.read`, và chỉ nhân sự của khách nền tảng (`require_platform`, như `GET /kernel/ops/alerts`):
  số liệu là của cả database, gồm mọi khách trên đó. KHÔNG làm phần `top_tenants` và "quản trị khách thấy khách mình"
  của đặc tả: quản trị một khách không cần số của database chung, và tra theo khách đã có ở log / trace / số đo dùng.
- Danh sách module có bảng job / event do `Modules` điền vào một slot (`BacklogModules`) sau khi mọi module khởi động —
  cùng cách với `MoveSetup`; gauge (`BacklogMonitor`) dùng chung danh sách đó.
- Không làm `GET /kernel/ops/backups` (danh sách mọi lượt backup) và lệnh `autoxide jobs stats` của đặc tả: lúc backup
  tốt mới nhất đã có trong `ops/backlog`, lịch sử ở bảng `kernel_backups`; `autoxide jobs failed` đã có cho dead-letter.
  Thêm khi giao diện cần.

Lý do: người vận hành và agent không có Prometheus vẫn đọc được hàng đợi kẹt (đặc tả bước 39, A3).

Đã cân nhắc: cho quản trị một khách xem số của database chung (lộ quy mô của khách khác); trang danh sách backup (chưa
có giao diện nào cần).
