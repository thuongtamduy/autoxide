## D-20261126-no-local-full-tests · Gate nhẹ: không chạy bộ test tổng trên máy, CI chạy — 2026-11-26 — chủ dự án quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: khi `GATE_LOCAL=off`, trước khi push agent chỉ chạy `./gate.sh --quick` (bản nhẹ), `cargo xtask regen
--check` khi đổi API hay cấu hình, và các bài test chính nó vừa viết hay sửa (bài chạm database thêm trên Postgres nếu
nhanh), rồi gộp `origin/main` và push. Agent **không** chạy bộ test tổng trên máy: `cargo test -p autoxide`, test toàn
crate hay toàn workspace. CI trên pull request chạy bộ đó; CI đỏ thì agent sửa và push lại. Không merge khi CI chưa xanh
(không đổi).

Lý do: chủ dự án thấy mỗi pull request chờ quá lâu. Lượt test tổng trên máy (cả SQLite và Postgres) mất ba mươi đến bốn
mươi phút cho mỗi lần push, rồi CI chạy lại gần như đúng bộ đó thêm hai mươi đến hai mươi lăm phút. Luật gộp `main`
ngay trước lượt test cuối còn bắt chạy lại từ đầu mỗi khi một pull request khác vừa merge — trong một buổi, một pull
request phải chạy lại ba lần. Máy cloud còn hay hết kết nối Postgres khi hai tiến trình test chạy cùng lúc, làm đỏ giả.
Chốt chặn thật vẫn là CI xanh trước khi merge, nên bỏ lượt trên máy không làm `main` kém an toàn; cái giá là thỉnh thoảng
một vòng CI đỏ phải sửa thêm.

Đã cân nhắc: giữ `cargo test -p autoxide` chỉ khi thêm bảng, index, setting, event hay endpoint (luật trước — vẫn tốn
ba mươi phút cho phần lớn pull request); chạy bộ tổng chỉ trên SQLite (vẫn hai mươi phút, và lỗi riêng của Postgres vẫn
chỉ CI bắt).

Thay: phần "`cargo test -p autoxide` khi thêm bảng, index, setting, event hay endpoint" ở mục 2 của
D-20261126-light-gate-clippy
