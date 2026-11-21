## D-20261121-ci-sccache-build · sccache cho job build tests của CI, không cho lint — 2026-11-21 — chủ dự án quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: job `build tests` của `.github/workflows/gate.yml` dùng `sccache` (`mozilla-actions/sccache-action`, backend cache của
GitHub Actions, `RUSTC_WRAPPER=sccache`) cùng `rust-cache` hiện có. Job `lint` không dùng. Không thêm mold: Rust 1.99 trên Linux đã
link bằng `rust-lld` mặc định.

Số đo (đầy đủ ở `docs/BENCHMARKS.md`, mục "sccache cho job build tests"): cache ấm, đổi code thật ở một crate lá, build tests 379 s
so với 605 s của baseline (trung bình 3 lượt) — nhanh hơn 37%, vượt ngưỡng 15% chủ dự án đặt. Lint chậm hơn 31% (398 s so với 305 s)
nên bị loại. Lượt đầu cache lạnh chậm hơn 26%, một lần cho mỗi khoá.

Còn phải theo dõi: PR đổi kernel gần như không trúng cache (mong đợi bằng baseline cộng phần ghi cache); PR mới chỉ đọc cache do `main`
ghi, nên cần vài lượt chạy trên `main` sau khi merge để cache đầy. Nếu sau vài lượt build tests trên PR thực tế không nhanh hơn ≥ 15%
trung bình thì bỏ (ghi quyết định mới thay mục này).

Lý do: build tests là bước dài nhất của CI trước khi các shard test chạy được (10 phút), và hầu hết PR chỉ đổi vài crate.
