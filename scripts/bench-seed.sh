#!/usr/bin/env bash
# Sinh bộ dữ liệu cỡ thật cho bài đo (bước 39 B1). CHỈ dùng cho database đo, không bao giờ cho
# dữ liệu thật: các lệnh từ chối chạy khi khách đã có dữ liệu.
#
#   scripts/bench-seed.sh small     # 1/100: 50 người dùng, 500 ảnh, 1 000 bài, 10 000 bình luận
#   scripts/bench-seed.sh full      # 5 000 người dùng, 50 000 ảnh, 100 000 bài, 1 000 000 bình luận
#
# Chạy trong thư mục làm việc của máy chủ đo, với cùng biến môi trường (DATABASE_URL, nơi lưu
# file…) như `autoxide serve`; database đã migrate (`autoxide migrate up`). Bài viết có hai ngôn
# ngữ khi AUTOXIDE_CONTENT_LOCALES có hai ngôn ngữ (vd `en,vi`). ID các bản ghi ghi vào
# `bench-ids.json` của thư mục hiện tại.
#
# Biến (không bắt buộc): BENCH_BINARY (binary đã build với `--features bench-seed`; không đặt thì
# build), BENCH_TENANT (slug khách, mặc định `default`).

set -euo pipefail

SIZE="${1:-small}"
case "$SIZE" in
  small) USERS=50 MEDIA=500 POSTS=1000 COMMENTS=10000 ;;
  full) USERS=5000 MEDIA=50000 POSTS=100000 COMMENTS=1000000 ;;
  *) echo "bench-seed: size must be 'small' or 'full', not '$SIZE'" >&2; exit 64 ;;
esac

if [ -n "${BENCH_BINARY:-}" ]; then
  BINARY="$BENCH_BINARY"
else
  ROOT="$(cd "$(dirname "$0")/.." && pwd)"
  cargo build --release --locked -q -p autoxide --features bench-seed --manifest-path "$ROOT/Cargo.toml"
  BINARY="$ROOT/target/release/autoxide"
fi

IDS="$PWD/bench-ids.json"
rm -f "$IDS"
common=(confirm=yes-this-is-a-benchmark-database "ids=$IDS" "tenant=${BENCH_TENANT:-default}")

seed() {
  local task="$1" count="$2" started
  started=$(date +%s)
  # Lệnh in cảnh báo cấu hình (chưa đặt SMTP…) ra stderr: chỉ giữ dòng kết quả.
  "$BINARY" task "$task" "${common[@]}" "count=$count" 2>/dev/null
  echo "  ($task: $(( $(date +%s) - started )) s)"
}

seed auth.bench_seed "$USERS"
seed media.bench_seed "$MEDIA"
seed content.bench_seed "$POSTS"
seed comments.bench_seed "$COMMENTS"
