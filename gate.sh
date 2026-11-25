#!/usr/bin/env bash
# Gate của Autoxide: định nghĩa "xong". Hai mức (D-173):
#
#   ./gate.sh --quick   gate nhanh, trước mỗi commit lúc đang làm một bước: như dưới đây
#                       nhưng chỉ chạy test của crate bị sửa (kể từ lần gate nhanh xanh gần
#                       nhất, chưa có thì so với origin/main) và crate phụ thuộc vào chúng, chỉ
#                       trên SQLite, không cần Docker. Test chạy bằng nextest khi có (từ 0.9.85; cài mà cũ
#                       hơn thì đỏ), không thì bộ chạy `cargo xtask`.
#   ./gate.sh           gate đầy đủ: bắt buộc trước khi merge vào main và trước khi phát hành.
#   ./gate.sh --pr      gate của pull request trên CI (D-20261118-ci-pr-affected-tests): như gate đầy đủ nhưng chỉ chạy test
#                       của crate bị sửa so với GATE_BASE (mặc định origin/main) và crate phụ thuộc
#                       vào chúng, trên cả SQLite và Postgres, không chạy doc-test. Push lên main
#                       vẫn chạy gate đầy đủ.
#
#   --only lint|sqlite|postgres   đi kèm gate đầy đủ hoặc `--pr`: chỉ chạy một phần, để CI chạy ba
#                       phần trên ba máy cùng lúc (D-20261119-ci-faster). Ba phần cộng lại đúng bằng
#                       gate không có cờ này:
#                         lint      bước 1–6 (fmt, clippy, xtask check, api-diff, sdk --check, deny,
#                                   gitleaks);
#                         sqlite    bước 7 (kèm doc-test ở gate đầy đủ);
#                         postgres  bước 8.
#                       Không có `--only` thì chạy cả ba, như cũ.
#   --only example      module ở repo riêng (D-20261121-external-modules): `examples/external-module/check.sh` — workspace
#                       Cargo riêng gồm một module và binary ghép nó với module chính thức; fmt, clippy, test trên
#                       SQLite và (khi có GATE_POSTGRES_URL) Postgres — cộng `cargo xtask check-module` trên module
#                       đó; rồi sinh khung `cargo xtask new-module --external` vào thư mục tạm và lint + test nó trên
#                       SQLite. CI chạy job riêng song song. Gate đầy đủ không có `--only` cũng chạy phần này; gate
#                       nhanh chỉ chạy khi `examples/` hay `xtask/src/scaffold.rs` đổi so với origin/main (nó build
#                       lại cả core ở workspace riêng).
#   --only build        đi kèm gate đầy đủ hoặc `--pr`: build mọi chương trình test MỘT lần thành archive
#                       của nextest ở GATE_ARCHIVE_DIR (D-20261120-ci-nextest-archive), kèm file `filter`
#                       (filterset chọn test: `all()` ở gate đầy đủ, crate bị ảnh hưởng ở `--pr`). Gate đầy
#                       đủ chạy luôn doc-test ở đây (chúng cần trình biên dịch). Không chạy test nào khác.
#   --archive DIR       đi kèm `--only sqlite|postgres`: chạy test từ archive trong DIR (do `--only build`
#                       làm, có thể ở máy khác) bằng nextest thay vì build rồi chạy bằng `cargo xtask`;
#                       không biên dịch gì. `lint` + `build` + `sqlite --archive` + `postgres --archive`
#                       cộng lại đúng bằng gate không có cờ. Trên Postgres tối đa 16 test chạy cùng lúc,
#                       như `cargo xtask test-all`.
#   --shard k/n         đi kèm gate đầy đủ hoặc `--pr`: bước test (7, 8) chỉ chạy phần thứ k trong n
#                       phần của mọi test (`cargo xtask … --shard`); n phần cộng lại đúng bằng mọi
#                       test, doc-test chạy ở phần 1. CI chạy mỗi phần trên một máy (D-20261119-ci-faster).
#
# CI trên GitHub (.github/workflows/ci.yml): pull request chạy `--pr`, push lên main chạy gate đầy
# đủ (D-175, D-197), mỗi lần tách thành nhiều job: `--only lint`, `--only build` (archive test), rồi
# `--only sqlite|postgres --archive … --shard k/n` chạy song song (D-20261120-ci-nextest-archive);
# gate trên máy vẫn là định nghĩa "xong", CI là lưới thứ hai.
#
# Thay đổi không chạm Rust (D-20261125-ci-skip-rust): `scripts/rust-changed.sh` in `false` khi mọi file đã đổi nằm
# trong danh sách cho phép `scripts/rust-free-paths.txt` (tài liệu, SDK TypeScript, changelog, quyết định…). Khi đó
# `--pr` và `--quick` bỏ clippy (bước 2) — kết quả của nó không thể khác ở base; các bước rẻ còn lại vẫn chạy. Trên CI,
# pull request như vậy còn bỏ phần nặng của job `build tests`, các shard test và `external module example` (gate.yml);
# job vẫn báo xong cùng tên. Gate đầy đủ (push lên main, tag, chạy tay) không bao giờ bỏ.
#
# Cả hai chạy ở mức ưu tiên CPU thấp (`renice`) để máy vẫn dùng được trong lúc build.
#
# Theo thứ tự (dừng ở bước đỏ đầu tiên). Số là nhóm; thứ tự chạy thật: các kiểm rẻ 1, 3 (ranh giới) và SDK của 4 chạy ĐẦU
# TIÊN (fail nhanh), rồi AVIF / ví dụ module khi cần, rồi clippy, kiểm file sinh (`cargo xtask regen --check`), api-diff, 5, 6, 7, 8.
#   1. định dạng            cargo fmt --check
#   2. lint                 cargo clippy, cảnh báo tính là lỗi (`--pr`/`--quick` bỏ khi thay đổi không chạm Rust)
#      avif                 clippy + test AVIF của `media` với `--features avif` (feature không bật mặc
#                           định, D-20261121-avif-feature); phần riêng `--only avif`, CI chạy job riêng
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
#   GATE_GITLEAKS_IMAGE  image gitleaks khi máy không cài gitleaks (mặc định: dòng `FROM` của
#                        .github/gitleaks/Dockerfile, ghim theo digest, cùng image với CI).
#   GATE_BASE            với --pr: so với ref này để biết crate nào bị sửa (mặc định origin/main); với --pr và
#                        --quick: thay đổi kể từ chỗ rẽ nhánh với ref này có chạm Rust không (bỏ clippy).
#   GATE_ARCHIVE_DIR     với --only build: thư mục ghi archive test và file filter (mặc định
#                        target/gate-archive).
#   GATE_DEPS_OPT_LEVEL  với --only build: mức tối ưu (0|1|2|3) của THƯ VIỆN NGOÀI trong archive test. Mặc định
#                        `0` trên GitHub Actions (build một lần, chạy một lần: build từ đầu nhanh hơn ~23%,
#                        D-20261120-build-faster), còn lại để nguyên `opt-level = 1` của Cargo.toml
#                        (máy dev chạy test nhiều lần). Rỗng: không đổi.
#   GATE_TARGET_MAX_GB   trần dung lượng của target/ (mặc định 40): vượt thì gate dọn trước khi build
#                        (cache incremental trước, vẫn vượt thì `cargo clean`). 0: không dọn.
#
# Gate build không incremental (CARGO_INCREMENTAL=0, như CI): cache incremental chỉ giúp sửa rồi
# build lại một crate trong lúc làm, còn gate build lại nhiều crate và cache đó phình theo mỗi nhánh.
#
# Gate đỏ thì sửa code. Không nới gate, không tắt lint, không bỏ qua test
# (CLAUDE.md, mục "Gate").

set -euo pipefail
cd "$(dirname "$0")"

QUICK=0
PR=0
ONLY=""
SHARD=()
ARCHIVE=""
usage() {
  echo "usage: ./gate.sh [--quick | --pr] [--only lint|avif|example|build|sqlite|postgres] [--archive DIR] [--shard k/n]" >&2
  exit 2
}
while [ $# -gt 0 ]; do
  case "$1" in
    --quick) QUICK=1 ;;
    --pr) PR=1 ;;
    --only)
      [ $# -ge 2 ] || usage
      ONLY="$2"
      shift
      ;;
    --archive)
      [ $# -ge 2 ] || usage
      ARCHIVE="$2"
      shift
      ;;
    --shard)
      [ $# -ge 2 ] || usage
      [[ "$2" =~ ^[1-9][0-9]*/[1-9][0-9]*$ ]] || usage
      SHARD=(--shard "$2")
      shift
      ;;
    *) usage ;;
  esac
  shift
done
[ "$QUICK" = 1 ] && [ "$PR" = 1 ] && usage
case "$ONLY" in
  "" | lint | avif | example | build | sqlite | postgres) ;;
  *) usage ;;
esac
# Archive chỉ để chạy test của một database; `build` thì không chia phần.
if [ -n "$ARCHIVE" ]; then
  case "$ONLY" in sqlite | postgres) ;; *) usage ;; esac
fi
[ "$ONLY" = build ] && [ ${#SHARD[@]} -gt 0 ] && usage
# Gate nhanh vốn chỉ có SQLite và đã nhanh: không chia.
[ "$QUICK" = 1 ] && [ -n "$ONLY" ] && usage
[ "$QUICK" = 1 ] && [ ${#SHARD[@]} -gt 0 ] && usage
# Phần `part` có được chạy không (không có `--only` thì mọi phần đều chạy).
runs() { [ -z "$ONLY" ] || [ "$ONLY" = "$1" ]; }

# Ưu tiên CPU thấp cho cả script và mọi tiến trình con (cargo, rustc, test): việc khác trên máy
# được chạy trước, gate dùng phần CPU còn thừa. Không có `renice` thì chạy bình thường.
renice -n 10 -p $$ >/dev/null 2>&1 || true

POSTGRES_IMAGE="${GATE_POSTGRES_IMAGE:-postgres:16-alpine}"
S3_IMAGE="${GATE_S3_IMAGE:-motoserver/moto:5.2.5}"
# Image gitleaks ghim theo digest ở một chỗ, CI đọc cùng dòng này (I-5, D-20261125-gitleaks-digest).
GITLEAKS_IMAGE="${GATE_GITLEAKS_IMAGE:-$(sed -n 's/^FROM //p' .github/gitleaks/Dockerfile)}"
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

# Kiểm mọi công cụ cần cho lần chạy này NGAY ĐẦU và báo hết một lần: trước đây thiếu gitleaks/Docker chỉ lộ ra sau
# khi clippy đã chạy xong, và thiếu hai thứ thì phải chạy lại hai lần mới biết.
MISSING=()
need() {
  command -v "$1" >/dev/null 2>&1 || MISSING+=("  - $1: $2")
}
need cargo "install Rust from https://rustup.rs"
if [ "$ONLY" = build ] || [ -n "$ARCHIVE" ]; then
  need cargo-nextest "cargo install cargo-nextest --locked (or a release binary: https://nexte.st)"
fi
if runs lint; then
  need cargo-deny "cargo install cargo-deny --locked (or a release binary: https://github.com/EmbarkStudios/cargo-deny/releases)"
  if ! command -v gitleaks >/dev/null 2>&1; then
    need docker "the secret scan runs gitleaks in Docker; install Docker or gitleaks (https://github.com/gitleaks/gitleaks)"
  fi
fi
if [ ${#MISSING[@]} -gt 0 ]; then
  fail "missing tools for this run:
$(printf '%s\n' "${MISSING[@]}")"
fi

export CARGO_INCREMENTAL=0

# --- test không được ghi file vào cây mã nguồn ---------------------------------------------------------
# Test chạy trong thư mục của crate: đường dẫn tương đối mặc định (thư mục dữ liệu, file mật khẩu tự
# sinh…) rơi vào cây mã nguồn rồi bị commit nhầm. Chụp danh sách file chưa theo dõi (không tính file
# trong .gitignore) trước khi chạy test; trước khi báo xanh, file mới xuất hiện thì đỏ
# (D-20261121-admin-password-path).
untracked() { git ls-files --others --exclude-standard 2>/dev/null | sort; }
UNTRACKED_BEFORE="$(untracked)"
tree_unchanged() {
  local added
  added="$(comm -13 <(printf '%s\n' "$UNTRACKED_BEFORE") <(untracked) | sed '/^$/d')"
  [ -z "$added" ] || fail "the run left new files in the source tree (a test wrote to a relative path; give it a temporary directory):
$added"
}

# --- dọn target/ khi quá trần (D-20261118-build-size) ---------------------------------------------
# Cargo không bao giờ tự xoá bản build cũ: mỗi nhánh, mỗi Cargo.lock khác nhau để lại một bộ file.
target_gb() { du -s --block-size=1G target 2>/dev/null | cut -f1 || echo 0; }
TARGET_MAX_GB="${GATE_TARGET_MAX_GB:-40}"
if [ "$TARGET_MAX_GB" != 0 ] && [ -d target ] && [ "$(target_gb)" -gt "$TARGET_MAX_GB" ]; then
  step "target/ is over ${TARGET_MAX_GB} GB: removing the incremental cache"
  rm -rf target/*/incremental
  if [ "$(target_gb)" -gt "$TARGET_MAX_GB" ]; then
    step "target/ is still over ${TARGET_MAX_GB} GB: cargo clean (the next build starts from scratch)"
    cargo clean
  fi
fi

# --- Kiểm rẻ chạy ĐẦU TIÊN (fail nhanh, D-20261125-regen-and-gate-order) ---------------------------------
# Định dạng, ranh giới workspace và SDK chỉ mất vài giây: lỗi ở đây phải lộ ra ngay, không sau ~10 phút
# clippy / AVIF / ví dụ module. Thuộc nhóm `lint`; số thứ tự ở chú thích đầu file là nhóm, không phải thứ tự chạy.
# `--pr` / `--quick`: thay đổi không chạm Rust (mọi file nằm trong scripts/rust-free-paths.txt) thì clippy và kiểm file
# sinh cho đúng kết quả như ở base — bỏ, vì trên máy mới nó là cả một lần biên dịch workspace (D-20261125-ci-skip-rust).
RUST_CHANGED=true
if runs lint && { [ "$QUICK" = 1 ] || [ "$PR" = 1 ]; }; then
  RUST_CHANGED="$(scripts/rust-changed.sh "${GATE_BASE:-origin/main}")"
fi
CLIPPY="clippy"
[ "$RUST_CHANGED" = true ] || CLIPPY="clippy skipped (no Rust-relevant change)"
if runs lint; then
  step "format (cargo fmt)"
  cargo fmt --all --check

  step "workspace boundaries (cargo xtask check)"
  cargo xtask check

  # Kiểu TypeScript của SDK sinh từ docs/openapi.json; không cần Node (D-243).
  step "TypeScript SDK matches the contract (cargo xtask sdk --check)"
  cargo xtask sdk --check
fi

# --- AVIF: phần riêng (`--only avif`), không nằm trong lint ------------------------------------------
# Feature `avif` của `media` không bật mặc định (D-20261121-avif-feature) nên clippy và test thường không
# biên dịch phần AVIF: lint và chạy riêng test của `media` với feature đó (SQLite), để nó không mục. Tách
# khỏi lint vì biên dịch bộ mã hoá AV1 (rav1e) làm lint từ ~5 lên ~14 phút trên CI; CI chạy phần này ở job
# riêng song song. Gate nhanh chỉ chạy khi `media` hay danh sách thư viện đổi so với origin/main.
avif_needed() {
  [ "$QUICK" = 1 ] || return 0
  git rev-parse --verify --quiet "${GATE_BASE:-origin/main}" >/dev/null || return 0
  git diff --name-only "$(git merge-base "${GATE_BASE:-origin/main}" HEAD)" -- \
    modules/media Cargo.lock Cargo.toml gate.sh | grep -q .
}
if runs avif && avif_needed; then
  step "AVIF feature of media (cargo clippy + cargo test --features avif)"
  cargo clippy -p autoxide-media --all-targets --locked --features avif -- -D warnings
  env -u TEST_DATABASE_URL cargo test -p autoxide-media --locked --features avif --lib -- avif
fi
if [ "$ONLY" = avif ]; then
  tree_unchanged
  printf '\n\033[32mgate --only avif: green — AVIF feature of media\033[0m\n'
  exit 0
fi

# --- Module ở repo riêng: phần riêng (`--only example`) -----------------------------------------------
# `examples/external-module` là workspace Cargo riêng (D-20261121-external-modules): build lại cả core với bộ
# feature của người ngoài, nên không nằm trong lint hay test của workspace gốc. Thư mục build riêng
# (`examples/external-module/target`) trừ khi CARGO_TARGET_DIR đã đặt.
example_needed() {
  [ "$QUICK" = 1 ] || return 0
  git rev-parse --verify --quiet "${GATE_BASE:-origin/main}" >/dev/null || return 0
  git diff --name-only "$(git merge-base "${GATE_BASE:-origin/main}" HEAD)" -- examples xtask/src/scaffold.rs | grep -q .
}
if runs example && example_needed; then
  step "module outside the repository: rules (cargo xtask check-module)"
  cargo xtask check-module examples/external-module/bookmarks
  step "module outside the repository: examples/external-module/check.sh"
  POSTGRES_URL="${GATE_POSTGRES_URL:-}" ./examples/external-module/check.sh
  # Khung `cargo xtask new-module --external` sinh ra (D-20261122-external-template): sinh vào thư mục tạm, trỏ
  # core vào bản checkout này, rồi định dạng, lint và test trên SQLite. Dùng chung thư mục build với ví dụ nên chỉ
  # biên dịch lại module đổi tên và binary; Postgres đã chạy ở ví dụ cùng mã nguồn ngay trên.
  step "module outside the repository: generated workspace (cargo xtask new-module --external)"
  scaffold_dir="$(mktemp -d)"
  example_target="${CARGO_TARGET_DIR:-$PWD/examples/external-module/target}"
  # Cùng hồ sơ build với `check.sh` của ví dụ (trên CI: thư viện không tối ưu), để dùng lại bản đã biên dịch.
  EXAMPLE_PROFILE=()
  if [ -n "${GITHUB_ACTIONS:-}" ]; then
    EXAMPLE_PROFILE=(--config 'profile.dev.package."*".opt-level=0')
  fi
  cargo xtask new-module --external "$scaffold_dir/acme" reading_lists reading_list --core-path . >/dev/null
  cp Cargo.lock "$scaffold_dir/acme/Cargo.lock"
  (
    cd "$scaffold_dir/acme"
    export CARGO_TARGET_DIR="$example_target"
    cargo fmt --check -p acme-reading_lists -p acme-server
    cargo clippy --workspace --all-targets ${EXAMPLE_PROFILE[@]+"${EXAMPLE_PROFILE[@]}"} -- -D warnings
    env -u TEST_DATABASE_URL cargo test --workspace ${EXAMPLE_PROFILE[@]+"${EXAMPLE_PROFILE[@]}"}
  )
  rm -rf "$scaffold_dir"
fi
if [ "$ONLY" = example ]; then
  printf '\n\033[32mgate --only example: green — module outside the repository\033[0m\n'
  exit 0
fi

if runs lint; then

# --- Phần còn lại của lint: lâu hơn (biên dịch) hoặc cần mạng ---------------------------------------------

if [ "$RUST_CHANGED" = true ]; then
  step "lint (cargo clippy)"
  cargo clippy --workspace --all-targets --locked -- -D warnings
else
  step "lint (cargo clippy): skipped, no Rust-relevant change since ${GATE_BASE:-origin/main} (scripts/rust-free-paths.txt)"
fi

# File sinh tự động khớp code (openapi.json, mcp-tools.json, .env.example): trước đây chỉ lộ ra ở bài test, sau cả
# vòng gate. Cần dựng binary `autoxide`, cùng bản dựng mà bước test dùng sau đó, nên chỉ chạy khi có Rust đổi, và đứng
# sau clippy để lỗi biên dịch hiện ra bằng thông báo của clippy trước. Lệch thì thông báo ghi `cargo xtask regen`.
if [ "$RUST_CHANGED" = true ]; then
  step "generated files match the code (cargo xtask regen --check)"
  cargo xtask regen --check
fi

# So với tag phát hành gần nhất (tổ tiên của HEAD); cần lịch sử và tag (D-059, đặc tả 31 A1).
step "api contract (cargo xtask api-diff)"
cargo xtask api-diff

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

fi

if [ "$ONLY" = lint ]; then
  tree_unchanged
  printf '\n\033[32mgate%s --only lint: green — fmt, %s, boundaries, api contract, dependencies, secrets\033[0m\n' "$([ "$PR" = 1 ] && echo ' --pr')" "$CLIPPY"
  exit 0
fi

# --- Build test một lần thành archive (D-20261120-ci-nextest-archive) -----------------------------

if [ "$ONLY" = build ]; then
  ARCHIVE_DIR="${GATE_ARCHIVE_DIR:-target/gate-archive}"
  mkdir -p "$ARCHIVE_DIR"
  # Thư viện ngoài tối ưu nhẹ (opt-level 1, D-299) làm test chạy nhanh hơn nhưng build lâu hơn; archive của
  # CI chạy mỗi test đúng một lần nên chọn build nhanh. `--config` đè `[profile.dev.package."*"]`;
  # argon2 và blake2 vẫn giữ mức riêng của chúng trong Cargo.toml. Doc-test dùng cùng cấu hình để khỏi build lại.
  deps_opt="${GATE_DEPS_OPT_LEVEL-${GITHUB_ACTIONS:+0}}"
  PROFILE_ARGS=()
  if [ -n "$deps_opt" ]; then
    PROFILE_ARGS=(--config "profile.dev.package.\"*\".opt-level=$deps_opt")
  fi
  step "build every test program into a nextest archive ($ARCHIVE_DIR)"
  cargo nextest archive --workspace --locked ${PROFILE_ARGS[@]+"${PROFILE_ARGS[@]}"} --archive-file "$ARCHIVE_DIR/tests.tar.zst"
  if [ "$PR" = 1 ]; then
    BASE="${GATE_BASE:-origin/main}"
    step "tests to run: crates changed since $BASE and their dependents (cargo xtask test-filter)"
    cargo xtask test-filter --base "$BASE" > "$ARCHIVE_DIR/filter"
  else
    echo "all()" > "$ARCHIVE_DIR/filter"
    # Doc-test cần trình biên dịch nên chạy ở đây, không ở máy chạy archive; như bước 7 của gate
    # một máy, trên SQLite.
    step "doc tests"
    env -u TEST_DATABASE_URL cargo test --workspace --locked ${PROFILE_ARGS[@]+"${PROFILE_ARGS[@]}"} --doc
  fi
  printf 'filter: %s\n' "$(cat "$ARCHIVE_DIR/filter")"
  tree_unchanged
  printf '\n\033[32mgate%s --only build: green — test archive built%s\033[0m\n' \
    "$([ "$PR" = 1 ] && echo ' --pr')" "$([ "$PR" = 1 ] || echo ', doc tests')"
  exit 0
fi

# Chạy test từ archive bằng nextest, không biên dịch gì. Gọi thẳng `cargo-nextest` (không qua `cargo`):
# máy chạy không cần toolchain ghim trong rust-toolchain.toml. Archive giải nén vào chính thư mục
# repo: test dùng `env!("CARGO_BIN_EXE_autoxide")` (đường dẫn ghi lúc build, `target/debug/autoxide`).
# Mỗi test chạy trong tiến trình riêng (nextest). `$1`: số test chạy cùng lúc; phần còn lại: tham số cho `env`.
run_archive() {
  local threads="$1" filter partition=()
  shift
  filter="$(cat "$ARCHIVE/filter")"
  if [ ${#SHARD[@]} -gt 0 ]; then
    partition=(--partition "count:${SHARD[1]}")
  fi
  env "$@" cargo-nextest nextest run \
    --archive-file "$ARCHIVE/tests.tar.zst" \
    --workspace-remap "$PWD" --extract-to "$PWD" --extract-overwrite \
    --filterset "$filter" "${partition[@]}" \
    --test-threads "$threads" --no-fail-fast --no-tests pass
}

# --- Gate nhanh: test của crate bị ảnh hưởng, chỉ SQLite ---------------------------

if [ "$QUICK" = 1 ]; then
  step "tests on SQLite, changed crates only (cargo xtask test-affected)"
  cargo xtask test-affected
  tree_unchanged
  printf '\n\033[32mgate --quick: green — fmt, %s, boundaries, api contract, dependencies, secrets, tests of changed crates on SQLite\033[0m\n' "$CLIPPY"
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

BASE="${GATE_BASE:-origin/main}"

# `env -u`: bỏ biến nếu người chạy lỡ đặt sẵn, để bước này chắc chắn là SQLite.
if ! runs sqlite; then
  :
elif [ -n "$ARCHIVE" ]; then
  step "tests on SQLite, from the archive in $ARCHIVE (nextest)"
  # Mỗi nhân hai test: nhiều test chờ I/O (file, S3); `cargo xtask` cũng cho tới 4 chương trình
  # × 4 thread chạy cùng lúc trên máy 4 nhân.
  run_archive $(( $(nproc 2>/dev/null || echo 4) * 2 )) -u TEST_DATABASE_URL
elif [ "$PR" = 1 ]; then
  step "tests on SQLite, crates changed since $BASE only (cargo xtask test-affected)"
  env -u TEST_DATABASE_URL cargo xtask test-affected --base "$BASE" --keep-env "${SHARD[@]}"
else
  step "tests on SQLite"
  env -u TEST_DATABASE_URL cargo xtask test-all --doc "${SHARD[@]}"
fi

if [ "$ONLY" = sqlite ]; then
  tree_unchanged
  printf '\n\033[32mgate%s --only sqlite: green — tests on SQLite\033[0m\n' "$([ "$PR" = 1 ] && echo ' --pr')"
  exit 0
fi

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
if [ -n "$ARCHIVE" ]; then
  # 16: như POSTGRES_CONCURRENT_TESTS của `cargo xtask test-all` — dưới trần 300 kết nối.
  run_archive 16 TEST_DATABASE_URL="$POSTGRES_URL"
  tree_unchanged
  printf '\n\033[32mgate%s --only postgres --archive: green — tests on Postgres\033[0m\n' "$([ "$PR" = 1 ] && echo ' --pr')"
  exit 0
fi
if [ "$PR" = 1 ]; then
  TEST_DATABASE_URL="$POSTGRES_URL" cargo xtask test-affected --base "$BASE" --keep-env "${SHARD[@]}"
  if [ "$ONLY" = postgres ]; then
    tree_unchanged
    printf '\n\033[32mgate --pr --only postgres: green — tests of crates changed since %s on Postgres\033[0m\n' "$BASE"
    exit 0
  fi
  tree_unchanged
  printf '\n\033[32mgate --pr: green — fmt, clippy, boundaries, api contract, dependencies, secrets, tests of crates changed since %s on SQLite and Postgres\033[0m\n' "$BASE"
  printf 'Not checked: doc tests, crates the change does not reach. The push to main runs the full gate.\n'
  exit 0
fi
TEST_DATABASE_URL="$POSTGRES_URL" cargo xtask test-all "${SHARD[@]}"
if [ "$ONLY" = postgres ]; then
  tree_unchanged
  printf '\n\033[32mgate --only postgres: green — tests on Postgres (files also on S3)\033[0m\n'
  exit 0
fi

tree_unchanged
printf '\n\033[32mgate: green — fmt, clippy, boundaries, api contract, dependencies, secrets, tests on SQLite and Postgres (files also on S3)\033[0m\n'
