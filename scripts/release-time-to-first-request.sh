#!/usr/bin/env bash
# Đo "tải về tới request đầu tiên" của một bản phát hành (71-G9; ngưỡng 1.0: dưới 300 giây), đi đúng đường của người
# lạ trong README (Quickstart): tải compose.quickstart.yml, tạo .env, `docker compose up -d` (kéo image của bản đó),
# chờ sẵn sàng, đọc mật khẩu admin sinh ra, đăng nhập, rồi gọi `GET /api/v1/auth/me` có xác thực.
#
#   scripts/release-time-to-first-request.sh <phiên bản không có chữ v, vd 1.0.0> [owner/repo]
#
# In một dòng JSON ra stdout: {"version":…,"seconds":…,"steps":{"download":…,"start":…,"ready":…,"first_request":…}}.
# Cần: docker (có compose), curl, openssl, python3. Chạy trên máy sạch (runner CI): máy đã có image thì số đo thấp giả.
# ĐÂY CHỈ LÀ PHẦN MÁY (tải, khởi động, một request): thời gian người đọc tài liệu không có ở đây — xem chỉ số "người mới
# xong module đầu tiên" trong docs/RELEASING.md.
set -euo pipefail

version="${1:?thiếu phiên bản (vd 1.0.0)}"
repo="${2:-thuongtamduy/autoxide}"
port=8080
work="$(mktemp -d)"
cd "$work"
compose=(docker compose -f compose.quickstart.yml)
cleanup() { "${compose[@]}" down -v >/dev/null 2>&1 || true; rm -rf "$work"; }
trap cleanup EXIT

now() { python3 -I -c 'import time; print(time.time())'; }
t0="$(now)"
curl -fsSLO "https://raw.githubusercontent.com/$repo/v$version/compose.quickstart.yml"
t_download="$(now)"
printf 'AUTOXIDE_AUTH_TOKEN_SECRET=%s\nAUTOXIDE_SECRET_KEY=%s\nAUTOXIDE_IMAGE=ghcr.io/%s:%s\n' \
  "$(openssl rand -base64 48 | tr -d '\n')" "$(openssl rand -base64 48 | tr -d '\n')" "${repo,,}" "$version" > .env
"${compose[@]}" up -d >&2
t_start="$(now)"
for _ in $(seq 1 600); do curl -sf "http://127.0.0.1:$port/health/ready" >/dev/null && break; sleep 0.5; done
curl -sf "http://127.0.0.1:$port/health/ready" >/dev/null || { "${compose[@]}" logs >&2; echo "server not ready" >&2; exit 1; }
t_ready="$(now)"
# File mật khẩu: dòng 1 email, dòng 2 mật khẩu (README, "The first administrator's password").
creds="$("${compose[@]}" exec -T autoxide cat data/initial-admin-password)"
email="$(printf '%s\n' "$creds" | sed -n 1p)"
password="$(printf '%s\n' "$creds" | sed -n 2p)"
# Mật khẩu đi qua biến môi trường của Python và stdin của curl, không qua đối số dòng lệnh (`ps` thấy được đối số).
token="$(EMAIL="$email" PASSWORD="$password" python3 -I -c 'import json,os; print(json.dumps({"email": os.environ["EMAIL"], "password": os.environ["PASSWORD"]}))' \
  | curl -sf "http://127.0.0.1:$port/api/v1/auth/login" -H 'content-type: application/json' -d @- \
  | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["data"]["access_token"])')"
curl -sf "http://127.0.0.1:$port/api/v1/auth/me" -H "authorization: Bearer $token" >/dev/null
t_end="$(now)"
python3 -I - "$version" "$t0" "$t_download" "$t_start" "$t_ready" "$t_end" <<'PY'
import json, sys
version, *t = sys.argv[1], *map(float, sys.argv[2:])
t0, download, start, ready, end = t
print(json.dumps({"version": version, "seconds": round(end - t0, 1), "steps": {
    "download": round(download - t0, 1), "start": round(start - download, 1),
    "ready": round(ready - start, 1), "first_request": round(end - ready, 1)}}))
PY
