## D-20261118-step39-monitoring-files · Luật cảnh báo, dashboard và sổ tay sự cố nằm trong repo, `xtask` giữ khớp — 2026-11-18 — agent quyết

Chủ đề: Vận hành và hiệu năng

Quyết định: Bước 39 mục A4 và A6.

- `deploy/prometheus/autoxide-alerts.yml` và `deploy/grafana/autoxide-dashboard.json` là file thường trong repo (không
  sinh từ code). Mỗi luật cảnh báo có `summary` và `action` (câu "làm gì", trỏ mục của sổ tay). Gauge tồn đọng lấy
  `max without(instance)`, không `sum` (mỗi tiến trình worker đặt cùng gauge).
- Giữ khớp bằng luật `deploy-metrics` của `cargo xtask check` thay vì một test trong crate (đặc tả phác test
  `canh_bao_va_dashboard_chi_dung_so_do_co_that` đọc `KNOWN_METRICS`): xtask đọc mọi tên số đo mà code ghi bằng
  `metrics::counter!` / `gauge!` / `histogram!` (histogram nhận thêm đuôi `_bucket`, `_sum`, `_count`), rồi đòi mọi tên
  trong `expr` của hai file phải có trong đó; mọi `- alert:` phải có `action:`. Không cần danh sách tên viết tay — số đo
  mới của module tự được nhận. Test của luật nằm trong xtask (`canh_bao_va_dashboard_chi_dung_so_do_co_that`).
- Cảnh báo "backup quá cũ" thêm cùng backup theo lịch (mục A1), khi số đo `backup_last_success_timestamp_seconds` có.
- Sổ tay sự cố là mục "Runbook" / "Sổ tay xử lý sự cố" trong `docs/DEPLOY.md` / `.vi.md`. "Lộ khoá" dùng cách xoay
  đang có (`AUTOXIDE_SECRET_KEY_PREVIOUS` + `autoxide secrets reseal`; đổi `AUTOXIDE_AUTH_TOKEN_SECRET` thì access
  token cũ hết hiệu lực) tới khi mục A5 thêm khoá cũ cho access token.
- Không thêm `promtool check rules` vào gate: cần image Docker, trong khi gate nhanh không cần Docker; luật của xtask
  bắt lỗi hay gặp nhất (tên số đo sai).

Lý do: đặc tả bước 39, mục A4 và A6.

Đã cân nhắc: danh sách `KNOWN_METRICS` viết tay (phải sửa mỗi lần thêm số đo); `promtool` trong gate (cần Docker).
