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
Số đo sau: xem pull request của thay đổi này.

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
