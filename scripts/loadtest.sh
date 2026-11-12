#!/usr/bin/env bash
# Đo hiệu năng và chịu tải qua API THẬT (D-076): binary release, `autoxide serve` thật,
# TCP thật, đăng nhập thật (access token, API key), database thật. Xem docs/BENCHMARKS.md.
#
#   scripts/loadtest.sh                                   # SQLite (file tạm)
#   LOADTEST_DATABASE_URL=postgres://u:p@host:5432/db scripts/loadtest.sh   # database PHẢI trống
#
# Cần: cargo, curl, python3, oha (`cargo install oha --locked`).
# Biến (không bắt buộc): LOADTEST_DURATION (mặc định 10s), LOADTEST_CONNECTIONS (64),
# LOADTEST_RAMP ("1 16 64 256 1024"), LOADTEST_PORT (18080).
#
# Chế độ chạy dài (soak): LOADTEST_SOAK=15m chạy hỗn hợp đọc / ghi / file / MCP liên tục
# trong thời gian đó, đo bộ nhớ (RSS), số file đang mở của máy chủ và số kết nối
# PostgreSQL mỗi 30 giây, rồi kết luận có rò rỉ hay không — thay cho các bài đo ngắn.
#
# Máy tạo tải và máy chủ chạy chung một máy nên tranh CPU: số đo là cận dưới, dùng để so
# trước / sau một thay đổi trên cùng máy.

set -euo pipefail
cd "$(dirname "$0")/.."

DURATION="${LOADTEST_DURATION:-10s}"
CONNECTIONS="${LOADTEST_CONNECTIONS:-64}"
RAMP="${LOADTEST_RAMP:-1 16 64 256 1024}"
PORT="${LOADTEST_PORT:-18080}"
BASE="http://127.0.0.1:${PORT}"
PASSWORD="correct horse battery staple"

for tool in cargo curl python3 oha; do
  command -v "$tool" >/dev/null || { echo "loadtest: '$tool' not found" >&2; exit 1; }
done

cargo build --release --locked -q -p autoxide
BINARY="$PWD/target/release/autoxide"
WORK="$(mktemp -d)"
SERVER=""
cleanup() {
  [ -n "$SERVER" ] && kill "$SERVER" 2>/dev/null && wait "$SERVER" 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT
cd "$WORK"

export DATABASE_URL="${LOADTEST_DATABASE_URL:-sqlite://data/autoxide.sqlite}"
export AUTOXIDE_HTTP_ADDRESS="127.0.0.1:${PORT}"
# Đo máy chủ, không đo bộ giới hạn request: tắt rate limiting (mặc định 600 request mỗi
# phút mỗi địa chỉ — một máy tạo tải sẽ bị chặn ngay).
export AUTOXIDE_HTTP_RATE_LIMIT_PER_MINUTE=0
export AUTOXIDE_LOG_LEVEL=warn
export AUTOXIDE_AUTH_TOKEN_SECRET="$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')"
export AUTOXIDE_AUTH_ACCESS_TOKEN_SECONDS=86400
export AUTOXIDE_FILES_MAX_BYTES=1048576

"$BINARY" migrate up >/dev/null
printf '%s' "$PASSWORD" | "$BINARY" user create --email admin@example.com --name Admin \
  --admin --password-stdin >/dev/null
"$BINARY" serve >serve.log 2>&1 &
SERVER=$!
for _ in $(seq 1 100); do curl -sf "$BASE/health/ready" >/dev/null && break; sleep 0.1; done

json() { python3 -c "import sys,json; print(json.load(sys.stdin)$1)"; }
post() { curl -sf "$BASE$1" -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' -d "$2"; }

TOKEN="$(curl -sf "$BASE/api/v1/auth/login" -H 'content-type: application/json' \
  -d "{\"email\":\"admin@example.com\",\"password\":\"$PASSWORD\"}" | json '["data"]["access_token"]')"
API_KEY="$(post /api/v1/auth/api-keys "{\"name\":\"loadtest\",\"permissions\":[\"auth.users.read\"],\"current_password\":\"$PASSWORD\"}" \
  | json '["data"]["key"]')"
# Dữ liệu cho list: 120 người dùng (mỗi người tạo một event → audit log có dữ liệu).
for i in $(seq 1 120); do
  post /api/v1/auth/users "{\"email\":\"user$i@example.com\",\"name\":\"User $i\"}" >/dev/null
done
BOB="$(post /api/v1/auth/users '{"email":"bob@example.com","name":"Bob"}' | json '["data"]["id"]')"
head -c 16384 /dev/urandom >file.bin
head -c 4096 /dev/urandom >small.bin
FILE="$(curl -sf "$BASE/api/v1/files?name=seed.bin" -H "authorization: Bearer $TOKEN" \
  -H 'content-type: application/octet-stream' --data-binary @file.bin | json '["data"]["id"]')"
sleep 2 # để bộ phát event ghi audit log

AUTH="authorization: Bearer $TOKEN"
MCP_BODY='{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"auth.whoami","arguments":{}}}'

# run <tên> <số kết nối> <tham số oha…>
run() {
  local name="$1" connections="$2"
  shift 2
  oha --no-tui -z "$DURATION" -c "$connections" --output-format json "$@" >result.json
  python3 - "$name" "$connections" <<'PY'
import json, sys
name, connections = sys.argv[1], sys.argv[2]
r = json.load(open("result.json"))
s, p = r["summary"], r["latencyPercentiles"]
codes = r.get("statusCodeDistribution", {})
# Request đang bay khi hết giờ bị oha cắt: không phải lỗi của máy chủ.
dist = {k: v for k, v in r.get("errorDistribution", {}).items() if k != "aborted due to deadline"}
errors = sum(dist.values())
ok = sum(v for k, v in codes.items() if k.startswith("2"))
total = sum(codes.values()) + errors
ms = lambda v: f"{(v or 0) * 1000:8.2f}"
bad = {k: v for k, v in codes.items() if not k.startswith("2")}
note = (f"  non-2xx {bad}" if bad else "") + (f"  errors {dist}" if errors else "")
print(f"| {name:<34} | {connections:>5} | {s['requestsPerSec']:>9.0f} | {ms(p.get('p50'))} | {ms(p.get('p95'))} | {ms(p.get('p99'))} | {100 * ok / max(total, 1):6.2f}% |{note}")
PY
}

backend="SQLite"; [[ "$DATABASE_URL" == postgres* ]] && backend="PostgreSQL"

soak() {
  local duration="$1" pids=() samples=soak.csv
  echo "## Soak $backend — $duration, $(nproc) CPU"
  # Hỗn hợp chạy song song. Ghi có giới hạn tốc độ (-q) để đĩa không đầy sau vài phút.
  oha --no-tui -z "$duration" -c 16 --output-format json -H "$AUTH" "$BASE/api/v1/auth/me" >soak-read.json & pids+=($!)
  oha --no-tui -z "$duration" -c 8 --output-format json -H "$AUTH" "$BASE/api/v1/auth/users?limit=50" >soak-list.json & pids+=($!)
  oha --no-tui -z "$duration" -c 8 --output-format json -m POST -H "$AUTH" -H 'accept: application/json, text/event-stream' -T application/json -d "$MCP_BODY" "$BASE/mcp" >soak-mcp.json & pids+=($!)
  oha --no-tui -z "$duration" -c 8 --output-format json -H "$AUTH" "$BASE/api/v1/files/$FILE/content" >soak-download.json & pids+=($!)
  oha --no-tui -z "$duration" -c 8 -q 200 --output-format json -m PATCH -H "$AUTH" -T application/json -d '{"name":"Robert"}' "$BASE/api/v1/auth/users/$BOB" >soak-write.json & pids+=($!)
  oha --no-tui -z "$duration" -c 4 -q 20 --output-format json -m POST -H "$AUTH" -T application/octet-stream -D small.bin "$BASE/api/v1/files?name=soak.bin" >soak-upload.json & pids+=($!)
  echo "seconds,rss_mib,open_fds,db_connections" >"$samples"
  local start=$SECONDS
  while kill -0 "${pids[0]}" 2>/dev/null; do
    local rss fds db=""
    rss=$(awk '/VmRSS/ {printf "%.1f", $2 / 1024}' "/proc/$SERVER/status")
    fds=$(ls "/proc/$SERVER/fd" | wc -l)
    if [[ "$DATABASE_URL" == postgres* ]] && command -v psql >/dev/null; then
      db=$(psql "$DATABASE_URL" -tAc "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database()" 2>/dev/null || true)
    fi
    echo "$((SECONDS - start)),$rss,$fds,$db" >>"$samples"
    sleep 30
  done
  wait "${pids[@]}"
  python3 - "$samples" <<'PY'
import csv, glob, json, sys
rows = list(csv.DictReader(open(sys.argv[1])))
print("\n| Giây | RSS MiB | File mở | Kết nối DB |\n| ---: | ---: | ---: | ---: |")
for r in rows:
    print(f"| {r['seconds']} | {r['rss_mib']} | {r['open_fds']} | {r['db_connections'] or '-'} |")
total = ok = 0
print("\n| Luồng | Request | Request/s | p99 ms | Thành công |\n| --- | ---: | ---: | ---: | ---: |")
for path in sorted(glob.glob("soak-*.json")):
    r = json.load(open(path))
    codes = r.get("statusCodeDistribution", {})
    errors = {k: v for k, v in r.get("errorDistribution", {}).items() if k != "aborted due to deadline"}
    n = sum(codes.values()) + sum(errors.values())
    good = sum(v for k, v in codes.items() if k.startswith("2"))
    total += n; ok += good
    print(f"| {path[5:-5]} | {n} | {r['summary']['requestsPerSec']:.0f} | {r['latencyPercentiles']['p99'] * 1000:.1f} | {100 * good / max(n, 1):.3f}% |")
# Bỏ 2 phút đầu (bộ nhớ đệm, pool đang ấm lên); so nửa sau với nửa đầu của phần còn lại.
steady = [r for r in rows if int(r["seconds"]) >= 120] or rows
half = len(steady) // 2 or 1
first = max(float(r["rss_mib"]) for r in steady[:half])
last = max(float(r["rss_mib"]) for r in steady[half:]) if steady[half:] else first
growth = 100 * (last - first) / first
# File mở cũng so đỉnh nửa sau với đỉnh nửa đầu: số kết nối dao động theo tải là bình
# thường, chỉ tăng dần mãi mới là rò.
fds = [int(r["open_fds"]) for r in steady]
fds_first = max(fds[:half])
fds_last = max(fds[half:]) if fds[half:] else fds_first
print(f"\nTổng {total} request, thành công {100 * ok / max(total, 1):.3f}%. RSS đỉnh nửa đầu {first:.1f} MiB, nửa sau {last:.1f} MiB ({growth:+.1f}%). File mở đỉnh nửa đầu {fds_first}, nửa sau {fds_last}.")
print("Kết luận:", "không thấy rò rỉ" if growth < 10 and fds_last - fds_first < 20 else "CẦN XEM: bộ nhớ hoặc file mở tăng dần")
PY
}

if [ -n "${LOADTEST_SOAK:-}" ]; then
  soak "$LOADTEST_SOAK"
  curl -sf "$BASE/health/ready" >/dev/null && echo "Máy chủ vẫn sẵn sàng sau tải."
  echo "Dòng ERROR trong log máy chủ: $(grep -c 'ERROR' serve.log || true)"
  exit 0
fi
echo "## $backend — $(nproc) CPU, ${DURATION} mỗi bài"
echo
echo "| Bài đo                             | Kết nối |  Request/s |   p50 ms |   p95 ms |   p99 ms |  Thành công |"
echo "| ---------------------------------- | ----: | --------: | -------: | -------: | -------: | ------: |"
run "GET /health/ready"                   "$CONNECTIONS" "$BASE/health/ready"
run "GET /auth/me (access token)"         "$CONNECTIONS" -H "$AUTH" "$BASE/api/v1/auth/me"
run "GET /auth/me (API key)"              "$CONNECTIONS" -H "authorization: Bearer $API_KEY" "$BASE/api/v1/auth/me"
run "GET /auth/users?limit=50"            "$CONNECTIONS" -H "$AUTH" "$BASE/api/v1/auth/users?limit=50"
run "GET /audit?limit=50"                 "$CONNECTIONS" -H "$AUTH" "$BASE/api/v1/audit?limit=50"
run "GET /files/{id}/content (16 KiB)"    "$CONNECTIONS" -H "$AUTH" "$BASE/api/v1/files/$FILE/content"
run "MCP tools/call auth.whoami"          "$CONNECTIONS" -m POST -H "$AUTH" -H 'accept: application/json, text/event-stream' -T application/json -d "$MCP_BODY" "$BASE/mcp"
run "PATCH /auth/users/{id} (ghi + event)" "$CONNECTIONS" -m PATCH -H "$AUTH" -T application/json -d '{"name":"Robert"}' "$BASE/api/v1/auth/users/$BOB"
run "POST /files (4 KiB, ghi)"            "$CONNECTIONS" -m POST -H "$AUTH" -T application/octet-stream -D small.bin "$BASE/api/v1/files?name=load.bin"
run "POST /auth/login (Argon2id)"         16 -m POST -T application/json -d "{\"email\":\"admin@example.com\",\"password\":\"$PASSWORD\"}" "$BASE/api/v1/auth/login"

echo
echo "### Tăng dần số kết nối: đọc (GET /auth/me) và ghi (PATCH /auth/users/{id})"
echo
echo "| Bài đo                             | Kết nối |  Request/s |   p50 ms |   p95 ms |   p99 ms |  Thành công |"
echo "| ---------------------------------- | ----: | --------: | -------: | -------: | -------: | ------: |"
for connections in $RAMP; do
  run "đọc" "$connections" -H "$AUTH" "$BASE/api/v1/auth/me"
done
for connections in $RAMP; do
  run "ghi" "$connections" -m PATCH -H "$AUTH" -T application/json -d '{"name":"Robert"}' "$BASE/api/v1/auth/users/$BOB"
done

# Máy chủ còn khoẻ sau tải, và log không có lỗi.
curl -sf "$BASE/health/ready" >/dev/null && echo && echo "Máy chủ vẫn sẵn sàng sau tải."
errors="$(grep -c 'ERROR' serve.log || true)"
echo "Dòng ERROR trong log máy chủ: $errors"
grep 'ERROR' serve.log | sort | uniq -c | sort -rn | head -5 || true
