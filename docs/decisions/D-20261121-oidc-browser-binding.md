## D-20261121-oidc-browser-binding · Lượt đăng nhập OIDC gắn trình duyệt bằng `browser_secret` (tuỳ chọn) — 2026-11-21 — agent quyết

Chủ đề: Xác thực, phân quyền, bảo mật

Quyết định: `POST /api/v1/auth/oidc/{slug}/start` và `POST /api/v1/auth/me/identities/{slug}/start` nhận
`bind_browser: true`; khi đó trả thêm `browser_secret` (ngẫu nhiên 256 bit, server chỉ giữ bản băm trong dữ liệu
mã hoá của `state`). Callback nhận `browser_secret`; lượt đã gắn mà thiếu hay sai thì 400 `auth.oidc_state_invalid`
(`reason = browser`), `state` đã tiêu. Không gửi `bind_browser` thì như trước. Giao diện nên luôn bật.

Lý do: PKCE `code_verifier` nằm ở server, nên `code` + `state` lộ ra (log, lịch sử, referrer) đủ để kết thúc lượt
từ nơi khác. Gắn trình duyệt như `device_secret` của login link (D-311) đóng lỗ đó. Tuỳ chọn để không phá client
cũ (D-059): đòi bắt buộc là đổi hành vi của endpoint đang có.

Đã cân nhắc: cookie do server đặt — core chỉ có API, giao diện ở tên miền khác; trả `code_verifier` cho trình duyệt
— lộ verifier cùng chỗ với `code` thì như không có.
