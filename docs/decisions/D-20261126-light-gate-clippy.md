## D-20261126-light-gate-clippy · Gate nhẹ (`GATE_LOCAL=off`) vẫn chạy clippy; việc agent tự chạy thêm trước khi push — 2026-11-26 — chủ dự án quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. `GATE_LOCAL=off ./gate.sh --quick` chạy cả `cargo clippy --workspace --all-targets -- -D warnings` (bỏ khi thay đổi
   không chạm Rust, như gate nhanh). Vẫn không chạy test và không kiểm file sinh.
2. Khi công tắc `off`, trước khi push agent tự chạy thêm: `cargo xtask regen --check` khi đổi API hay cấu hình; các bài
   test vừa viết hay sửa trên cả hai database; `cargo test -p autoxide` khi thêm bảng, index, setting, event hay
   endpoint (crate `autoxide` phụ thuộc mọi module và giữ các bài "mọi bảng của mọi module phải khai…").

Lý do: chủ dự án yêu cầu không để lỗi vặt lọt lên CI. Trong một ngày tắt gate nặng, hai pull request đỏ CI vì lỗi rẻ:
một cái thiếu khai báo cho bảng mới (bài test tổng của `autoxide` bắt), một cái còn hàm không ai gọi (clippy
`dead_code`). Mỗi lần đỏ mất một vòng CI khoảng hai mươi phút và một lượt kiểm lại. Clippy mất vài phút trên máy bốn
nhân khi đã có cache — rẻ hơn nhiều so với một vòng CI, và vẫn nhanh hơn hẳn gate nhanh đầy đủ (phần tốn là biên dịch
và chạy test của mọi crate phụ thuộc).

Đã cân nhắc: bật lại gate nhanh đầy đủ (chủ dự án đã chọn tắt vì quá chậm); chỉ dặn agent chạy clippy bằng lời (dễ
quên — đưa vào lệnh thì không quên được); chạy luôn `cargo test -p autoxide` trong gate nhẹ (khoảng hai mươi lăm phút
trên cả hai database, chỉ cần khi thêm bảng hay endpoint nên để agent quyết theo luật ở mục 2).

Thay: phần "không clippy" ở mục 2 của D-20261125-local-gate-switch
