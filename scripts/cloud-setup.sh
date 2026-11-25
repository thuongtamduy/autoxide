#!/usr/bin/env bash
# Chuẩn bị máy mới (phiên agent trên cloud, container dùng xong bỏ) để chạy được `./gate.sh --quick`
# ngay: toolchain Rust đúng bản ghim và các công cụ ngoài mà gate cần. Xem docs/CLOUD-SESSIONS.md.
#
#   scripts/cloud-setup.sh           toolchain + công cụ (cargo-deny, gitleaks, cargo-nextest, cargo-semver-checks) + lịch sử git đầy đủ + mục lục quyết định
#   scripts/cloud-setup.sh --warm    thêm `cargo fetch`, build sẵn `xtask` và test của workspace (làm nóng cache)
#
# Chạy lại nhiều lần được: thứ đã có đúng phiên bản thì bỏ qua. Không cần secret, chỉ cần mạng tới
# static.rust-lang.org và github.com. Hỏng bước nào thì thoát với thông báo nói rõ bước đó.
#
# Chuỗi cung ứng (D-20261123-supply-chain): mỗi công cụ ghim phiên bản VÀ SHA-256 của file tải về; sai
# checksum thì dừng, không cài. Nâng phiên bản: đổi ba dòng (phiên bản, URL, SHA-256) của công cụ đó
# — phiên bản phải khớp phiên bản CI dùng (`.github/workflows/gate.yml`, `audit.yml`; gitleaks ở
# `gate.sh`/`ci.yml`) — và lấy SHA-256 từ file checksum của nhà phát hành nếu có.
#
# Phiên bản Rust KHÔNG viết ở đây: đọc từ rust-toolchain.toml (nguồn duy nhất).

set -euo pipefail

WARM=0
for arg in "$@"; do
  case "$arg" in
    --warm) WARM=1 ;;
    *)
      echo "usage: scripts/cloud-setup.sh [--warm]" >&2
      exit 2
      ;;
  esac
done

cd "$(dirname "$0")/.."

# --- Phiên bản công cụ (giữ khớp CI) ----------------------------------------------------------------
# Binary dựng sẵn (không `cargo install`: biên dịch cargo-deny mất nhiều phút). x86_64 Linux.
CARGO_DENY_VERSION="0.22.0"
CARGO_DENY_URL="https://github.com/EmbarkStudios/cargo-deny/releases/download/${CARGO_DENY_VERSION}/cargo-deny-${CARGO_DENY_VERSION}-x86_64-unknown-linux-musl.tar.gz"
CARGO_DENY_SHA256="98c56e6a408bfa5c3c60744b6015aaa6aa08f743fdeb88386d06fca035bb1786" # khớp file .sha256 của nhà phát hành

GITLEAKS_VERSION="8.30.1"
GITLEAKS_URL="https://github.com/gitleaks/gitleaks/releases/download/v${GITLEAKS_VERSION}/gitleaks_${GITLEAKS_VERSION}_linux_x64.tar.gz"
GITLEAKS_SHA256="551f6fc83ea457d62a0d98237cbad105af8d557003051f41f3e7ca7b3f2470eb" # khớp gitleaks_${GITLEAKS_VERSION}_checksums.txt

NEXTEST_VERSION="0.9.152"
NEXTEST_URL="https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-${NEXTEST_VERSION}/cargo-nextest-${NEXTEST_VERSION}-x86_64-unknown-linux-gnu.tar.gz"
NEXTEST_SHA256="d039b43c3e0958441b6ad0a39c8884d018b3d2e8fb9928f209074b1100b5e17a" # nhà phát hành không công bố checksum: giá trị đo lúc ghim

SEMVER_CHECKS_VERSION="0.51.0" # khớp `.github/workflows/ci.yml` và STACK.md
SEMVER_CHECKS_URL="https://github.com/obi1kenobi/cargo-semver-checks/releases/download/v${SEMVER_CHECKS_VERSION}/cargo-semver-checks-x86_64-unknown-linux-musl.tar.gz"
SEMVER_CHECKS_SHA256="396c8508d973c83de52660943a057e971b586f7511c3ba695762373452b43d5b" # nhà phát hành không công bố checksum: giá trị đo lúc ghim

# Nơi đặt binary: `~/.cargo/bin` đã nằm trong PATH cùng cargo, không cần quyền root.
BIN_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"

log() { printf 'cloud-setup: %s\n' "$*"; }
die() {
  printf 'cloud-setup: LỖI — %s\n' "$*" >&2
  exit 1
}

[ "$(uname -s)-$(uname -m)" = "Linux-x86_64" ] ||
  die "script này ghim binary Linux x86_64; máy này là $(uname -s)-$(uname -m). Cài công cụ theo tay (xem docs/CLOUD-SESSIONS.md)."
for tool in curl tar sha256sum; do
  command -v "$tool" >/dev/null 2>&1 || die "thiếu '$tool'."
done
command -v rustup >/dev/null 2>&1 || die "thiếu rustup. Cài từ https://rustup.rs rồi chạy lại."

# --- 1. Toolchain Rust -------------------------------------------------------------------------------
channel="$(sed -n 's/^channel *= *"\(.*\)"/\1/p' rust-toolchain.toml | head -n 1)"
[ -n "$channel" ] || die "không đọc được channel từ rust-toolchain.toml."
if rustup toolchain list | grep -q "^${channel}-" && rustup component list --toolchain "$channel" --installed 2>/dev/null | grep -q '^clippy'; then
  log "toolchain $channel: đã có"
else
  log "toolchain $channel: đang cài (đọc rust-toolchain.toml)"
  rustup toolchain install || die "không cài được toolchain $channel (kiểm tra mạng tới static.rust-lang.org)."
fi

# --- 2. Công cụ ngoài ---------------------------------------------------------------------------------
# install_tool <lệnh> <phiên bản> <url> <sha256> <đường dẫn file trong archive, `*` khớp một thư mục>
install_tool() {
  local name="$1" version="$2" url="$3" sha="$4" member="$5" work
  if command -v "$name" >/dev/null 2>&1 && "$name" --version 2>/dev/null | grep -qF "$version"; then
    log "$name $version: đã có"
    return 0
  fi
  log "$name $version: đang tải"
  work="$(mktemp -d)"
  # Dọn thư mục tạm kể cả khi một bước hỏng.
  # shellcheck disable=SC2064
  trap "rm -rf '$work'" RETURN
  curl --fail --silent --show-error --location --retry 3 --output "$work/archive.tgz" "$url" ||
    die "không tải được $name $version từ $url (kiểm tra mạng tới github.com)."
  echo "$sha  $work/archive.tgz" | sha256sum --check --status ||
    die "SHA-256 của $name $version không khớp giá trị ghim — KHÔNG cài. Nếu vừa nâng phiên bản thì cập nhật SHA-256 trong script."
  mkdir "$work/x"
  tar -xzf "$work/archive.tgz" -C "$work/x" --wildcards "$member" ||
    die "không giải nén được $member từ archive của $name."
  mkdir -p "$BIN_DIR"
  install -m 0755 "$work"/x/$member "$BIN_DIR/$name"
  "$BIN_DIR/$name" --version >/dev/null 2>&1 || die "$name vừa cài nhưng không chạy được."
  log "$name $version: đã cài vào $BIN_DIR"
}

install_tool cargo-deny "$CARGO_DENY_VERSION" "$CARGO_DENY_URL" "$CARGO_DENY_SHA256" '*/cargo-deny'
install_tool gitleaks "$GITLEAKS_VERSION" "$GITLEAKS_URL" "$GITLEAKS_SHA256" 'gitleaks'
install_tool cargo-nextest "$NEXTEST_VERSION" "$NEXTEST_URL" "$NEXTEST_SHA256" 'cargo-nextest'
install_tool cargo-semver-checks "$SEMVER_CHECKS_VERSION" "$SEMVER_CHECKS_URL" "$SEMVER_CHECKS_SHA256" 'cargo-semver-checks'

# --- 3. Lịch sử git đầy đủ và tag ------------------------------------------------------------------------
# Máy cloud clone nông (shallow); `cargo xtask api-diff` (bước của gate) cần tag phát hành gần nhất và
# gitleaks quét cả lịch sử. Đã đủ lịch sử thì bỏ qua.
if [ "$(git rev-parse --is-shallow-repository 2>/dev/null)" = true ]; then
  log "git: clone nông, đang lấy đủ lịch sử và tag"
  git fetch --unshallow --tags --quiet || die "git fetch --unshallow hỏng (kiểm tra mạng và quyền đọc remote)."
else
  log "git: đã đủ lịch sử"
fi

# --- 4. Làm nóng cache (tuỳ chọn) ----------------------------------------------------------------------
if [ "$WARM" = 1 ]; then
  log "cargo fetch --locked"
  cargo fetch --locked || die "cargo fetch hỏng (kiểm tra mạng tới crates.io)."
  log "build trước xtask"
  cargo build -p xtask --locked || die "build xtask hỏng."
  # Build trước test của cả workspace: đúng lệnh và đúng cấu hình (CARGO_INCREMENTAL=0) mà
  # `./gate.sh --quick` dùng, nên lần gate đầu chỉ còn dựng phần đã sửa (D-20261125-cloud-dev-friction).
  # Tốn ~13 phút (máy 4 CPU) và ~12 GB đĩa: bỏ qua khi đĩa trống dưới 25 GB, không làm hỏng script.
  free_gb="$(df -Pk . | awk 'NR==2 {print int($4 / 1048576)}')"
  if [ "${free_gb:-0}" -lt 25 ]; then
    log "bỏ qua build test trước: đĩa trống ${free_gb} GB (< 25 GB)"
  else
    log "build trước test của workspace (vài phút, ~12 GB đĩa)"
    CARGO_INCREMENTAL=0 cargo test --workspace --locked --no-run ||
      log "build test trước lỗi, bỏ qua (gate sẽ dựng lại khi chạy)"
  fi
fi

# --- 5. Mục lục quyết định ----------------------------------------------------------------------------
# `docs/decisions/INDEX.md` không nằm trong git (D-20261125-decision-index-untracked); CLAUDE.md dặn đọc nó
# trước khi tìm quyết định. Không làm hỏng script khi lỗi: chỉ là tiện cho agent.
if cargo xtask decisions >/dev/null 2>&1; then
  log "mục lục quyết định: đã sinh docs/decisions/INDEX.md"
else
  log "mục lục quyết định: chưa sinh được (chạy 'cargo xtask decisions' sau)"
fi

# --- 6. PostgreSQL cục bộ cho test (chỉ máy cloud) ------------------------------------------------------
# Máy cloud có sẵn PostgreSQL nhưng tắt, và không có Docker daemon. Bật cụm có sẵn, đặt mật khẩu cho role
# `postgres` (chỉ nghe loopback trong container dùng xong bỏ), để chạy bài test trên Postgres ngay
# (D-263, D-20261125-cloud-dev-friction). Thiếu binary hay không đủ quyền thì bỏ qua, không làm hỏng script.
POSTGRES_TEST_URL="postgres://postgres:autoxide@127.0.0.1:5432/postgres"
if [ "${CLAUDE_CODE_REMOTE:-}" = true ] && command -v pg_lsclusters >/dev/null 2>&1 && command -v pg_ctlcluster >/dev/null 2>&1; then
  pg_version="$(pg_lsclusters --no-header 2>/dev/null | awk '$2 == "main" {print $1; exit}')"
  if [ -z "$pg_version" ]; then
    log "postgres: không có cụm 'main', bỏ qua"
  elif { pg_lsclusters --no-header | awk -v v="$pg_version" '$1 == v && $2 == "main" && $4 == "online" {found=1} END {exit !found}'; } ||
    pg_ctlcluster "$pg_version" main start >/dev/null 2>&1; then
    # `su` chạy được khi script chạy bằng root (máy cloud); không thì chỉ báo.
    if su postgres -c "psql -qtAc \"alter user postgres password 'autoxide'\"" >/dev/null 2>&1 &&
      PGPASSWORD=autoxide psql -h 127.0.0.1 -U postgres -qtAc 'select 1' >/dev/null 2>&1; then
      log "postgres $pg_version: đang chạy. Chạy test trên Postgres: export TEST_DATABASE_URL=$POSTGRES_TEST_URL"
    else
      log "postgres $pg_version: đã bật nhưng không đặt được mật khẩu / kết nối qua 127.0.0.1 (xem docs/CLOUD-SESSIONS.md)"
    fi
  else
    log "postgres $pg_version: không bật được cụm (xem docs/CLOUD-SESSIONS.md)"
  fi
fi

log "xong. Chạy ./gate.sh --quick"
