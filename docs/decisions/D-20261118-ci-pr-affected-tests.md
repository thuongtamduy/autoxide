## D-20261118-ci-pr-affected-tests · CI của pull request chỉ test crate bị sửa, trên cả SQLite và Postgres, không doc-test (`./gate.sh --pr`; D-299, D-300, D-20261118-public-ci-openapi-dup) — 2026-11-18 — chủ dự án quyết hướng, agent quyết cách làm

Chủ đề: Quy trình, gate, lộ trình

Bối cảnh: job `gate` của CI chạy trọn `./gate.sh` khoảng 25 phút trên runner miễn phí của GitHub (4 CPU). Đo trên một lần
chạy của pull request #128: clippy 2 phút 40 giây; build test cả workspace 6 phút 30 giây; chạy mọi test trên SQLite khoảng 3
phút 40 giây; build và chạy doc-test 2 phút 54 giây; mọi test trên Postgres khoảng 7 phút 30 giây. Chủ dự án thấy chờ quá lâu
và chọn: pull request chỉ test phần bị sửa; bỏ lần build thứ hai (doc-test) ở pull request.

Quyết định:

1. `./gate.sh --pr`: mọi bước kiểm tĩnh như gate đầy đủ (fmt, clippy, ranh giới, hợp đồng API, SDK, deny, gitleaks), rồi
   `cargo xtask test-affected --base "$GATE_BASE" --keep-env` trên SQLite rồi trên Postgres. `GATE_BASE` mặc định
   `origin/main`; CI checkout cả lịch sử (`fetch-depth: 0`) nên có ref này. Không chạy doc-test.
2. `test-affected --keep-env`: chọn crate như gate nhanh (crate bị sửa và crate phụ thuộc vào chúng; sửa file build chung thì
   chạy hết) nhưng chạy trên database và S3 của môi trường như `test-all` (Postgres thì giới hạn số test cùng lúc). Không ghi
   mốc `refs/autoxide/quick-green`: mốc đó chỉ cho gate nhanh trên SQLite.
3. CI: `pull_request` chạy `./gate.sh --pr`; push lên `main` và chạy tay chạy `./gate.sh` đầy đủ (cả doc-test, PostgreSQL 16
   và 18).

Đã cân nhắc: `cargo test -p <crate>` để build ít hơn — build lại sqlx, sea-orm… thành bộ file thứ hai vì hợp feature khác (lý
do có sẵn trong `affected.rs`, D-299), chậm hơn; bỏ hẳn test SQLite ở pull request — Luật số một cần cả hai database.

Hệ quả: pull request sửa `kernel` hay file build chung vẫn chạy mọi test (như trước, trừ doc-test); pull request chỉ sửa một
module tiết kiệm phần lớn thời gian chạy test. Build test cả workspace vẫn là phần lớn nhất còn lại. Doc-test hỏng và lỗi ở
crate không bị chạm (vd do dữ liệu dùng chung) chỉ lộ ở lần push lên `main`. D-300 không đổi.
