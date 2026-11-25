# Image gitleaks của secret scan, ghim theo digest (I-5 của docs/security/review-2026-11.md,
# D-20261125-gitleaks-digest). File này không để build: nó là chỗ DUY NHẤT ghi image, để
# `gate.sh` và job `secrets` của `.github/workflows/ci.yml` đọc cùng một dòng `FROM`, và để
# Dependabot (hệ `docker` trong `.github/dependabot.yml`) đề xuất digest mới như với `Dockerfile`.
# Thẻ chỉ để người đọc và Dependabot; Docker kéo đúng digest. Nâng thì nâng cả
# GITLEAKS_VERSION / GITLEAKS_SHA256 trong `scripts/cloud-setup.sh` (binary cho máy không có Docker).
FROM ghcr.io/gitleaks/gitleaks:v8.30.1@sha256:c00b6bd0aeb3071cbcb79009cb16a60dd9e0a7c60e2be9ab65d25e6bc8abbb7f
