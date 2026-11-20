## D-20261120-ci-nextest-archive · CI build test một lần thành archive của nextest, các máy test chỉ tải về và chạy — 2026-11-20 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. `./gate.sh [--pr] --only build`: `cargo nextest archive --workspace` build mọi chương trình test MỘT lần thành
   `target/gate-archive/tests.tar.zst` (đổi chỗ bằng `GATE_ARCHIVE_DIR`), kèm file `filter` — filterset của nextest
   chọn test: `all()` ở gate đầy đủ; ở `--pr` là `cargo xtask test-filter --base <ref>` (lệnh mới, cùng cách chọn crate
   với `test-affected`: crate bị sửa và crate phụ thuộc). Gate đầy đủ chạy doc-test ở phần này (cần trình biên dịch).
2. `./gate.sh [--pr] --only sqlite|postgres --archive <thư mục> [--shard k/n]`: chạy test từ archive bằng
   `cargo-nextest nextest run --archive-file … --partition count:k/n --filterset "$(cat filter)"`, không biên dịch gì,
   gọi thẳng `cargo-nextest` (máy chạy không cần toolchain ghim). Archive giải nén vào chính thư mục repo
   (`--extract-to "$PWD"`): test dùng `env!("CARGO_BIN_EXE_autoxide")`, đường dẫn ghi lúc build. Postgres: tối đa
   16 test cùng lúc (`--test-threads 16`, bằng `POSTGRES_CONCURRENT_TESTS` của `cargo xtask test-all`, dưới trần 300
   kết nối). SQLite: hai test mỗi nhân.
3. `./gate.sh`, `--quick`, `--pr` không cờ và `--only lint|sqlite|postgres` không `--archive` chạy như cũ (build rồi
   chạy bằng `cargo xtask`). `lint` + `build` + `sqlite --archive` + `postgres --archive` cộng lại đúng bằng gate.
4. CI (`ci.yml`): job `gate / build tests` (cache `shared-key: test`, chỉ main ghi) build archive, đẩy lên artifact
   (giữ một ngày); matrix `tests` cần job này, tải archive, chạy 4 phần mỗi database (`sqlite k/4`, `postgres 16 k/4`,
   thêm `postgres 18 k/4` khi push lên main). Job tổng `gate (PostgreSQL 16)` / `gate (PostgreSQL 16, 18)` cần cả
   `lint`, `build`, `tests`. `cargo-nextest` cài bằng `taiki-e/install-action` (STACK.md).

Lý do: trước đây mỗi job test tự build lại cả workspace (7–11 phút có cache, D-20261119-ci-faster), nên chia thêm máy
không rút ngắn được: mỗi máy trả thêm một lần build. Đo trên run 44614887150 (main): mỗi phần test 13–24 phút. Build
một lần rồi phát archive thì thời gian là build + phần chạy chậm nhất, và chia thêm máy chỉ tốn thời gian tải archive.
Số đo sau (chạy đầy đủ bằng workflow_dispatch trên nhánh, run 44670917141; 12 shard, mỗi shard một phần tư của 2189 test,
đều xanh, chỉ `lint` đỏ vì advisory `humantime` đã được sửa ở main): `build tests` 9,1 phút (cache nguội một phần); mỗi shard
2,5–4,8 phút, trong đó tải archive 1,3 GB (~1 phút) và giải nén (~20 giây); phần test thật 36 giây (shard sqlite 1/4: 563
test). Đường găng khi có đủ máy: build 9,1 + shard chậm nhất 4,8 ≈ 14 phút, so với 17–25 phút (trung vị ~21) của năm lần
chạy đầy đủ gần nhất trên main (run 463: 21,7; 461: 17,3; 456: 24,5; 465: 19,5; 459: 18,4). Tổng thời gian wall của chính
run này là 27 phút vì 12 shard xếp hàng chờ máy (nhiều run chạy cùng lúc, giới hạn máy đồng thời của GitHub) — số 14 phút
là đường găng, không phải wall đo được. Phần chạy test (đã tính database mẫu của #221) không còn là chỗ tốn: build và tải
archive mới là chỗ tốn; job `upgrade` (11,7–15,4 phút, build lại bản cũ) giờ dài hơn cả chuỗi build + test và sẽ ngắn lại
khi chủ dự án chạy workflow image cho v0.14.0 (D-20261120-upgrade-image).

nextest chạy mỗi test trong một tiến trình riêng: test không còn chia trạng thái trong tiến trình (dọn rác của testkit
chạy ở mỗi test — một truy vấn nhỏ). Database mẫu đã migrate của testkit (pull request riêng) dùng chung giữa các tiến
trình qua file / database mẫu, nên vẫn có tác dụng khi mỗi test một tiến trình.

Đã cân nhắc:
- Giữ `cargo xtask test-all` ở máy chạy, chỉ chép thư mục `target/` qua artifact: `target/` có cả thư viện ngoài và file
  trung gian (hàng chục GB), và `cargo test` vẫn kiểm lại fingerprint, có thể build lại. Archive của nextest chỉ gồm
  chương trình test, chương trình cần cho test và metadata, có sẵn cách đổi đường dẫn.
- Chạy tất cả gate trên máy bằng nextest: gate trên máy không cần chia, `cargo xtask` đủ và không đòi cài thêm công cụ.
- `--partition hash:k/n`: ổn định khi thêm test nhưng chia lệch hơn `count` (round-robin theo thứ tự).

Hệ quả: thêm một job (build) đứng trước các job test; job test không cần Rust nhưng cần `cargo-nextest`. Artifact
archive mỗi lần chạy giữ một ngày. Pull request dùng 10 máy cho gate thay vì 5 (repo công khai: không tính phí, D-197).
