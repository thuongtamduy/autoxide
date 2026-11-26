## D-20261126-placement-migration-exclusive · Một lượt migrate placement một lúc trên mọi đường, tự dọn lượt bỏ dở — 2026-11-26 — agent quyết

Chủ đề: Database, dữ liệu, job, event

Quyết định: mọi đường migrate placement — lệnh `migrate up --all` (và `Placements::up_all` / `up_placements`), job
`kernel.migrate_placements` của API, lượt `serve` xếp lúc khởi động — đều:

1. giữ **khoá placement** (`MigrationLock::acquire_placements`, qua `Placements::lock`) suốt lượt — khoá riêng, khác
   khoá chính của database điều khiển (`MigrationLock::acquire`): máy chủ khởi động chỉ cần khoá chính nên không bao
   giờ chờ một lượt dài qua các placement. Ai cần cả hai (`migrate up --all`, `up_all`) lấy khoá chính trước;
   `up_all_report` nhận khoá chính đang giữ làm tham số. Trên Postgres khoá là advisory lock **mức session** trên một
   kết nối lấy hẳn ra khỏi pool (không giữ transaction mở, nên `idle_in_transaction_session_timeout` không cắt);
   trước mỗi placement người giữ gọi `MigrationLock::verify` (đọc `pg_locks`) và dừng nếu kết nối đã đứt. Job chỉ
   chờ khoá 30 giây; chưa được thì job hỏng và thử lại sau (tối đa 10 lần — migration hỏng không làm job hỏng, lượt
   ghi `failed`);
2. giữ chỗ `active` duy nhất của `kernel_migration_runs` (`migration_runs::claim`), nên lượt chạy từ dòng lệnh cũng hiện
   trong danh sách lượt và API báo 409 thay vì xếp lượt thứ hai. Lượt dòng lệnh gặp một lượt API đang chờ cùng phạm vi
   (hay chính nó chạy mọi placement) thì nhận luôn lượt đó; job của lượt đó sau này thấy đã xong và thôi.

Placement không còn migration chờ thì không chuyển qua `migrating` (khách của nó không thấy 503 vô cớ); lúc khởi động
`serve` quyết định xếp lượt theo `schema_state` đã lưu, không mở kết nối tới từng placement. Lượt dòng lệnh va lượt API
vừa xếp (unique `active_key`) thì nhận lại lượt đó; job thấy lượt của chính nó đã xong thì thôi, không thử lại.

Người đang giữ khoá biết mọi lượt `running` khác là của tiến trình đã chết (chủ lượt luôn giữ khoá): đánh dấu `failed`
("interrupted"), và placement còn `migrating` trở về `behind` để lượt này nâng tiếp. Lượt `pending` quá một giờ (job đã
mất) cũng đóng lại. API `POST …/placements/migrations` lấy thử khoá không chờ để dọn như vậy trước khi xếp lượt mới,
nên một lượt bỏ dở không gây 409 mãi. `autoxide doctor` có bước `kernel.placements`: nêu placement `behind` và placement
kẹt `migrating` không ai đang chạy (lấy thử khoá không chờ), kèm cách sửa.

Lý do: kiểm độc lập thiết kế G3 (bước 71) thấy job của API migrate placement không giữ khoá nào, nên lệnh, job và
`serve` lúc khởi động có thể migrate cùng một placement một lúc (hai bên cùng tạo một bảng, một bên hỏng giữa chừng); và
một lượt mà tiến trình chết giữa chừng giữ chỗ `active` mãi. Khoá tiến trình là loại trừ thật (tự nhả khi chết); dòng
`active` là thứ người vận hành và API nhìn thấy — cần cả hai. Test đồng thời (ba lần chạy cùng lúc; job API và lệnh
cùng lúc) và test dọn lượt bỏ dở chạy trên cả SQLite và Postgres.

Giới hạn (ghi trong `docs/DEPLOY.md`): registry không có cấu hình database (chế độ `single`, SQLite trong bộ nhớ)
không có khoá thật — chỉ một tiến trình; trong lúc cập nhật cuốn chiếu từ bản cũ hơn, job của máy chủ bản cũ chạy không
giữ khoá; PgBouncer chế độ transaction không giữ được advisory lock theo session.

Đã cân nhắc: advisory lock theo transaction (bản đầu) — transaction mở hàng giờ bị timeout, proxy hay failover cắt mà
không ai biết; một khoá chung cho database chính và placement (bản đầu) — máy chủ khởi động chờ lượt nền tới 15 phút; khoá riêng từng placement (advisory lock theo id) — cho chạy song song nhiều lượt nhưng cần khoá file cho
từng placement SQLite và không giải quyết lượt bỏ dở; hạn thuê (lease) trên dòng `active` — cần nhịp gia hạn trong lúc
migration dài, phức tạp hơn khoá tiến trình đã có sẵn; chỉ dựa vào dòng `active` — dòng không tự nhả khi tiến trình chết.
