#!/usr/bin/env bash
# Thử scripts/check-dco.sh trên một repo git tạm: ai được miễn, ai bị chặn, và các cách đọc CODEOWNERS (CI chạy ở job `dco`).
set -euo pipefail

script="$(cd "$(dirname "$0")" && pwd)/check-dco.sh"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cd "$work"
git init -q -b main .
git config user.name Tester
git config user.email tester@example.invalid

commit() { # commit <message> [<file> <content>]
  if [ $# -ge 3 ]; then mkdir -p "$(dirname "$2")"; printf '%s\n' "$3" > "$2"; else echo x >> change.txt; fi
  git add -A
  git commit -q -m "$1"
}

pass=0
fail=0
expect() { # expect <0|1> <name> <command...>
  local want="$1" name="$2"; shift 2
  local got=0
  "$@" >/dev/null 2>&1 || got=1
  if [ "$got" = "$want" ]; then pass=$((pass + 1)); else fail=$((fail + 1)); echo "FAIL: $name (wanted exit $want)"; fi
}

mkdir .github
cat > .github/CODEOWNERS <<'OWNERS'
# @commented-out is not a maintainer
*            @Alice
/docs/       @acme/team someone@example.com @bob # @trailing-comment
OWNERS
git add -A && git commit -q -m "base"
base="$(git rev-parse HEAD)"

# Một commit chưa ký, rồi một commit ký.
commit "unsigned change"
unsigned="$(git rev-parse HEAD)"
commit $'signed change\n\nSigned-off-by: Tester <tester@example.invalid>'
signed="$(git rev-parse HEAD)"
git checkout -q "$base"
commit $'only signed\n\nSigned-off-by: Tester <tester@example.invalid>'
only_signed="$(git rev-parse HEAD)"

expect 0 "maintainer (any case) is exempt"        "$script" "$base" "$unsigned" alice
expect 0 "maintainer by exact case is exempt"     "$script" "$base" "$unsigned" Alice
expect 0 "dependabot is exempt"                   "$script" "$base" "$unsigned" 'dependabot[bot]'
expect 1 "stranger, unsigned commit, is refused"  "$script" "$base" "$unsigned" stranger
expect 0 "stranger, signed commits, passes"       "$script" "$base" "$only_signed" stranger
expect 1 "no author is refused when unsigned"     "$script" "$base" "$unsigned"
expect 1 "team member name is not a maintainer"   "$script" "$base" "$unsigned" acme
expect 1 "team name is not a maintainer"          "$script" "$base" "$unsigned" team
expect 1 "email local part is not a maintainer"   "$script" "$base" "$unsigned" someone
expect 1 "email domain is not a maintainer"       "$script" "$base" "$unsigned" example
expect 1 "commented-out login is not a maintainer" "$script" "$base" "$unsigned" commented-out
expect 1 "trailing comment is not a maintainer"   "$script" "$base" "$unsigned" trailing-comment
expect 0 "a login after a team on the line is read" "$script" "$base" "$unsigned" bob

# Un PR đổi CODEOWNERS của chính nó để tự miễn: danh sách phải lấy từ nhánh base, nên vẫn bị chặn.
git checkout -q "$base"
commit "add myself" .github/CODEOWNERS $'*  @stranger\n'
selfish="$(git rev-parse HEAD)"
expect 1 "a pull request cannot add itself to CODEOWNERS" "$script" "$base" "$selfish" stranger

echo "check-dco tests: $pass passed, $fail failed"
[ "$fail" = 0 ]
