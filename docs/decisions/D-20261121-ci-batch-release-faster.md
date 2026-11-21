## D-20261121-ci-batch-release-faster · PR lô chạy CI đầy đủ; phát hành dùng lại CI xanh của đúng commit; upgrade-test chỉ khi đụng dạng lưu — 2026-11-21 — chủ dự án quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. **PR ghép lô ("Lô: …") chạy CI đầy đủ** như mọi PR: bỏ điều kiện bỏ qua job nặng theo tiêu đề "Lô:" ở `ci.yml`.
   Phiên điều phối ghép, sinh lại file tự động, chạy `cargo xtask check` + `cargo xtask sdk --check`, push PR lô và **chờ
   CI xanh rồi mới merge**; không chạy `test-affected` trên máy nữa. Thay phần "kiểm trên máy" của
   D-20261119-merge-batches và toàn bộ yêu cầu chạy Postgres trên máy của D-20261120-batch-postgres (CI của PR đã chạy
   Postgres 16). Phần còn lại của D-20261119-merge-batches (một PR cho một cụm việc, file sinh tự động không gộp tay)
   giữ nguyên.
2. **Phát hành dùng lại kết quả CI của đúng commit.** Trong `release.yml`, job `ci-result` hỏi API Actions (bằng
   `GITHUB_TOKEN`, quyền `actions: read`) lần chạy `ci.yml` của push vào `main` trên đúng SHA của tag. `completed` +
   `success` thì bỏ qua job `gate`; chưa có, đang chạy hay đỏ thì `gate` chạy như trước (không chờ). Job `image` chấp
   nhận gate `success` hoặc `skipped`, từ chối `failure`/`cancelled`.
   **Vì sao vẫn đúng D-300:** D-300 đòi gate đầy đủ SQLite + Postgres xanh trước khi phát hành. Lần chạy CI trên `main`
   của cùng SHA chính là gate đầy đủ đó (build archive, SQLite, Postgres 16 và 18, doc-test) cộng upgrade-test và quét
   secret, trên cùng mã nguồn bit-cho-bit; kết luận của workflow chỉ `success` khi mọi job xanh. Không có kết quả xanh
   của đúng SHA thì vẫn chạy gate, nên không có đường nào phát hành mà chưa có gate xanh.
3. **upgrade-test ở PR chỉ chạy khi PR đụng thứ có thể đổi dạng dữ liệu đã lưu:** đường dẫn `migration(s)`, `entity`,
   `crates/kernel/src/data/`, `Cargo.lock`, `xtask/src/upgrade.rs`, test nâng cấp, `.github/workflows/`, `Dockerfile`.
   Kiểm trong một bước (không dùng `paths:`) nên job vẫn báo xong và không chặn check bắt buộc. Push vào `main` và
   `workflow_dispatch` luôn chạy.

Lý do: ghép lô trên máy tốn thời gian của phiên điều phối và đã để lọt lỗi chỉ Postgres bắt được; CI chạy song song
nhiều máy nên chờ xanh vừa an toàn hơn vừa không chiếm máy phiên điều phối. Phát hành chạy lại đúng thứ vừa chạy xong
trên `main` là lãng phí. upgrade-test tải image và build tag cũ (tới 60 phút) trong khi phần lớn PR không đụng dạng lưu.

Đã cân nhắc: chờ CI đang chạy trong release.yml (phức tạp, tag thường đẩy sau khi main xanh); lọc theo `paths:` ở
trigger (job không báo xong, có thể kẹt check bắt buộc).

Thay: phần "kiểm trên máy" của D-20261119-merge-batches và D-20261120-batch-postgres (mục 1).
