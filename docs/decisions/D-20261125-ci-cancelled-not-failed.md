## D-20261125-ci-cancelled-not-failed · CI: lần chạy bị huỷ vì có commit mới không hiện đỏ; gate nhanh chạy test bằng nextest — 2026-11-25 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. **Check gộp không đỏ giả khi bị huỷ.** `gate (PostgreSQL …)` và `Rust API compatibility (cargo-semver-checks)` chạy với `always()` để
   phần nào đỏ thì báo đỏ; nhưng `always()` cũng chạy khi cả lần chạy bị huỷ (`concurrency.cancel-in-progress` của pull request khi
   có push mới), thấy `cancelled` rồi thoát 1 — ba lần trong ngày hiện thành "CI failed" trên commit cũ. Nay khi kết quả là
   `cancelled` thì job hỏi GitHub đầu PR hiện giờ: khác commit của lần chạy này (đã bị thay bằng commit mới) thì thoát 0 kèm
   `::notice`; vẫn là đầu PR (huỷ tay), không phải `pull_request`, hay hỏi lỗi thì vẫn thoát 1. Kết quả khác (`failure`, `skipped`…) vẫn đỏ.
   Không bao giờ tính bị huỷ là xanh cho commit hiện tại; commit cũ đã bị thay không còn là thứ ai merge. Job được thêm
   `pull-requests: read` (đọc đầu PR công khai). Job tải artifact của semver có `continue-on-error` vì shard bị huỷ không tải gì lên.
   Thử logic bằng `gh` giả (7 trường hợp); chưa thử trên GitHub thật — xem lần push chồng đầu tiên.
2. **Gate nhanh chạy test bằng nextest khi có.** `cargo xtask test-affected` (SQLite, cả bộ, không `--keep-env`/`--shard`) gọi
   `cargo nextest run --filterset <crate bị ảnh hưởng>`; không có nextest thì dùng bộ chạy cũ. Cùng bộ test, cùng chọn crate, cùng mốc
   "lần xanh gần nhất". Đo (máy 4 nhân, `CARGO_INCREMENTAL=0`): sửa kernel, gate nhanh 30 phút 23 giây = clippy 5 phút 32 giây
   + build test 11 phút 53 giây + chạy test 12 phút 12 giây; riêng chương trình test của `autoxide` (217 test) chiếm 709 giây của
   12 phút 12 giây và các chương trình khác dưới 50 giây — một chương trình đứng một mình kéo dài cả bước dù máy nhàn rỗi. Chạy
   chương trình đó một mình: libtest 674 giây, nextest 203 giây. Cả bước chạy test của 43 crate (2830 test, sửa kernel): bộ chạy
   cũ 732 giây, nextest 293 giây (không tính biên dịch, giống nhau ở hai đường). Phần còn lại của gate nhanh khi sửa kernel vẫn là
   biên dịch (~18 phút gồm clippy) và không đổi: nextest tiết kiệm ~7 phút trong ~30.

Lý do: người xem tưởng code hỏng và nhận thông báo CI failed giả; gate nhanh trên máy cloud 4 nhân quá lâu cho mỗi vòng sửa kernel.

Đã cân nhắc: `!cancelled()` thay `always()` (job bị bỏ qua, mà check bị bỏ qua được tính là đạt — huỷ tay lần chạy mới nhất sẽ qua check bắt buộc); bỏ clippy khỏi gate nhanh (clippy là một bước của gate và là lưới đầu tiên, không đổi); giảm thông tin debug (đã tối ưu ở D-20261118-build-size); chia nhỏ "kernel đổi phần nào" (mọi module phụ thuộc kernel thật, chọn tinh hơn có nguy cơ bỏ sót test).
