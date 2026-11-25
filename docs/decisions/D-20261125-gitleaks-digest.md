## D-20261125-gitleaks-digest · Image gitleaks ghim theo digest ở `.github/gitleaks/Dockerfile`, gate và CI đọc cùng dòng — 2026-11-25 — agent quyết

Chủ đề: Quy trình, gate, lộ trình

Quyết định: image gitleaks của secret scan ghim theo digest
(`ghcr.io/gitleaks/gitleaks:v8.30.1@sha256:…`) trong dòng `FROM` của `.github/gitleaks/Dockerfile` — file chỉ để
ghi image, không build. `gate.sh` (mặc định của `GATE_GITLEAKS_IMAGE`) và job `secrets` của
`.github/workflows/ci.yml` đọc dòng đó bằng `sed`; Dependabot (hệ `docker`, thêm thư mục `/.github/gitleaks`) đề xuất
digest mới mỗi tuần như với `Dockerfile`. Sửa I-5 của `docs/security/review-2026-11.md`.

Lý do: thẻ (`v8.30.1`) có thể bị đẩy lại trỏ sang image khác; mọi action và image nền khác đã ghim theo digest
(D-20261123-supply-chain). Dependabot không đọc chuỗi image trong lệnh `docker run` của một bước `run:` hay trong
script shell, nhưng đọc dòng `FROM` của Dockerfile — nên một Dockerfile làm chỗ ghim duy nhất cho cả hai nơi dùng.
Digest lấy từ `ghcr.io` (header `docker-content-digest` của image index đa nền tảng) và kiểm bằng SHA-256 của chính
nội dung index tải về.

Đã cân nhắc: viết digest thẳng vào `gate.sh` và `ci.yml` — hai chỗ phải nâng tay, Dependabot không thấy; `container:`
của job GitHub Actions — gate trên máy vẫn cần chỗ ghim riêng.

Thay: phần "Nâng phiên bản về sau: đổi đủ ba chỗ" của D-20261122-gitleaks-8-30-1 — nay là hai chỗ:
`.github/gitleaks/Dockerfile` (gate và CI) và `scripts/cloud-setup.sh` (binary).
