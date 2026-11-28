## D-20261128-event-per-subscriber · Event: giao và thử lại theo từng bên nhận — 2026-11-28 — agent quyết

Chủ đề: Database, dữ liệu, job, event

Quyết định:

- **Một lượt giao (fan-out) không dừng ở bên lỗi.** `Dispatcher` giao event cho từng module nhận theo thứ tự đăng
  ký; bên nào lỗi (trả lỗi, panic, quá giờ) thì ghi lại và đi tiếp tới bên sau. Xong lượt, event ở `<module>_events`
  được đánh dấu `delivered_at` — nghĩa mới: "đã chia xong", mỗi bên hoặc đã nhận, hoặc có dòng thử lại riêng.
- **Trạng thái của bên lỗi nằm ở bảng `kernel_event_deliveries`** (kernel sở hữu, cấp khách hàng, có ở mọi
  database dữ liệu như `kernel_jobs`): một dòng cho (module phát, event, module nhận), cùng bộ cột hàng đợi như
  outbox (`attempts`, `next_attempt_at`, `claimed_until`, `delivered_at`, `failed_at`, `last_error`). Chỉ bên lỗi
  mới có dòng: đường bình thường không ghi thêm gì. Dòng thử lại và `delivered_at` của event ghi trong **một**
  transaction, nên không có lúc "event đã giao mà mất dòng thử lại".
- **Thử lại chỉ bên lỗi**, với backoff như cũ (2, 4, 8… giây, tối đa 1 giờ); sau `MAX_ATTEMPTS` lần thì dòng đó
  vào dead-letter (`failed_at`) — **riêng bên đó**; event xoá tài khoản (`privacy::USER_DELETED`) vẫn thử lại mỗi giờ
  mãi (giữ D-149), cũng riêng từng bên. Nhận việc thử lại bằng cùng câu `UPDATE` có điều kiện như event
  (`claimed_until`): nhiều máy chủ không giao trùng.
- **Thời hạn một lượt (`DELIVERY_TIMEOUT`, 50 giây) giữ nguyên** để không ai nhận lại event đang chia (D-158). Bên
  quá giờ tính một lần lỗi; các bên sau chưa kịp tới lượt được ghi dòng thử lại với `attempts = 0`, tới hạn ngay —
  lượt sau giao riêng từng bên, không bên nào phải chờ bên chậm.
- **Đơn vị là module nhận**, không phải từng `Subscription`: một module có hai đăng ký khớp cùng event mà cái thứ
  hai lỗi thì lần thử lại chạy cả hai (bên nhận chịu nhận lại — D-054).
- **Thử lại tới module không còn nhận** (đã gỡ, đang tắt, hay gói của khách không còn module đó): bỏ qua, đánh dấu
  xong — như module tắt không nhận bù event lúc tắt (bước 70 P9).
- **Dọn:** event đã chia xong quá 7 ngày bị xoá trừ khi còn dòng thử lại chưa xong (đang chờ hay dead-letter) — dòng
  thử lại cần đọc lại event. Dòng thử lại đã giao quá 7 ngày bị xoá.
- **Số đo:** `events_pending` / `events_dead` / `events_oldest_pending_seconds` của module phát cộng thêm dòng thử lại
  (chờ / dead-letter) của event của module đó; nhãn giữ nguyên (`database`, `module` — tập nhỏ cố định).
  `events_failed_total` đếm thêm mỗi dòng vào dead-letter; không thêm nhãn mới.
- **Nâng cấp:** không đổi bảng `<module>_events`, không chuyển dữ liệu. Event đang chờ lúc nâng cấp (kể cả đã thử
  vài lần theo cách cũ) được chia lại cho mọi bên: bên đã nhận ở lần cũ nhận lại — trong mức D-054 cho phép, và
  cách cũ cũng đã làm vậy mỗi lần thử. Event đã hỏng hẳn theo cách cũ (`failed_at`) giữ nguyên trong dead-letter;
  người vận hành muốn giao lại thì xoá `failed_at` của dòng đó, event được chia lại cho mọi bên. `failed_at` trên
  `<module>_events` từ nay chỉ còn cho dòng không đọc được (D-158) hay bản sao bỏ lại ở placement cũ.
- Migration mới của kernel `m0044_create_event_deliveries` (có `down`); bảng là hàng đợi nên dời khách mang theo,
  tách / gộp khách đòi trống (như `<module>_events`), xuất dữ liệu khách bỏ qua.

Lý do: cách cũ dừng ở bên đầu tiên lỗi rồi thử lại cả event từ đầu — một module hỏng (hay chậm) chặn mọi module đăng
ký sau nó; sau 10 lần bên sau không bao giờ nhận (mục G1 "Event: một module lỗi chặn các module nhận sau" của bước
71). Ghi trạng thái theo bên chỉ khi lỗi giữ đường nóng như cũ (một `UPDATE` mỗi event) và không đụng bảng của module.

Đã cân nhắc:

- Bảng con `<module>_event_deliveries` cho từng module phát: bảng gần event hơn, nhưng mọi module phát event (và
  module viết ngoài repo) phải thêm một migration, module nào quên thì bộ phát hỏng; kernel giữ bảng thì một
  migration là đủ và không đổi hợp đồng `Module`.
- Ghi một dòng cho mọi (event, bên nhận) ngay lúc ghi event: thêm N lần ghi trong transaction nghiệp vụ, và
  `Outbox::record` phải biết ai nhận — bộ ghi không biết (đăng ký là của bộ phát).
- Cột theo module trên `<module>_events`: số module nhận đổi theo bản cài, cột phải sinh động — không khả thi với
  migration đã commit.
- Giao song song cho các bên: không cần để sửa lỗi chặn, và đổi thứ tự quen thuộc (audit trước); để sau nếu đo thấy
  chậm.

Thay: phần "Lỗi thì thử lại với backoff … sau 10 lần đánh dấu `failed_at`" của D-054
