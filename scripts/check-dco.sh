#!/usr/bin/env bash
# Kiểm DCO (Developer Certificate of Origin, file `DCO` ở gốc repo): mỗi commit trong khoảng BASE..HEAD của một pull
# request phải có dòng `Signed-off-by: Tên <email>` (git commit -s). Bỏ qua: commit merge, và commit của tác giả có email
# trong `.github/dco-maintainers.txt` (người duy trì và phiên agent của họ — họ là bên cấp license, xem GOVERNANCE.md).
# Dùng: scripts/check-dco.sh <BASE> [HEAD]   (BASE là commit gốc của PR; HEAD mặc định là HEAD)
set -euo pipefail

base="${1:?usage: check-dco.sh <base> [head]}"
head="${2:-HEAD}"
exempt_file="$(git rev-parse --show-toplevel)/.github/dco-maintainers.txt"

failed=0
while read -r sha; do
  [ -n "$sha" ] || continue
  email="$(git log -1 --format=%ae "$sha")"
  if [ -f "$exempt_file" ] && grep -Fxqi -- "$email" <(grep -v '^[[:space:]]*#' "$exempt_file" | sed 's/[[:space:]]*$//'); then
    continue
  fi
  # `%(trailers:...)` chỉ nhận trailer ở khối cuối của message, đúng chỗ `git commit -s` đặt.
  if ! git log -1 --format='%(trailers:key=Signed-off-by,valueonly)' "$sha" | grep -q '<[^>]*@[^>]*>'; then
    echo "::error::commit ${sha:0:12} ($(git log -1 --format=%s "$sha")) has no 'Signed-off-by: Name <email>' line. Fix: git rebase --signoff <base> && git push --force-with-lease (see CONTRIBUTING.md, 'Sign your commits')."
    failed=1
  fi
done < <(git rev-list --no-merges "$base..$head")

exit "$failed"
