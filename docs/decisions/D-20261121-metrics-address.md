## D-20261121-metrics-address · `/metrics` trên port riêng — 2026-11-21 — agent quyết

Chủ đề: Vận hành và hiệu năng

Quyết định: thêm `AUTOXIDE_HTTP_METRICS_ADDRESS` (không mặc định). Đặt cùng `AUTOXIDE_HTTP_METRICS=true` thì `serve`
mở thêm một listener chỉ phục vụ `/metrics`, và `/metrics` rời port của API (không có trong router lẫn tài liệu OpenAPI
của API). Đặt mà không bật số đo, hay trùng port với `AUTOXIDE_HTTP_ADDRESS`, là cấu hình sai (báo khi khởi động).
Listener số đo dừng sau máy chủ API, cùng việc nền, để máy thu số đo vẫn đọc được lúc rút. Số đo vẫn tắt mặc định.

Lý do: `/metrics` không có xác thực (máy thu số đo gọi, D-245); trước đây chỉ có cách chặn nó ở reverse proxy. Port
riêng là cách chuẩn của Prometheus và Kubernetes: chỉ mở port đó cho máy thu, không cần sửa proxy.

Đã cân nhắc: token cho `/metrics` (Prometheus hỗ trợ `bearer_token`, nhưng thêm một secret phải quản lý, và endpoint
vẫn lộ ra internet); danh sách IP được phép (sai khi đứng sau proxy).
