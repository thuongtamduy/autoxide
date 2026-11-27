#!/usr/bin/env bash
# Đo hiệu năng và chịu tải qua API THẬT (D-076): binary release, `autoxide serve` thật,
# TCP thật, đăng nhập thật (access token, API key), database thật. Xem docs/BENCHMARKS.md.
#
#   scripts/loadtest.sh                                   # SQLite (file tạm)
#   LOADTEST_DATABASE_URL=postgres://u:p@host:5432/db scripts/loadtest.sh   # database PHẢI trống
#
# Cần: cargo, curl, python3, oha (`cargo install oha --locked`).
# Biến (không bắt buộc): LOADTEST_DURATION (mặc định 10s), LOADTEST_CONNECTIONS (64),
# LOADTEST_RAMP ("1 16 64 256 1024"), LOADTEST_PORT (18080), LOADTEST_BINARY (binary có sẵn,
# không build).
#
# Chế độ chạy dài (soak): LOADTEST_SOAK=15m chạy hỗn hợp đọc / ghi / file / MCP liên tục
# trong thời gian đó, đo bộ nhớ (RSS), số file đang mở của máy chủ và số kết nối
# PostgreSQL mỗi 30 giây, rồi kết luận có rò rỉ hay không — thay cho các bài đo ngắn.
#
# Chế độ nhiều khách (bước 33): LOADTEST_TENANCY=multi chạy máy chủ ở AUTOXIDE_TENANCY_MODE=multi
# với 100 khách trên database chính và 10 khách có database SQLite riêng, rồi thêm bài đo đọc /
# list / ghi của người dùng một khách chung và một khách riêng (chọn khách bằng credential).
# LOADTEST_TENANTS (100) và LOADTEST_DEDICATED (10) đổi số khách.
# Thêm LOADTEST_PLAN=1 (bước 35): khách s1 có thuê bao gói bật mọi module, khách s2 không — đo cả
# GET /forms (module bị gói chi phối) của hai khách để so lớp chặn theo gói.
#
# Rate limit theo API key (bước 40 P6): LOADTEST_RATE_LIMIT_STORE=memory|database bật giới hạn theo
# API key (1 000 000 request/phút để không có 429, burst LOADTEST_RATE_LIMIT_BURST, mặc định 30 như
# cấu hình mặc định — lô của `database` là burst / 4) để đo chi phí đếm mỗi request.
# LOADTEST_ONLY=api_key chỉ chạy bài GET /auth/me bằng API key (so nhanh trước / sau).
#
# Bộ dữ liệu cỡ thật (bước 39 B2): LOADTEST_DATASET=small|full sinh dữ liệu bằng `scripts/bench-seed.sh`
# (binary build với `--features bench-seed`; bài hai ngôn ngữ `en,vi`) rồi thêm các bài đọc công khai
# của website: bài theo đường dẫn, danh sách, theo danh mục, sitemap, feed, tìm, ảnh và bản nhỏ,
# bình luận. LOADTEST_ONLY=public chỉ chạy các bài đó.
#
# Đo hằng tuần trên CI (71-G9, `.github/workflows/load.yml`): LOADTEST_ONLY=hot chỉ chạy các đường nóng (đăng nhập,
# list có pagination, tạo bản ghi, MCP tools/call, đọc có xác thực) và LOADTEST_JSON_OUT=<file> ghi thêm mỗi bài
# một dòng JSON (name, connections, rps, p50/p95/p99 tính bằng mili giây, tỉ lệ 2xx) cho `scripts/load-compare.py`
# so với `docs/quality/load-baseline.json`. Số liệu CI là số của runner chung, nhiễu: dùng ngưỡng rộng, không so tuyệt đối.
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

# LOADTEST_BINARY: đo một binary có sẵn (vd bản trước một thay đổi, để so trước / sau trên
# cùng máy) thay vì build mới.
if [ -n "${LOADTEST_BINARY:-}" ]; then
  BINARY="$LOADTEST_BINARY"
else
  if [ -n "${LOADTEST_DATASET:-}" ]; then
    cargo build --release --locked -q -p autoxide --features bench-seed
  else
    cargo build --release --locked -q -p autoxide
  fi
  BINARY="$PWD/target/release/autoxide"
fi
REPO="$PWD"
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
# Cả giới hạn theo API key (D-109): không thì bài đo API key chỉ đo câu trả lời 429.
export AUTOXIDE_HTTP_RATE_LIMIT_API_KEY_PER_MINUTE=0
if [ -n "${LOADTEST_RATE_LIMIT_STORE:-}" ]; then
  export AUTOXIDE_HTTP_RATE_LIMIT_API_KEY_PER_MINUTE=1000000
  export AUTOXIDE_HTTP_RATE_LIMIT_API_KEY_BURST="${LOADTEST_RATE_LIMIT_BURST:-30}"
  export AUTOXIDE_HTTP_RATE_LIMIT_STORE="$LOADTEST_RATE_LIMIT_STORE"
fi
export AUTOXIDE_LOG_LEVEL=warn
export AUTOXIDE_AUTH_TOKEN_SECRET="$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')"
export AUTOXIDE_AUTH_ACCESS_TOKEN_SECONDS=86400
export AUTOXIDE_FILES_MAX_BYTES=1048576
if [ -n "${LOADTEST_DATASET:-}" ]; then
  export AUTOXIDE_CONTENT_LOCALES=en,vi
  # Tìm công khai tắt mặc định; không có mô hình embedding thì tìm theo từ.
  export AUTOXIDE_CONTENT_PUBLIC_SEMANTIC_SEARCH=true
fi
TENANCY="${LOADTEST_TENANCY:-single}"
if [ "$TENANCY" = multi ]; then
  export AUTOXIDE_TENANCY_MODE=multi
  export AUTOXIDE_SECRET_KEY="$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')"
fi

"$BINARY" migrate up >/dev/null
printf '%s' "$PASSWORD" | "$BINARY" user create --email admin@example.com --name Admin \
  --admin --password-stdin >/dev/null
if [ "$TENANCY" = multi ]; then
  # Khách chung: dữ liệu nằm cùng database chính. Khách riêng: mỗi khách một file SQLite.
  # Lệnh CLI in cảnh báo cấu hình (chưa đặt SMTP…) ra stderr: bỏ đi cho bảng kết quả gọn.
  for i in $(seq 1 "${LOADTEST_TENANTS:-100}"); do
    "$BINARY" tenant create --slug "s$i" --name "Shared $i" >/dev/null 2>&1
  done
  mkdir -p dedicated
  for i in $(seq 1 "${LOADTEST_DEDICATED:-10}"); do
    printf '%s' "sqlite://$PWD/dedicated/d$i.sqlite?mode=rwc" \
      | "$BINARY" placement add "d$i" --kind dedicated --url - >/dev/null 2>&1
  done
  "$BINARY" migrate up --all >/dev/null 2>&1
  for i in $(seq 1 "${LOADTEST_DEDICATED:-10}"); do
    "$BINARY" tenant create --slug "d$i" --name "Dedicated $i" --placement "d$i" >/dev/null 2>&1
  done
  for slug in s1 s2 d1; do
    printf '%s' "$PASSWORD" | "$BINARY" user create --tenant "$slug" --email "admin@$slug.example" \
      --name Admin --admin --password-stdin >/dev/null 2>&1
  done
fi
if [ -n "${LOADTEST_DATASET:-}" ]; then
  echo "Sinh bộ dữ liệu '$LOADTEST_DATASET'…"
  BENCH_BINARY="$BINARY" "$REPO/scripts/bench-seed.sh" "$LOADTEST_DATASET"
fi
"$BINARY" serve >serve.log 2>&1 &
SERVER=$!
# Chờ sẵn sàng tới 2 phút: trên bộ dữ liệu `full`, SQLite lập thống kê lúc mở (vài giây).
for _ in $(seq 1 1200); do curl -sf "$BASE/health/ready" >/dev/null && break; sleep 0.1; done
curl -sf "$BASE/health/ready" >/dev/null || { echo "loadtest: server not ready; log:" >&2; tail -20 serve.log >&2; exit 1; }

json() { python3 -c "import sys,json; print(json.load(sys.stdin)$1)"; }
post() { curl -sf "$BASE$1" -H "authorization: Bearer $TOKEN" -H 'content-type: application/json' -d "$2"; }

# Chế độ multi: trước khi đăng nhập, request chọn khách bằng header (khách nền tảng tên `default`).
TOKEN="$(curl -sf "$BASE/api/v1/auth/login" -H 'content-type: application/json' -H 'autoxide-tenant: default' \
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
import json, os, sys
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
out = os.environ.get("LOADTEST_JSON_OUT")
if out:
    with open(out, "a") as handle:
        handle.write(json.dumps({
            "name": name, "connections": int(connections), "rps": s["requestsPerSec"],
            "p50_ms": (p.get("p50") or 0) * 1000, "p95_ms": (p.get("p95") or 0) * 1000,
            "p99_ms": (p.get("p99") or 0) * 1000, "ok_ratio": ok / max(total, 1),
        }) + "\n")
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
echo "## $backend ($TENANCY) — $(nproc) CPU, ${DURATION} mỗi bài"
echo
echo "| Bài đo                             | Kết nối |  Request/s |   p50 ms |   p95 ms |   p99 ms |  Thành công |"
echo "| ---------------------------------- | ----: | --------: | -------: | -------: | -------: | ------: |"
# Đọc công khai của website trên bộ dữ liệu cỡ thật (bước 39 B2): không đăng nhập, như trình duyệt
# hay CDN gọi. Lấy địa chỉ thật từ chính API (bài thứ 10 của trang đầu, danh mục đầu, ảnh của bài).
public_reads() {
  local pub="$BASE/api/v1/content/public" first path term slug image thumb entry thread
  first="$(curl -sf "$pub/posts?locale=en&limit=20")"
  path="$(printf '%s' "$first" | json '["data"][9]["path"]')"
  entry="$(printf '%s' "$first" | json '["data"][9]["id"]')"
  image="$(printf '%s' "$first" | json '["data"][9]["featured_media"]["url"]')"
  thumb="$(printf '%s' "$first" | json '["data"][9]["featured_media"]["variants"][0]["webp_url"]')"
  term="$(curl -sf "$pub/terms?locale=en&taxonomy=category&limit=1")"
  slug="$(printf '%s' "$term" | json '["data"][0]["slug"]')"
  term="$(printf '%s' "$term" | json '["data"][0]["id"]')"
  thread="$(curl -sf "$BASE/api/v1/comments/public/threads?target_type=content.entry&target_id=$entry&limit=1" | json '["data"][0]["id"]')"
  run "public resolve (một bài)"            "$CONNECTIONS" "$pub/resolve?locale=en&path=$path"
  run "public posts (trang đầu, 20)"        "$CONNECTIONS" "$pub/posts?locale=en&limit=20"
  run "public posts theo danh mục (20)"     "$CONNECTIONS" "$pub/posts?locale=en&limit=20&term_id=$term"
  run "public terms/category/{slug}"        "$CONNECTIONS" "$pub/terms/category/$slug?locale=en"
  run "public sitemap (trang đầu)"          "$CONNECTIONS" "$pub/sitemap?locale=en"
  run "public feed"                         "$CONNECTIONS" "$pub/feed?locale=en"
  run "public search (từ khoá)"             "$CONNECTIONS" "$pub/search?locale=en&q=coffee+garden"
  run "media content (ảnh gốc)"             "$CONNECTIONS" "$BASE$image"
  run "media content (bản nhỏ WebP)"        "$CONNECTIONS" "$BASE$thumb"
  run "comments public threads (20)"        "$CONNECTIONS" "$BASE/api/v1/comments/public/threads?target_type=content.entry&target_id=$entry&limit=20"
  run "comments public comments/{id}"       "$CONNECTIONS" "$BASE/api/v1/comments/public/comments/$thread"
}

if [ -n "${LOADTEST_DATASET:-}" ] && [ "${LOADTEST_ONLY:-}" = public ]; then
  public_reads
  exit 0
fi
if [ "${LOADTEST_ONLY:-}" = hot ]; then
  # Đăng nhập chậm có chủ ý (Argon2id): 8 kết nối, tối đa 20 lần/giây — chạy hết công suất thì máy chủ trả 429 để tự bảo vệ,
  # và p95 của một cuộc đăng nhập bình thường mới là thứ cần theo dõi.
  run "login"          8 -q 20 -m POST -H 'autoxide-tenant: default' -T application/json -d "{\"email\":\"admin@example.com\",\"password\":\"$PASSWORD\"}" "$BASE/api/v1/auth/login"
  run "read (auth/me)" "$CONNECTIONS" -H "$AUTH" "$BASE/api/v1/auth/me"
  run "list (page of 50)" "$CONNECTIONS" -H "$AUTH" "$BASE/api/v1/auth/users?limit=50"
  run "create (file, 4 KiB)" "$CONNECTIONS" -m POST -H "$AUTH" -T application/octet-stream -D small.bin "$BASE/api/v1/files?name=load.bin"
  run "mcp tools/call" "$CONNECTIONS" -m POST -H "$AUTH" -H 'accept: application/json, text/event-stream' -T application/json -d "$MCP_BODY" "$BASE/mcp"
  curl -sf "$BASE/health/ready" >/dev/null || { echo "Máy chủ không còn sẵn sàng sau tải" >&2; exit 1; }
  exit 0
fi
if [ "${LOADTEST_ONLY:-}" = api_key ]; then
  run "GET /auth/me (API key, ${LOADTEST_RATE_LIMIT_STORE:-off})" "$CONNECTIONS" -H "authorization: Bearer $API_KEY" "$BASE/api/v1/auth/me"
  run "MCP auth.whoami (API key, ${LOADTEST_RATE_LIMIT_STORE:-off})" "$CONNECTIONS" -m POST -H "authorization: Bearer $API_KEY" -H 'accept: application/json, text/event-stream' -T application/json -d "$MCP_BODY" "$BASE/mcp"
  exit 0
fi
run "GET /health/ready"                   "$CONNECTIONS" "$BASE/health/ready"
run "GET /auth/me (access token)"         "$CONNECTIONS" -H "$AUTH" "$BASE/api/v1/auth/me"
run "GET /auth/me (API key)"              "$CONNECTIONS" -H "authorization: Bearer $API_KEY" "$BASE/api/v1/auth/me"
run "GET /auth/users?limit=50"            "$CONNECTIONS" -H "$AUTH" "$BASE/api/v1/auth/users?limit=50"
run "GET /audit?limit=50"                 "$CONNECTIONS" -H "$AUTH" "$BASE/api/v1/audit?limit=50"
run "GET /files/{id}/content (16 KiB)"    "$CONNECTIONS" -H "$AUTH" "$BASE/api/v1/files/$FILE/content"
run "MCP tools/call auth.whoami"          "$CONNECTIONS" -m POST -H "$AUTH" -H 'accept: application/json, text/event-stream' -T application/json -d "$MCP_BODY" "$BASE/mcp"
run "PATCH /auth/users/{id} (ghi + event)" "$CONNECTIONS" -m PATCH -H "$AUTH" -T application/json -d '{"name":"Robert"}' "$BASE/api/v1/auth/users/$BOB"
run "POST /files (4 KiB, ghi)"            "$CONNECTIONS" -m POST -H "$AUTH" -T application/octet-stream -D small.bin "$BASE/api/v1/files?name=load.bin"
run "POST /auth/login (Argon2id)"         16 -m POST -H 'autoxide-tenant: default' -T application/json -d "{\"email\":\"admin@example.com\",\"password\":\"$PASSWORD\"}" "$BASE/api/v1/auth/login"

if [ -n "${LOADTEST_DATASET:-}" ]; then public_reads; fi

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

if [ "$TENANCY" = multi ]; then
  echo
  echo "### Người dùng của một khách (máy chủ có $((${LOADTEST_TENANTS:-100} + ${LOADTEST_DEDICATED:-10} + 1)) khách)"
  echo
  echo "| Bài đo                             | Kết nối |  Request/s |   p50 ms |   p95 ms |   p99 ms |  Thành công |"
  echo "| ---------------------------------- | ----: | --------: | -------: | -------: | -------: | ------: |"
  # `LOADTEST_PLAN=1` (bước 35): khách s1 có thuê bao gói bật mọi module (không giới hạn), s2 không
  # có — so lớp chặn theo gói của hai đường. Bản chưa có module `plans` thì bỏ qua bước gán gói.
  slugs="s1 d1"
  if [ -n "${LOADTEST_PLAN:-}" ]; then
    slugs="s1 s2 d1"
    plan="$(post /api/v1/plans/plans '{"code":"loadtest","name":"Loadtest","status":"active","modules":["ai","media","content","forms","newsletter","comments","menus"],"features":["auth.api_keys","content.ai_suggestions"],"limits":{"organizations.units":null,"auth.members":null,"files.storage_bytes":null,"kernel.api_requests":null}}' \
      | json '["data"]["id"]' 2>/dev/null || true)"
    s1="$(curl -sf "$BASE/api/v1/kernel/tenants/s1" -H "$AUTH" | json '["data"]["id"]' 2>/dev/null || true)"
    if [ -n "$plan" ] && [ -n "$s1" ]; then
      curl -sf -X PUT "$BASE/api/v1/plans/tenants/$s1/subscription" -H "$AUTH" -H 'content-type: application/json' \
        -d "{\"plan_id\":\"$plan\",\"trial\":false}" >/dev/null
    fi
  fi
  for slug in $slugs; do
    kind="chung"; [ "$slug" = d1 ] && kind="riêng"
    [ -n "${LOADTEST_PLAN:-}" ] && [ "$slug" = s1 ] && kind="chung, có gói"
    [ "$slug" = s2 ] && kind="chung, không gói"
    # Đăng nhập chọn khách bằng header; request sau đó lấy khách từ chính token.
    tenant_token="$(curl -sf "$BASE/api/v1/auth/login" -H 'content-type: application/json' \
      -H "autoxide-tenant: $slug" \
      -d "{\"email\":\"admin@$slug.example\",\"password\":\"$PASSWORD\"}" | json '["data"]["access_token"]')"
    for i in $(seq 1 20); do
      curl -sf "$BASE/api/v1/auth/users" -H "authorization: Bearer $tenant_token" \
        -H 'content-type: application/json' -d "{\"email\":\"user$i@$slug.example\",\"name\":\"User $i\"}" >/dev/null
    done
    member="$(curl -sf "$BASE/api/v1/auth/users" -H "authorization: Bearer $tenant_token" \
      -H 'content-type: application/json' -d "{\"email\":\"bob@$slug.example\",\"name\":\"Bob\"}" \
      | json '["data"]["id"]')"
    run "khách $kind: GET /auth/me"        "$CONNECTIONS" -H "authorization: Bearer $tenant_token" "$BASE/api/v1/auth/me"
    run "khách $kind: GET /auth/users"     "$CONNECTIONS" -H "authorization: Bearer $tenant_token" "$BASE/api/v1/auth/users?limit=50"
    run "khách $kind: PATCH /auth/users"   "$CONNECTIONS" -m PATCH -H "authorization: Bearer $tenant_token" -T application/json -d '{"name":"Robert"}' "$BASE/api/v1/auth/users/$member"
    if [ -n "${LOADTEST_PLAN:-}" ]; then
      run "khách $kind: GET /forms"        "$CONNECTIONS" -H "authorization: Bearer $tenant_token" "$BASE/api/v1/forms"
    fi
  done
fi

# Máy chủ còn khoẻ sau tải, và log không có lỗi.
curl -sf "$BASE/health/ready" >/dev/null && echo && echo "Máy chủ vẫn sẵn sàng sau tải."
errors="$(grep -c 'ERROR' serve.log || true)"
echo "Dòng ERROR trong log máy chủ: $errors"
grep 'ERROR' serve.log | sort | uniq -c | sort -rn | head -5 || true
