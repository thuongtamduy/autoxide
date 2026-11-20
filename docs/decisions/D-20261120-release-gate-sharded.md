## D-20261120-release-gate-sharded · Gate của phát hành dùng chung workflow chia máy với CI (`gate.yml`, `workflow_call`) — 2026-11-20 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Bối cảnh: job `gate` của `release.yml` chạy cả `./gate.sh` trên MỘT máy (timeout 60 phút: build, rồi test SQLite và
Postgres tuần tự), trong khi CI đã chia gate thành `lint` + `build tests` (archive nextest) + 8 shard
(D-20261119-ci-faster, D-20261120-ci-nextest-archive). Phát hành là chỗ cần gate nhanh và đủ nhất, nhưng lại là chỗ chậm nhất.

Quyết định:

1. `.github/workflows/gate.yml` (mới, `on: workflow_call`) chứa ba job `lint`, `build tests`, `tests` (matrix) lấy nguyên từ
   `ci.yml`, thêm đầu vào `postgres-18` (boolean): bật thì matrix chạy cả PostgreSQL 16 và 18, tắt thì chỉ 16.
   Ngữ cảnh `github` của workflow dùng chung là của workflow gọi, nên pull request vẫn chạy `--pr`, còn push, tag và chạy tay
   chạy gate đầy đủ; cache `rust-cache` vẫn chỉ main ghi.
2. `ci.yml` gọi nó bằng một job tên `gate` (`uses: ./.github/workflows/gate.yml`, `postgres-18` bật khi không phải pull
   request) nên tên check giữ nguyên ("gate / lint", "gate / build tests", "gate / sqlite 1/4"…). Job tổng
   `gate (PostgreSQL …)` chờ job gọi đó. Điều kiện "repo công khai" và "PR ghép lô" ở job gọi.
3. `release.yml`: job `verify` (mới, vài giây) giữ kiểm riêng của phát hành — tag khớp `version` trong Cargo.toml, CHANGELOG.md
   có mục của bản — rồi job `gate` gọi `gate.yml` với `postgres-18: false` (PostgreSQL 16 như trước; bản 18 đã chạy ở lần push
   lên main của cùng commit). `image` chờ cả `verify` và `gate`.
4. `lint` của phát hành nay checkout đủ lịch sử và tag (`fetch-depth: 0`) như CI, cần cho `cargo xtask api-diff` và gitleaks
   (job cũ checkout nông nên `api-diff` báo lỗi shallow clone). Lưu ý: khi chạy trên chính tag mới, `git describe` trả về
   tag đó nên api-diff so hợp đồng với chính nó; kiểm phá vỡ thật vẫn ở CI của main và trong `cargo xtask semver`/api-diff
   lúc chuẩn bị phát hành.
5. Test `ci_va_phat_hanh_dung_chung_workflow_gate` (xtask) giữ cả hai file gọi `gate.yml` và `release.yml` không quay lại
   `./gate.sh` cả cục.

Đã cân nhắc: sao chép matrix sang release.yml (lặp cấu hình, hai nơi lệch nhau); composite action (không chứa được `services`,
`strategy.matrix` và nhiều job); `workflow_run` từ CI (khó gắn với tag, chạy lại tay không rõ).

Hệ quả: gate phát hành thành đường găng `build tests` + shard chậm nhất (~14 phút khi có đủ máy, D-20261120-ci-nextest-archive)
thay vì cả gate tuần tự trên một máy; tốn nhiều máy hơn (lint + build + 8 shard), repo công khai không tính phí (D-197).
Đổi tên hay thêm job trong `gate.yml` làm đổi tên check bắt buộc của branch protection ("gate / …").
