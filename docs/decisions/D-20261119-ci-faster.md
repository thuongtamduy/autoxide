## D-20261119-ci-faster · CI chia gate thành nhiều job song song (theo phần và shard), main lưu cache, Postgres chia suất có sàn — 2026-11-19 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. `./gate.sh --only lint|sqlite|postgres` (đi kèm gate đầy đủ hoặc `--pr`) chạy một phần của gate: `lint` là bước 1–6
   (fmt, clippy, xtask check, api-diff, sdk --check, deny, gitleaks), `sqlite` là bước 7 (kèm doc-test ở gate đầy đủ),
   `postgres` là bước 8. Ba phần cộng lại đúng bằng gate không có cờ; `./gate.sh`, `--quick`, `--pr` không cờ chạy như cũ.
   `--quick` không nhận `--only`.
2. `./gate.sh --shard k/n` và `cargo xtask test-all|test-affected … --shard k/n`: bước test chỉ chạy phần thứ k trong n
   phần. Mỗi chương trình test chia kiểu chia bài theo thứ tự `--list` (test thứ i vào phần `i mod n`), chạy bằng
   `--exact <tên…>`; doc-test ở phần 1; gate nhanh không chia và chỉ ghi mốc xanh khi chạy hết.
3. CI (`.github/workflows/ci.yml`): job `lint` và matrix `tests` (`sqlite 1/2`, `sqlite 2/2`, `postgres 16 1/2`,
   `postgres 16 2/2`, thêm hai phần `postgres 18` khi push lên main) chạy song song, mỗi job một
   `./gate.sh … --only … --shard …`. Job `gate` (`needs:` cả hai, `if: always()`) giữ tên check cũ
   `gate (PostgreSQL 16)` ở pull request và chỉ xanh khi mọi phần xanh.
4. `concurrency.cancel-in-progress` chỉ bật cho pull request. Push lên main không huỷ lần chạy đang dở.
5. `Swatinem/rust-cache`: chỉ main ghi cache (`save-if`); các job test dùng chung `shared-key: test` (chỉ `sqlite 1/2` ghi),
   lint có khoá `lint`. Job `sdk` và `upgrade` cũng chỉ ghi ở main.
6. `cargo xtask test-all` / `test-affected` trên Postgres: mỗi chương trình test được ít nhất
   `min(số test, 4)` suất (`POSTGRES_MIN_THREADS`) thay vì chỉ theo tỉ lệ số test. Trần 16 test cùng lúc không đổi.

Lý do: đo trên run 44456618650 (pull request, runner `ubuntu-24.04` 4 CPU, ngày 2026-11-19) job gate mất 33 phút, tuần tự:
clippy 5 phút, build test 9 phút 14 giây, test SQLite 7 phút 12 giây, test Postgres 10 phút 33 giây. rust-cache báo "No
cache found": mọi lần chạy CI trên main trong 13 giờ trước đó đều `cancelled` (mỗi merge mới huỷ lần chạy main trước), trừ một
lần đỏ, nên bước Post của rust-cache trên main không bao giờ chạy; cache của pull request chỉ nhánh đó dùng được. Trong test Postgres, chương
trình test đầu-cuối của `autoxide` (125 test, mỗi test dựng mọi module) chỉ được 1/16 suất vì chia theo số test trên ~2000
test, nên chạy tuần tự 633 giây, sáu phút cuối một mình. Chia ba job thì tổng thời gian là job chậm nhất thay vì tổng ba phần.

Đo lại sau khi chia ba job, có cache (run 44479823483, pull request sửa kernel nên chạy mọi test): cache khôi phục đủ thư viện
ngoài (chỉ còn 36 crate của workspace phải build, 7–11 phút); lint 3 phút 35 giây; Postgres 15 phút (test 454 giây); SQLite 22
phút, trong đó riêng test đầu-cuối của `autoxide` 637 giây trên một máy. Test đó chậm vì mỗi test dựng mọi module, và thời gian
hệ thống chủ yếu là futex giữa các thread của sqlx SQLite (đo bằng `strace -c`; đặt database tạm lên tmpfs chỉ nhanh hơn 10%),
nên chia nó ra nhiều máy là cách giữ nguyên test mà rút ngắn được.

Đã cân nhắc:
- Linker nhanh hơn (mold): Rust 1.90+ đã dùng `rust-lld` mặc định trên `x86_64-unknown-linux-gnu`, nên lợi ích còn lại nhỏ;
  không thêm.
- `CARGO_PROFILE_*_DEBUG=0` ở CI: `[profile.dev]` đã là `line-tables-only` cho crate của workspace và `debug = false` cho
  thư viện ngoài (D-20261118-build-size); bỏ nốt chỉ mất số dòng trong stack trace của test đỏ trên CI. Không làm.
- Tăng `--test-threads` trên SQLite: đo trên máy 4 CPU, chương trình test `autoxide` 4 thread 630 giây, 8 thread 614 giây
  (thời gian hệ thống tăng từ 12 lên 20 phút) — không đáng.
- Một workflow riêng chỉ build để lưu cache trên main: thêm một lần build nữa mỗi lần merge; chạy hết gate trên main đã lưu
  được cache khi không bị huỷ.
- `sccache` để giữ cả crate của workspace không đổi: thêm công cụ và dịch vụ cache; pull request sửa `kernel` vẫn build lại
  hết. Để sau, nếu build vẫn là phần dài nhất.
- Chia nhiều hơn hai phần mỗi database: mỗi phần build lại workspace (7–11 phút) nên lợi ích giảm nhanh, và tốn thêm runner
  (giới hạn job chạy cùng lúc của tài khoản).
- Cache của pull request (`save-if` luôn đúng): mỗi nhánh một bản ~600 MB nén, đẩy cache của main ra khỏi 10 GB của repo.

Hệ quả: lần chạy main mới phải chờ lần đang chạy xong (GitHub chỉ giữ một lần chờ, lần mới nhất), nên main có thể chậm báo
kết quả của commit cuối. Năm máy runner cho phần gate của một pull request thay vì một (repo công khai: không tính phí, D-197); nhiều pull request
cùng lúc có thể phải chờ runner. Mỗi job vẫn
build lại crate của workspace (rust-cache không giữ chúng). Phần chậm nhất còn lại là test đầu-cuối của `autoxide` trên
SQLite (~7 phút trên CI, mỗi test dựng và migrate mọi module); làm nhanh nó là việc của testkit, không phải của CI.
