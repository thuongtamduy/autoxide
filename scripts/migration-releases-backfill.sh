#!/usr/bin/env bash
# Lấy danh sách migration của các bản đã phát hành từ image Docker của từng bản (chạy thật `migrate up` rồi
# `migrate status` trên một SQLite tạm, `AUTOXIDE_MODULES=all` để có cả module thử nghiệm), in ra mỗi bản một file `<thư mục>/<X.Y.Z>.txt` gồm các dòng
# `<module>/<tên>` theo thứ tự `migrate status`. Dùng một lần để điền `crates/app/migration-releases.json` cho các
# bản phát hành trước khi có file này (D-20261126-upgrade-rollback); bản mới do `cargo xtask migration-releases
# record` ghi lúc phát hành. Cần Docker và mạng tới ghcr.io.
#
#   scripts/migration-releases-backfill.sh <thư mục ra> 0.17.0 0.18.0 0.19.0
set -euo pipefail
out="$1"
shift
image="${AUTOXIDE_IMAGE:-ghcr.io/thuongtamduy/autoxide}"
mkdir -p "$out"
for version in "$@"; do
  data="$(mktemp -d)"
  chmod 777 "$data"
  url="sqlite:///var/lib/autoxide/data/backfill.sqlite?mode=rwc"
  if ! docker pull -q "$image:$version" >/dev/null 2>&1; then
    echo "$version: no image, skipped" >&2
    rm -rf "$data"
    continue
  fi
  docker run --rm -v "$data:/var/lib/autoxide/data" -e "DATABASE_URL=$url" -e AUTOXIDE_MODULES=all "$image:$version" migrate up >/dev/null
  docker run --rm -v "$data:/var/lib/autoxide/data" -e "DATABASE_URL=$url" -e AUTOXIDE_MODULES=all "$image:$version" migrate status \
    | awk '$3 == "applied" { print $1 "/" $2 }' >"$out/$version.txt"
  rm -rf "$data"
  echo "$version: $(wc -l <"$out/$version.txt") migrations"
done
