## D-20261126-idle-polling · Lượt rảnh của bộ phát event và worker: một câu `EXISTS` mỗi database, không giãn nhịp — 2026-11-26 — agent quyết

Chủ đề: Vận hành và hiệu năng

Quyết định: mỗi lượt (mỗi giây khi rảnh), bộ phát event và worker job hỏi trước bằng MỘT câu
`SELECT EXISTS(SELECT 1 FROM <module>_events WHERE <chờ giao>) AS t0, EXISTS(…) AS t1, …` (sea-query `Expr::exists`,
chạy như nhau trên SQLite và Postgres) bảng nào có việc, rồi chỉ đọc / nhận ở các bảng đó trong cùng lượt
(`crates/kernel/src/data/busy.rs`). Lượt rảnh: một câu mỗi database cho mỗi bên, thay vì một câu mỗi module có bảng
(đo với mọi module của app: bộ phát event 30 câu → 1). Nhịp poll giữ 1 giây.

Lý do: rà soát hiệu năng 2026-11 — máy rảnh vẫn chạy khoảng 80 câu SQL mỗi giây trên Postgres. Cách này không làm
chậm việc giao event hay chạy job: bảng có việc được đọc ngay trong lượt đó, như trước.

Đã cân nhắc: giãn nhịp poll khi rảnh (1 → 8 giây) — event ghi ở tiến trình khác (tách tiến trình API và tiến trình
việc nền) và job (worker không có tín hiệu đánh thức) bị chậm tới vài giây; LISTEN/NOTIFY của Postgres sau một trait —
chỉ một bên database có, SQLite vẫn phải poll, thêm một đường code cho cùng việc. Chưa cần khi lượt rảnh đã còn một câu.
