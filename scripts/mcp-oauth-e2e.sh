#!/usr/bin/env bash
# Kiểm đăng nhập OAuth cho MCP đầu-cuối bằng client MCP CHÍNH THỨC (SDK TypeScript `@modelcontextprotocol/sdk`)
# tới `autoxide serve` thật (D-20261126-mcp-oauth, bước 71 G4): khám phá qua `/.well-known`, đăng ký client (đăng ký
# động và tài liệu metadata), PKCE, consent, đổi mã, gọi tool, refresh, thu hồi. Chạy trên SQLite, và thêm Postgres
# khi đặt MCP_OAUTH_POSTGRES_URL (database PHẢI trống). CI chạy ở job `mcp-oauth` của .github/workflows/ci.yml.
#
#   cargo build -p autoxide && scripts/mcp-oauth-e2e.sh
#   MCP_OAUTH_POSTGRES_URL=postgres://u:p@127.0.0.1:5432/trong scripts/mcp-oauth-e2e.sh
#
# Biến: MCP_OAUTH_BINARY (mặc định target/debug/autoxide), MCP_OAUTH_PORT (mặc định 18431).
# Cần: node (22+), npm, curl.
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
BINARY="${MCP_OAUTH_BINARY:-$REPO/target/debug/autoxide}"
PORT="${MCP_OAUTH_PORT:-18431}"
BASE="http://127.0.0.1:$PORT"
[ -x "$BINARY" ] || { echo "mcp-oauth: không thấy binary $BINARY (cargo build -p autoxide)" >&2; exit 1; }

(cd "$REPO/scripts/mcp-oauth" && npm ci --ignore-scripts --no-audit --no-fund --loglevel=error)

WORK="$(mktemp -d)"
SERVER=""
cleanup() {
  [ -n "$SERVER" ] && kill "$SERVER" 2>/dev/null && wait "$SERVER" 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT

# Mật khẩu và secret sinh mỗi lần chạy: không viết cứng secret nào.
PASSWORD="$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')"

run() {
  local database_url="$1" label="$2"
  echo "mcp-oauth: $label"
  export DATABASE_URL="$database_url"
  export AUTOXIDE_HTTP_ADDRESS="127.0.0.1:$PORT"
  export AUTOXIDE_HTTP_PUBLIC_URL="$BASE"
  # Trang consent của giao diện: client.mjs đóng vai trang này, không ai mở địa chỉ thật.
  export AUTOXIDE_AUTH_OAUTH_CONSENT_URL="https://interface.example/connect?request={token}"
  # Tài liệu metadata của client do client.mjs phục vụ ở 127.0.0.1 qua http: chỉ cho bài kiểm này.
  export AUTOXIDE_AUTH_OAUTH_ALLOW_PRIVATE_NETWORKS=true
  export AUTOXIDE_HTTP_RATE_LIMIT_PER_MINUTE=0
  export AUTOXIDE_LOG_LEVEL=warn
  export AUTOXIDE_AUTH_TOKEN_SECRET
  AUTOXIDE_AUTH_TOKEN_SECRET="$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')"
  (cd "$WORK" && "$BINARY" migrate up >/dev/null)
  printf '%s' "$PASSWORD" | (cd "$WORK" && "$BINARY" user create --email admin@example.com --name Admin \
    --admin --password-stdin >/dev/null 2>&1)
  (cd "$WORK" && exec "$BINARY" serve >"$WORK/serve.log" 2>&1) &
  SERVER=$!
  for _ in $(seq 1 600); do curl -sf "$BASE/health/ready" >/dev/null && break; sleep 0.1; done
  curl -sf "$BASE/health/ready" >/dev/null || { echo "mcp-oauth: máy chủ không sẵn sàng" >&2; tail -30 "$WORK/serve.log" >&2; exit 1; }
  local registration
  for registration in dynamic metadata-document; do
    if ! AUTOXIDE_URL="$BASE" ADMIN_EMAIL=admin@example.com ADMIN_PASSWORD="$PASSWORD" REGISTRATION="$registration" \
      node "$REPO/scripts/mcp-oauth/client.mjs"; then
      echo "mcp-oauth: lỗi ($label, $registration); log máy chủ:" >&2
      tail -40 "$WORK/serve.log" >&2
      exit 1
    fi
  done
  kill "$SERVER" && wait "$SERVER" 2>/dev/null || true
  SERVER=""
}

mkdir -p "$WORK/data"
run "sqlite://$WORK/data/autoxide.sqlite?mode=rwc" "SQLite"
if [ -n "${MCP_OAUTH_POSTGRES_URL:-}" ]; then
  run "$MCP_OAUTH_POSTGRES_URL" "PostgreSQL"
fi
echo "mcp-oauth: xong"
