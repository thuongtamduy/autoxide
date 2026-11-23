## D-20261123-supply-chain · Chuỗi cung ứng và phát hành: ký không khoá, provenance cho mọi file, Actions tối thiểu quyền, Dependabot có cooldown, bản thử `X.Y.Z-rc.N` — 2026-11-23 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định:

1. **Ký không khoá giữ tay.** Workflow `Release` (và `Release image (existing tag)`) ký bằng Sigstore qua OIDC của
   GitHub Actions — chứng chỉ ngắn hạn cấp theo danh tính của đúng workflow, ghi vào log công khai Rekor:
   - image ghcr.io: chứng thực nguồn gốc build (SLSA, `actions/attest`, đã có) + chứng thực SBOM (đã có) + **chữ ký
     cosign** (`cosign sign`, mới) để người dùng `cosign verify` không cần GitHub CLI;
   - file đính kèm bản phát hành — **binary Linux** `autoxide-X.Y.Z-<target>.tar.gz` (amd64, arm64; lấy đúng file
     trong image vừa build, không build lại), gói SDK `autoxide-sdk-X.Y.Z.tgz`, `sbom.spdx.json`: chứng thực nguồn gốc
     build cho từng file (`actions/attest` với `subject-path`), và `SHA256SUMS` + `SHA256SUMS.sigstore.json` (bundle của
     `cosign sign-blob`).
   Cách kiểm cho người dùng: `docs/VERIFYING.md` + `.vi.md`. Công cụ: `sigstore/cosign-installer` v4.1.3 (ghim SHA),
   cosign v3.1.4.
2. **Workflow tối thiểu quyền và được kiểm tự động.** Mọi workflow `permissions: contents: read` ở đầu, job cần thêm
   thì khai riêng; mọi action ghim SHA (D-20261121-pin-ci-deps); mọi `actions/checkout` đặt `persist-credentials:
   false` (không job nào cần git push — tag tạo bằng API); giá trị ngữ cảnh vào `run:` qua `env:`. Job CI `workflows`
   chạy **zizmor** 1.32.0 (qua `taiki-e/install-action`, có token để kiểm SHA ghim thuộc đúng repo của action) và
   **actionlint** 1.7.12 (tải bản phát hành chính thức, kiểm SHA-256 ghim trong workflow; chạy cả shellcheck). Cấu
   hình `.github/zizmor.yml`: chỉ tắt `self-repository` (gợi ý kiểu viết, không phải lỗ hổng). `auto-release.yml` dùng
   `workflow_run` (zizmor coi là nguy hiểm) có ghi `ignore` kèm lý do, và thêm điều kiện lần CI thuộc chính repo.
   Không có `pull_request_target` nào.
3. **Dependabot** cho cargo, npm (`sdk/typescript`), github-actions, docker, docker-compose; mỗi loại gom một nhóm
   (cargo: nhóm minor + patch, major riêng); `cooldown` 7 ngày — bản bị chiếm quyền thường bị gỡ trong vài ngày đầu;
   bản vá lỗ hổng (security updates) không chịu cooldown.
4. **Bản thử trước bản major** (`1.0.0-rc.N`, sau này `2.0.0-rc.N`): chỉ dạng `X.Y.Z-rc.N` (job `verify` của
   `Release` từ chối dạng khác). Bản thử là pre-release trên GitHub (kèm dòng cảnh báo ở đầu ghi chú), image chỉ có thẻ
   `X.Y.Z-rc.N` — không `X.Y`, không `latest`. Bản chính thức chỉ nhận `latest` (image và "Latest release" của GitHub)
   khi là bản chính thức cao nhất: bản vá dòng cũ (`release/X.Y`) không giành `latest` (sửa luôn chỗ RELEASING ghi
   "workflow chưa phân biệt"). CHANGELOG: mỗi bản thử một mục `## [X.Y.Z-rc.N]`; mục của bản chính thức gồm thay đổi
   sau bản thử cuối và một dòng dẫn tới các mục bản thử. Quy trình, đóng băng tính năng và tiêu chí từ RC lên 1.0 ở
   `docs/RELEASING.md`, mục "Bản thử trước bản major". Mở rc.1 và lên 1.0.0 vẫn do chủ dự án quyết (version 1.0 là
   "chủ dự án tuyên bố API ổn định").
5. `SECURITY.md` + `.vi.md`: báo riêng qua GitHub private vulnerability reporting (đã có), thêm chính sách công bố
   phối hợp (CVE qua GitHub Security Advisory, mốc 90 ngày) và dẫn tới cách kiểm chữ ký.

Lý do: trước 1.0 người lạ cần kiểm được thứ họ tải là do đúng repo này build từ đúng tag, không phải tin máy chủ tải
về; Sigstore keyless làm được mà không có khoá bí mật nào để giữ, xoay vòng hay để lộ, và miễn phí cho repo công khai.
Workflow phát hành có quyền ghi package và tạo tag nên là đích tấn công giá trị nhất; zizmor bắt đúng các lỗi kiểu đó
(lộ token qua checkout, chèn lệnh từ tên nhánh, SHA giả mạo) mà review bằng mắt hay sót. Lần rà đầu (zizmor 1.33.1,
ngày 2026-11-23) thấy 17 checkout giữ credential, một `${{ }}` trong `run:`, một `workflow_run` chưa ghi lý do — đã
sửa hết.

Đã cân nhắc: khoá cosign giữ trong secret (phải giữ, xoay vòng, có thể lộ — trái mục tiêu "không khoá giữ tay"); chỉ
dùng `actions/attest` không cosign (đủ cho `gh attestation verify`, nhưng nhiều công cụ triển khai — policy controller
của Kubernetes, Kyverno — đọc chữ ký cosign); ký từng file bằng cosign (sáu file chữ ký thay vì một `SHA256SUMS` ký —
cùng độ an toàn, nhiều file hơn); build binary riêng bằng `cargo build` trên máy build (thêm 2 lần build dài, và binary
không chắc trùng với thứ trong image); chạy zizmor trong `./gate.sh` (cần cài thêm công cụ Python trên mọi máy dev,
trong khi workflow chỉ chạy trên GitHub); tag RC dạng tự do `-beta`, `-alpha` (một dạng duy nhất dễ kiểm và dễ hiểu).
