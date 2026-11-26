## D-20261126-permission-audit · Công tắc dò permission chỉ có trong test; bài dò PT-13 không được qua mà không thử — 2026-11-26 — agent quyết

Chủ đề: HTTP, REST, OpenAPI

Quyết định: bổ sung D-20261126-permission-gate.

- Hàm dựng router KHÔNG có lớp `x-permissions` (`Extras::without_permission_gate`) chỉ có khi bật feature
  `permission-audit` của `autoxide-kernel`; chỉ `autoxide-testkit` bật (`TestAppBuilder::without_permission_gate`).
  Testkit chỉ được khai ở `[dev-dependencies]`, nên bản build chạy thật không có hàm này; trường cờ là riêng tư.
- Bài dò `x_permissions_khop_cho_service_kiem` đếm lượt có kết luận theo từng operation (service trả 403
  `permission_denied`, hay nhận): 422 `validation_failed` và 404 của operation khai `x-hides-existence` (ID ngẫu nhiên
  không có, kiểm quyền hay không cũng 404) không tính. Operation có permission để thử mà không có lượt nào kết luận
  được thì test đỏ. Bài dò bỏ `limit=0` mà mẫu request thêm cho bài thứ tự PT-14, để danh sách cũng được dò.
- Lớp permission tra operation bằng map theo đường dẫn (`Arc` dùng chung, không chép danh sách permission mỗi
  request). Operation khai `head` riêng theo `x-permissions` của chính nó (không nêu permission thì không bị chặn,
  không mượn quy tắc `GET`); route không khai `head` thì `HEAD` dùng quy tắc của `GET` (axum chạy handler `get`).
- `Principal::require_any(&[&Permission])` công khai cho service nhận một trong nhiều permission: như `require`, đòi
  xác nhận lại (step-up) theo permission đầu tiên người gọi có khi permission đó rủi ro cao; lỗi `permission_denied`
  nêu đủ các permission và nói một trong số đó là đủ. `require_any_named` của lớp HTTP không đòi step-up — service đòi
  khi gọi `require` của permission nó dùng.
- Bài dò đỏ khi tổng số lượt có kết luận bằng 0 (tài liệu mất `x-permissions`).

Lý do: rà soát độc lập #533 — cờ `doc(hidden)` công khai không phải ranh giới; bài dò có thể xanh mà không thử
operation nào (mọi lượt 422, hay 404 của operation ẩn tồn tại).

Đã cân nhắc: danh sách cho phép đóng băng các operation không dò được — không cần, đo lúc dựng không còn operation nào.
