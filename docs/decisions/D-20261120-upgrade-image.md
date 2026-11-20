## D-20261120-upgrade-image · Workflow chạy tay đăng image cho tag phát hành đã có; job upgrade dùng image đó thay vì build lại bản cũ — 2026-11-20 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. Thêm `.github/workflows/release-image.yml` ("Release image (existing tag)"), chỉ `workflow_dispatch`, nhận
   `tag` (vd `v0.14.0`): chạy từ `main` nhưng build mã nguồn của tag, đẩy lên `ghcr.io/<owner>/<repo>` đúng hai thẻ
   như `release.yml` (`X.Y.Z`, `X.Y`) cho `linux/amd64` và `linux/arm64`, kèm chứng thực nguồn gốc build. Không chạy
   gate, không tạo bản phát hành GitHub, không tạo hay đổi tag.
2. `release.yml` giữ nguyên tên và thẻ image (đã khớp `cargo xtask upgrade-test`); một test của xtask đọc hai workflow
   và kiểm chúng đặt đúng tên, đúng thẻ `X.Y.Z` mà `upgrade-test` kéo. Không kéo được image thì `upgrade-test` in tên
   workflow cần chạy.

Lý do: job "upgrade from the last release" của CI báo `manifest unknown` khi kéo `ghcr.io/thuongtamduy/autoxide:0.14.0`
rồi build lại bản cũ từ tag (~10 phút mỗi lần, mỗi database). Tên không sai: chưa từng có image nào được đẩy. Workflow
Release của các tag v0.12.0, v0.13.0, v0.14.0 đều `skipped` (đẩy tag lúc repo còn riêng tư, D-197 bỏ qua mọi job); lần
chạy tay Release trên `v0.14.0` (run 44279708662) đỏ ở bước `cargo deny` của gate vì advisory mới của `lettre` công bố
sau ngày phát hành, nên các job `image` / `publish` không chạy. Chạy tay Release trên tag cũ còn dùng chính file
workflow ở tag đó, nên không sửa được bằng cách đổi `release.yml` trên `main`. Quyền không phải nguyên nhân: job `image`
có `packages: write`, job `upgrade` có `packages: read` và đăng nhập bằng `GITHUB_TOKEN` (gói do workflow của repo đẩy
thừa hưởng quyền của repo).

Đã cân nhắc:
- Thêm input "chỉ image, bỏ gate" vào `release.yml`: chạy tay trên tag thì GitHub dùng `release.yml` của tag (chưa có
  input), nên không dùng được cho tag đã có.
- Chạy lại Release đầy đủ sau khi vá advisory: phải phát hành bản mới; image của tag cũ vẫn thiếu.
- Job `upgrade` tự đẩy image nó build: pull request không có quyền ghi gói, và image build trong CI của một commit khác
  tag không phải image phát hành.

Hệ quả: chủ dự án chạy workflow một lần cho tag gần nhất (`v0.14.0`); từ bản phát hành sau, `release.yml` tự đẩy image
khi tag qua gate. Gói trên ghcr.io lần đầu là riêng tư: CI của repo vẫn kéo được (cùng repo); người ngoài kéo được sau
khi chủ dự án đổi gói sang Public (docs/RELEASING.md).
