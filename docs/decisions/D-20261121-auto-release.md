## D-20261121-auto-release · Merge PR phát hành là phát hành: GitHub tự tạo tag và chạy Release khi CI của commit đó xanh — 2026-11-21 — chủ dự án quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. Workflow mới `.github/workflows/auto-release.yml` chạy khi workflow CI (`ci.yml`) của một lần **push vào `main`**
   kết thúc `success` (`workflow_run`). Nó đọc `version` trong `Cargo.toml` của **đúng commit vừa chạy CI**:
   - tag `vX.Y.Z` đã có → không làm gì (mọi lần push không đổi phiên bản);
   - `CHANGELOG.md` chưa có mục `## [X.Y.Z]` → không làm gì (chưa phải commit phát hành);
   - còn lại → tạo tag có chú thích `vX.Y.Z` trên commit đó (API git của GitHub, `GITHUB_TOKEN`, quyền
     `contents: write`), rồi gọi workflow Release trên tag bằng `workflow_dispatch` (quyền `actions: write`).
2. Release giữ nguyên: kiểm tag khớp phiên bản và CHANGELOG, job `ci-result` thấy CI xanh của đúng SHA nên bỏ qua
   gate (D-20261121-ci-batch-release-faster), build image amd64 + arm64, chứng thực, SBOM, tạo trang Release.
3. Quy trình agent (docs/RELEASING.md): làm bước 1–4 như cũ, mở PR phát hành, chờ CI xanh, merge. Không còn bước
   "gắn tag" bằng tay hay nhờ chủ dự án bấm Publish. Đẩy tag tay vẫn chạy được như trước (trigger `push: tags` của
   Release giữ nguyên) cho trường hợp cần.

Vì sao phải gọi Release bằng `workflow_dispatch`: GitHub không kích hoạt workflow theo `push: tags` cho tag do
`GITHUB_TOKEN` tạo (chống vòng lặp), nhưng cho phép `workflow_dispatch`. Chạy trên ref của tag nên `github.ref_name`,
`github.sha` và `docker/metadata-action` (thẻ semver) giống hệt khi đẩy tag tay.

Vì sao an toàn:
- Chỉ phát hành commit đã có CI đầy đủ xanh trên `main` (gate SQLite + Postgres, upgrade-test, quét secret): đúng
  D-300.
- Chỉ commit có số phiên bản mới **và** mục CHANGELOG khớp mới phát hành; PR thường không bao giờ kích hoạt.
- Không cần token cá nhân hay secret nào: chỉ `GITHUB_TOKEN` với quyền tối thiểu của job. Miễn phí (Actions không
  giới hạn với repo công khai). Repo riêng tư thì workflow bỏ qua như Release (D-197).
- Quyền quyết phát hành nằm ở việc merge PR phát hành; khi nào mở PR đó vẫn theo D-248 (xong một giai đoạn hoặc chủ
  dự án bảo).

Lý do: môi trường của agent không đẩy được tag và không có công cụ tạo Release, nên mỗi lần phát hành phải chờ chủ
dự án bấm Publish trên GitHub. Chủ dự án muốn agent tự phát hành, cách dễ và miễn phí.

Đã cân nhắc: nút `workflow_dispatch` "phát hành" cho agent bấm qua API (phụ thuộc quyền chạy workflow của phiên agent,
dễ bị chặn); token cá nhân để tag đẩy kích hoạt Release (thêm secret phải giữ và xoay vòng); tạo tag trong `ci.yml`
(trộn kiểm tra với phát hành, chạy cả ở PR).

Thay: bước 6 (gắn tag tay) và phần "nhờ chủ dự án tạo trên GitHub" của docs/RELEASING.md.
