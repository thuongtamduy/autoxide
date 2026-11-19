## D-20261119-ci-faster · CI chia gate thành ba job song song, main lưu cache, Postgres chia suất có sàn — 2026-11-19 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. `./gate.sh --only lint|sqlite|postgres` (đi kèm gate đầy đủ hoặc `--pr`) chạy một phần của gate: `lint` là bước 1–6
   (fmt, clippy, xtask check, api-diff, sdk --check, deny, gitleaks), `sqlite` là bước 7 (kèm doc-test ở gate đầy đủ),
   `postgres` là bước 8. Ba phần cộng lại đúng bằng gate không có cờ; `./gate.sh`, `--quick`, `--pr` không cờ chạy như cũ.
   `--quick` không nhận `--only`.
2. CI (`.github/workflows/ci.yml`): job `lint` và matrix `tests` (`sqlite`, `postgres 16`, thêm `postgres 18` khi push lên
   main) chạy song song, mỗi job một `./gate.sh … --only …`. Job `gate` (`needs:` cả hai, `if: always()`) giữ tên check cũ
   `gate (PostgreSQL 16)` ở pull request và chỉ xanh khi mọi phần xanh.
3. `concurrency.cancel-in-progress` chỉ bật cho pull request. Push lên main không huỷ lần chạy đang dở.
4. `Swatinem/rust-cache`: chỉ main ghi cache (`save-if`); hai job test dùng chung `shared-key: test` (chỉ job SQLite ghi),
   lint có khoá `lint`. Job `sdk` và `upgrade` cũng chỉ ghi ở main.
5. `cargo xtask test-all` / `test-affected` trên Postgres: mỗi chương trình test được ít nhất
   `min(số test, 4)` suất (`POSTGRES_MIN_THREADS`) thay vì chỉ theo tỉ lệ số test. Trần 16 test cùng lúc không đổi.

Lý do: đo trên run 44456618650 (pull request, runner `ubuntu-24.04` 4 CPU, ngày 2026-11-19) job gate mất 33 phút, tuần tự:
clippy 5 phút, build test 9 phút 14 giây, test SQLite 7 phút 12 giây, test Postgres 10 phút 33 giây. rust-cache báo "No
cache found": mọi lần chạy CI trên main trong 13 giờ trước đó đều `cancelled` (mỗi merge mới huỷ lần chạy main trước), trừ một
lần đỏ, nên bước Post của rust-cache trên main không bao giờ chạy; cache của pull request chỉ nhánh đó dùng được. Trong test Postgres, chương
trình test đầu-cuối của `autoxide` (125 test, mỗi test dựng mọi module) chỉ được 1/16 suất vì chia theo số test trên ~2000
test, nên chạy tuần tự 633 giây, sáu phút cuối một mình. Chia ba job thì tổng thời gian là job chậm nhất thay vì tổng ba phần.

Đã cân nhắc:
- Linker nhanh hơn (mold): Rust 1.90+ đã dùng `rust-lld` mặc định trên `x86_64-unknown-linux-gnu`, nên lợi ích còn lại nhỏ;
  không thêm.
- `CARGO_PROFILE_*_DEBUG=0` ở CI: `[profile.dev]` đã là `line-tables-only` cho crate của workspace và `debug = false` cho
  thư viện ngoài (D-20261118-build-size); bỏ nốt chỉ mất số dòng trong stack trace của test đỏ trên CI. Không làm.
- Tăng `--test-threads` trên SQLite: đo trên máy 4 CPU, chương trình test `autoxide` 4 thread 630 giây, 8 thread 614 giây
  (thời gian hệ thống tăng từ 12 lên 20 phút) — không đáng.
- Một workflow riêng chỉ build để lưu cache trên main: thêm một lần build nữa mỗi lần merge; chạy hết gate trên main đã lưu
  được cache khi không bị huỷ.
- Cache của pull request (`save-if` luôn đúng): mỗi nhánh một bản ~600 MB nén, đẩy cache của main ra khỏi 10 GB của repo.

Hệ quả: lần chạy main mới phải chờ lần đang chạy xong (GitHub chỉ giữ một lần chờ, lần mới nhất), nên main có thể chậm báo
kết quả của commit cuối. Ba máy runner cho một pull request thay vì một (repo công khai: không tính phí, D-197). Mỗi job vẫn
build lại crate của workspace (rust-cache không giữ chúng). Phần chậm nhất còn lại là test đầu-cuối của `autoxide` trên
SQLite (~7 phút trên CI, mỗi test dựng và migrate mọi module); làm nhanh nó là việc của testkit, không phải của CI.
