## D-20261122-esms-environment · Bộ nối eSMS: môi trường thử theo quy ước chung `environment`, vẫn là cờ `Sandbox` trong request — 2026-11-22 — agent quyết

Chủ đề: Thương mại

Quyết định: cấu hình khách `esms.connection.sms.sandbox` (`true` / `false`) đổi thành `esms.connection.sms.environment`
(`production` / `sandbox`, kiểm bằng `provider::Environment::check`) như mọi bộ nối (D-20261122-provider-environments).
eSMS không có host sandbox riêng: môi trường `sandbox` chỉ thêm `"Sandbox": "1"` vào thân request tới cùng địa chỉ, nên
không có biến `AUTOXIDE_ESMS_SANDBOX_API_URL`. Không khai tên cũ bằng `formerly`: bộ nối chưa từng nằm trong bản phát
hành nào, và giá trị cũ (`true`) không hợp lệ dưới tên mới.

Lý do: một cách cấu hình môi trường thử cho mọi bộ nối; khách và agent học một lần.

Đã cân nhắc: giữ `sandbox` riêng cho eSMS (lệch quy ước); thêm biến địa chỉ sandbox cho đủ bộ (tài liệu eSMS không có
địa chỉ đó).

Thay: phần tên trường `sandbox` ở mục 2 của D-20261121-esms-adapter
