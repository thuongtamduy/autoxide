#!/usr/bin/env bash
# Thay đổi có chạm Rust không (D-20261125-ci-skip-rust)? In `true` hoặc `false` ra stdout.
#
#   scripts/rust-changed.sh <base>    file đã đổi kể từ chỗ rẽ nhánh với <base> (commit, cộng thay đổi chưa commit
#                                     và file mới chưa theo dõi); <base> không có thì in `true`.
#   scripts/rust-changed.sh --stdin   danh sách file (mỗi dòng một đường dẫn tính từ gốc repo) đọc từ stdin.
#
# `false` khi MỌI file đã đổi khớp danh sách cho phép `scripts/rust-free-paths.txt` (tài liệu, SDK TypeScript,
# changelog, quyết định…): khi đó biên dịch, clippy và test Rust chắc chắn cho cùng kết quả như ở <base>. Một file
# không khớp là `true` — nghi ngờ thì chạy. Không có file nào đổi thì `false`.
#
# Dùng ở: gate.yml (pull request: job `build tests`, các shard test, `external module example` bỏ qua phần nặng),
# gate.sh (`--pr` và `--quick`: bỏ clippy) và job `plan` của ci.yml (job MSRV; push lên main so với lần CI xanh gần nhất
# trên main, D-20261127-ci-runner-budget), job `plan` của codeql.yml (pull request: bỏ `analyze (rust)`,
# D-20261128-codeql-rust-on-change). Gate của push lên main, tag và chạy tay không gọi script này: luôn chạy đủ.
# Lý do chọn file nào bỏ qua được ghi ngay trong danh sách; cùng luật khớp viết lại ở `xtask/src/rust_free.rs` (test
# so hai bản).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"

rules=()
while IFS= read -r line || [ -n "$line" ]; do
  # Bỏ dòng trống và dòng chú thích.
  line="${line%"${line##*[![:space:]]}"}"
  case "$line" in "" | "#"*) continue ;; esac
  rules+=("$line")
done <"$root/scripts/rust-free-paths.txt"

# File có nằm trong danh sách cho phép không: dòng đầu tiên khớp quyết định; `!` là ngoại lệ (chạm Rust).
rust_free() {
  local rule
  for rule in "${rules[@]}"; do
    if [[ "$rule" == '!'* ]]; then
      # Vế phải không đặt trong ngoặc kép: là mẫu, `*` khớp mọi chuỗi kể cả `/`.
      # shellcheck disable=SC2053
      [[ "$1" == ${rule#!} ]] && return 1
    else
      # shellcheck disable=SC2053
      [[ "$1" == $rule ]] && return 0
    fi
  done
  return 1
}

case "${1:-}" in
  --stdin) files="$(cat)" ;;
  "")
    echo "usage: scripts/rust-changed.sh <base> | --stdin" >&2
    exit 2
    ;;
  *)
    base="$1"
    cd "$root"
    if ! git rev-parse --verify --quiet "$base^{commit}" >/dev/null; then
      echo "rust-changed: '$base' not found, assuming Rust changed" >&2
      echo true
      exit 0
    fi
    fork="$(git merge-base "$base" HEAD)"
    # Không có ref thứ hai: so với cây làm việc, nên tính cả thay đổi chưa commit. `--no-renames`: file đổi tên
    # tính cả tên cũ lẫn tên mới.
    files="$(git diff --name-only --no-renames "$fork"; git ls-files --others --exclude-standard)"
    ;;
esac

while IFS= read -r file; do
  [ -n "$file" ] || continue
  if ! rust_free "$file"; then
    echo "rust-changed: $file is not in scripts/rust-free-paths.txt: Rust builds and tests run" >&2
    echo true
    exit 0
  fi
done <<<"$files"
echo "rust-changed: every changed file is in scripts/rust-free-paths.txt: Rust builds and tests are skipped" >&2
echo false
