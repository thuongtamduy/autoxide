#!/usr/bin/env bash
# Gate của Autoxide: định nghĩa "xong". Hai mức (D-173):
#
#   ./gate.sh --quick   gate nhanh, trước mỗi commit lúc đang làm một bước: như dưới đây
#                       nhưng chỉ chạy test của crate bị sửa (kể từ lần gate nhanh xanh gần
#                       nhất, chưa có thì so với origin/main) và crate phụ thuộc vào chúng, chỉ
#                       trên SQLite, không cần Docker.
#   ./gate.sh           gate đầy đủ (cả SQLite và Postgres): bắt buộc trước khi phát hành (D-300).
#
# CI trên GitHub (.github/workflows/ci.yml) chạy gate đầy đủ trên mỗi pull request và mỗi lần
# push lên main (D-175, D-197); gate trên máy vẫn là định nghĩa "xong", CI là lưới thứ hai.
#
# Cả hai chạy ở mức ưu tiên CPU thấp (`renice`) để máy vẫn dùng được trong lúc build.
#
# Theo thứ tự (dừng ở bước đỏ đầu tiên):
#   1. định dạng            cargo fmt --check
#   2. lint                 cargo clippy, cảnh báo tính là lỗi
#   3. ranh giới workspace  cargo xtask check
#   4. hợp đồng API         cargo xtask api-diff: không phá vỡ /api/v1 so với tag phát hành gần nhất;
#                           cargo xtask sdk --check: SDK TypeScript khớp docs/openapi.json
#   5. thư viện             cargo deny check (license, lỗ hổng, nguồn)
#   6. secret               gitleaks: toàn bộ lịch sử git và thay đổi chưa commit
#   7. test trên SQLite
#   8. test trên Postgres   (tự dựng một container Docker dùng xong bỏ)
#   Bước 7 và 8 chạy các file test song song (`cargo xtask test-all`, D-299); trên Postgres giới
#   hạn tổng số test chạy cùng lúc để không vượt số kết nối của máy chủ.
#   Bước 7 và 8 chạy thêm test lưu file trên S3: gate dựng một máy chủ S3 giả lập
#   (moto) trong Docker, dùng xong bỏ.
#
# Cần có: toolchain Rust (rustup tự cài đúng bản ghim), cargo-deny, Docker (gitleaks cài
# sẵn trên máy thì dùng nó, không thì chạy image Docker ghim phiên bản).
# Cần mạng ở lần chạy đầu (tải thư viện, image Postgres) và mỗi lần cargo-deny
# cập nhật danh sách lỗ hổng.
#
# Biến môi trường (đều không bắt buộc):
#   GATE_POSTGRES_URL    dùng Postgres có sẵn thay vì dựng container, dạng
#                        postgres://user:password@host:port/postgres — người dùng
#                        phải có quyền CREATE DATABASE. CI dùng biến này.
#   GATE_POSTGRES_IMAGE  image cho container (mặc định là phiên bản Postgres tối
#                        thiểu được hỗ trợ, xem STACK.md).
#   GATE_S3_ENDPOINT     dùng dịch vụ tương thích S3 có sẵn thay vì dựng container,
#                        dạng http://host:port — phải có sẵn bucket `autoxide-test` (hoặc
#                        TEST_S3_BUCKET) và nhận TEST_S3_ACCESS_KEY_ID /
#                        TEST_S3_SECRET_ACCESS_KEY (mặc định `test` / `test`).
#   GATE_S3_IMAGE        image cho container S3 giả lập (mặc định motoserver/moto).
#   GATE_GITLEAKS_IMAGE  image gitleaks khi máy không cài gitleaks.
#
# Gate đỏ thì sửa code. Không nới gate, không tắt lint, không bỏ qua test
# (CLAUDE.md, mục "Gate").

set -euo pipefail
cd "$(dirname "$0")"

QUICK=0
case "${1:-}" in
  "") ;;
  --quick) QUICK=1 ;;
  *)
    echo "usage: ./gate.sh [--quick]" >&2
    exit 2
    ;;
esac

# Ưu tiên CPU thấp cho cả script và mọi tiến trình con (cargo, rustc, test): việc khác trên máy
# được chạy trước, gate dùng phần CPU còn thừa. Không có `renice` thì chạy bình thường.
renice -n 10 -p $$ >/dev/null 2>&1 || true

POSTGRES_IMAGE="${GATE_POSTGRES_IMAGE:-postgres:16-alpine}"
S3_IMAGE="${GATE_S3_IMAGE:-motoserver/moto:5.2.5}"
GITLEAKS_IMAGE="${GATE_GITLEAKS_IMAGE:-zricethezav/gitleaks:v8.30.0}"
CONTAINER=""
S3_CONTAINER=""

step() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }
fail() {
  printf '\033[31mgate: %s\033[0m\n' "$*" >&2
  exit 1
}

cleanup() {
  local container
  for container in "$CONTAINER" "$S3_CONTAINER"; do
    if [ -n "$container" ]; then
      docker rm --force --volumes "$container" >/dev/null 2>&1 || true
    fi
  done
}
# Chạy cleanup dù gate xanh, đỏ hay bị Ctrl-C.
trap cleanup EXIT

require() {
  command -v "$1" >/dev/null 2>&1 || fail "'$1' not found. $2"
}

require cargo "Install Rust from https://rustup.rs"
require cargo-deny "Install it with: cargo install cargo-deny --locked"

# --- 1-5: kiểm tĩnh, nhanh, không cần database ---------------------------------

step "format (cargo fmt)"
cargo fmt --all --check

step "lint (cargo clippy)"
cargo clippy --workspace --all-targets --locked -- -D warnings

step "workspace boundaries (cargo xtask check)"
cargo xtask check

# So với tag phát hành gần nhất (tổ tiên của HEAD); cần lịch sử và tag (D-059, đặc tả 31 A1).
step "api contract (cargo xtask api-diff)"
cargo xtask api-diff

# Kiểu TypeScript của SDK sinh từ docs/openapi.json; không cần Node (D-243).
step "TypeScript SDK matches the contract (cargo xtask sdk --check)"
cargo xtask sdk --check

step "dependencies (cargo deny)"
# -D ...-not-detected / -not-encountered: ngoại lệ trong deny.toml không còn cần
# thì thành lỗi, để ngoại lệ tự hết hạn.
cargo deny check -D advisory-not-detected -D license-not-encountered

# --- 6: secret --------------------------------------------------------------------

gitleaks_run() {
  if command -v gitleaks >/dev/null 2>&1; then
    gitleaks "$@"
  else
    require docker "Install gitleaks (https://github.com/gitleaks/gitleaks) or Docker."
    docker info >/dev/null 2>&1 || fail "Docker is installed but not running (or not accessible to this user)."
    docker run --rm --volume "$PWD:/repo" --workdir /repo "$GITLEAKS_IMAGE" "$@"
  fi
}

step "secrets (gitleaks)"
# Giữ trong gate dù CI cũng quét (job `secrets` của ci.yml, D-108): gate bắt secret trên máy,
# trước khi nó rời máy. Quét toàn bộ lịch sử (secret đã commit rồi xoá vẫn nằm trong git), rồi
# thay đổi đã stage và chưa stage (bắt trước khi commit).
leaks=(--no-banner --redact --config .gitleaks.toml)
gitleaks_run git . "${leaks[@]}"
gitleaks_run git . "${leaks[@]}" --pre-commit --staged
gitleaks_run git . "${leaks[@]}" --pre-commit

# --- Gate nhanh: test của crate bị ảnh hưởng, chỉ SQLite ---------------------------

if [ "$QUICK" = 1 ]; then
  step "tests on SQLite, changed crates only (cargo xtask test-affected)"
  cargo xtask test-affected
  printf '\n\033[32mgate --quick: green — fmt, clippy, boundaries, api contract, dependencies, secrets, tests of changed crates on SQLite\033[0m\n'
  printf 'Not checked: PostgreSQL, S3, crates you did not touch. Run ./gate.sh before merging into main.\n'
  exit 0
fi

# --- S3 cho test lưu file (dùng ở bước 7 và 8) -------------------------------------

start_s3() {
  require docker "Install Docker, or point GATE_S3_ENDPOINT at an existing S3-compatible service."
  docker info >/dev/null 2>&1 || fail "Docker is installed but not running (or not accessible to this user)."
  S3_CONTAINER="autoxide-gate-s3-$$"
  docker run --detach --name "$S3_CONTAINER" --publish 127.0.0.1::5000 "$S3_IMAGE" >/dev/null
  local port
  port="$(docker port "$S3_CONTAINER" 5000/tcp | head -n 1 | sed 's/.*://')"
  S3_ENDPOINT="http://127.0.0.1:${port}"
  local attempt
  for attempt in $(seq 1 60); do
    # Máy chủ giả lập nhận request không ký: tạo bucket là đủ để biết nó đã sẵn sàng.
    if curl --silent --fail --request PUT "$S3_ENDPOINT/autoxide-test" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.5
  done
  docker logs "$S3_CONTAINER" >&2 || true
  fail "S3 container did not become ready within 30 seconds."
}

S3_ENDPOINT="${GATE_S3_ENDPOINT:-}"
if [ -z "$S3_ENDPOINT" ]; then
  require curl "Install curl."
  step "S3 for file storage tests ($S3_IMAGE in Docker)"
  start_s3
fi
export TEST_S3_ENDPOINT="$S3_ENDPOINT"

# --- 7: SQLite ------------------------------------------------------------------

step "tests on SQLite"
# `env -u`: bỏ biến nếu người chạy lỡ đặt sẵn, để bước này chắc chắn là SQLite.
env -u TEST_DATABASE_URL cargo xtask test-all --doc

# --- 8: Postgres ----------------------------------------------------------------

start_postgres() {
  require docker "Install Docker, or point GATE_POSTGRES_URL at an existing Postgres."
  docker info >/dev/null 2>&1 || fail "Docker is installed but not running (or not accessible to this user)."

  # Mật khẩu ngẫu nhiên cho mỗi lần chạy: không có secret nào nằm trong repo.
  local password
  password="$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')"
  CONTAINER="autoxide-gate-$$"

  # -p 127.0.0.1::5432  chỉ mở trên máy này, port do Docker chọn (không đụng port đang dùng).
  # fsync=off ...       database test dùng xong bỏ: đổi độ bền lấy tốc độ.
  # max_connections     test chạy song song, mỗi test một pool nhỏ.
  docker run --detach --name "$CONTAINER" \
    --env POSTGRES_PASSWORD="$password" \
    --publish 127.0.0.1::5432 \
    "$POSTGRES_IMAGE" \
    -c fsync=off -c synchronous_commit=off -c full_page_writes=off \
    -c max_connections=300 >/dev/null

  # Image khởi động một máy chủ tạm (chỉ nghe unix socket) để khởi tạo rồi mới
  # chạy máy chủ thật; hỏi qua TCP (-h 127.0.0.1) để chỉ nhận máy chủ thật.
  local attempt
  for attempt in $(seq 1 120); do
    if docker exec "$CONTAINER" pg_isready --quiet -h 127.0.0.1 -U postgres 2>/dev/null; then
      local port
      port="$(docker port "$CONTAINER" 5432/tcp | head -n 1 | sed 's/.*://')"
      POSTGRES_URL="postgres://postgres:${password}@127.0.0.1:${port}/postgres"
      return 0
    fi
    sleep 0.5
  done
  docker logs "$CONTAINER" >&2 || true
  fail "Postgres container did not become ready within 60 seconds."
}

POSTGRES_URL="${GATE_POSTGRES_URL:-}"
if [ -n "$POSTGRES_URL" ]; then
  step "tests on Postgres (GATE_POSTGRES_URL)"
else
  step "tests on Postgres ($POSTGRES_IMAGE in Docker)"
  start_postgres
fi
TEST_DATABASE_URL="$POSTGRES_URL" cargo xtask test-all

printf '\n\033[32mgate: green — fmt, clippy, boundaries, api contract, dependencies, secrets, tests on SQLite and Postgres (files also on S3)\033[0m\n'
