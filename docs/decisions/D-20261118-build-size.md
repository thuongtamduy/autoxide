## D-20261118-build-size · Build gọn: test tích hợp mỗi crate một chương trình, bỏ debug info của thư viện ngoài, gate không incremental và tự dọn `target/` (D-299, D-20261118-ci-pr-affected-tests) — 2026-11-18 — chủ dự án quyết hướng, agent quyết cách làm

Chủ đề: Quy trình, gate, lộ trình

Bối cảnh: `target/` của máy phát triển lên 121 GB và làm đầy ổ đĩa giữa lúc nhiều phiên agent cùng build. Ba nguyên nhân:
mỗi file `tests/*.rs` là một chương trình test riêng link lại toàn bộ thư viện (63 chương trình: 58 của `kernel`, 2 của
`app`, 3 của `testkit`, chưa kể 25 chương trình test trong `src`); cargo không bao giờ xoá bản build cũ khi đổi nhánh hay
`Cargo.lock`; cache incremental phình theo mỗi lần sửa. Chủ dự án chọn làm cả bốn cách dưới đây.

Quyết định:

1. **Test tích hợp mỗi crate một chương trình:** `tests/*.rs` của `kernel`, `app`, `testkit` chuyển vào
   `tests/integration/`, khai `mod` ở `tests/integration/main.rs`. `cargo xtask check` thêm luật `test-binary`: chặn file
   rời trong `tests/` và file trong `tests/integration/` chưa khai `mod` (file đó không được biên dịch, test âm thầm không
   chạy). Một test so chuỗi tên kiểu đổi theo: tên kiểu đầy đủ giờ có tiền tố `integration::`.
2. **Chia suất Postgres theo số test** (`xtask`, thay cách chia đều của D-299): trước khi chạy, đếm test của mỗi chương
   trình (`--list`), chia 16 suất chạy cùng lúc theo tỉ lệ số test (ít nhất 1); chương trình lớn chạy trước; chỉ chạy
   cùng lúc các chương trình có tổng suất không vượt 16 (`Condvar`). Chia đều thì chương trình `kernel` hàng trăm test chỉ
   được một hai suất. SQLite giữ cách cũ.
3. **Thư viện ngoài không có debug info** (`[profile.dev.package."*"] debug = false`). Code của workspace vẫn giữ
   `line-tables-only`: stack trace trong code của mình vẫn có số dòng.
4. **Gate:** `CARGO_INCREMENTAL=0` (CI đã đặt sẵn); đầu gate đo `target/`, vượt `GATE_TARGET_MAX_GB` (mặc định 40) thì xoá
   cache incremental, vẫn vượt thì `cargo clean`. `0` tắt việc dọn.

Đã cân nhắc: `cargo sweep` — thêm một công cụ phải cài; xoá file theo tuổi trong `target/debug/deps` — cargo vẫn giữ
fingerprint của file đã xoá, khó lường hơn xoá cả cache; tắt incremental trong `Cargo.toml` cho mọi lệnh — người sửa code
rồi build lại một crate mất lợi thế của incremental, trong khi phần phình chủ yếu đến từ gate.

Hệ quả: chuyển giữa `cargo test` trên máy (incremental) và `./gate.sh` (không incremental) thì crate của workspace được
build lại. Một gate đang chạy có thể dọn cache của một build khác đang chạy cùng lúc trên cùng `target/`; build đó lỗi thì
chạy lại.

Số đo (18/11/2026, cùng một máy, `target/` sạch, `CARGO_INCREMENTAL=0`, `cargo test --workspace --no-run` rồi
`cargo xtask test-all`):

| | Trước | Sau |
| --- | --- | --- |
| Dung lượng `target/` | 19 GB | 6,1 GB (−68%) |
| Build toàn bộ test | 716 giây | 509 giây (−29%) |
| Số chương trình test | 89 | 29 |
| Chạy mọi test, SQLite | 299 giây | 280 giây |
| Chạy mọi test, PostgreSQL 16 | 381 giây | 355 giây |
