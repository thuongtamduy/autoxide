#!/usr/bin/env bash
# Kiểm DCO (Developer Certificate of Origin, file `DCO` ở gốc repo): mỗi commit trong khoảng BASE..HEAD của một pull
# request phải có dòng `Signed-off-by: Tên <email>` (git commit -s). Bỏ qua commit merge.
# Miễn theo NGƯỜI MỞ PR, không theo email commit (email commit ai cũng đặt được): nếu login của người mở PR (GitHub xác thực)
# là người duy trì — có `@login` trong `.github/CODEOWNERS` của nhánh base — thì cả PR được miễn: người duy trì là bên cấp license và chịu
# trách nhiệm về mọi commit họ đưa vào (kể cả do agent của họ viết). `dependabot[bot]` cũng được miễn. Không có email nào
# của ai được ghi trong repo (repo công khai).
# Dùng: scripts/check-dco.sh <BASE> <HEAD> <PR_AUTHOR_LOGIN>
set -euo pipefail

base="${1:?usage: check-dco.sh <base> <head> <pr-author-login>}"
head="${2:?usage: check-dco.sh <base> <head> <pr-author-login>}"
author="${3:-}"

# Danh sách người duy trì = các `@login` trong CODEOWNERS của nhánh BASE (nhánh đích của PR), KHÔNG phải của PR: PR tự sửa
# danh sách của chính nó thì không được tự miễn. Quy tắc đọc: bỏ từ `#` tới hết dòng; chỉ nhận token đứng riêng giữa khoảng
# trắng có dạng `@login` (chữ, số, gạch ngang); token có `/` (team `@org/team`) và email không phải người duy trì; so
# không phân biệt hoa thường.
# Khởi động: nhánh base CHƯA có CODEOWNERS (pull request thêm nó lần đầu) thì chưa có danh sách người duy trì đáng tin để đối
# chiếu, nên bỏ qua kiểm. Chỉ áp khi base thiếu file — không bao giờ theo nội dung CODEOWNERS của chính PR; sau khi file có
# trên base, mọi PR đều bị kiểm.
if ! git cat-file -e "$base:.github/CODEOWNERS" 2>/dev/null; then
  echo "DCO: the base branch has no .github/CODEOWNERS yet (this pull request adds it): nothing to check against."
  exit 0
fi
maintainers="$(git show "$base:.github/CODEOWNERS" 2>/dev/null \
  | sed 's/#.*$//' | tr -s '[:space:]' '\n' \
  | grep -E '^@[A-Za-z0-9]([A-Za-z0-9-]*[A-Za-z0-9])?$' | tr -d '@' | sort -u || true)"
if [ -n "$author" ] && { [ "${author,,}" = "dependabot[bot]" ] || grep -Fxqi -- "$author" <<<"$maintainers"; }; then
  echo "DCO: pull request opened by maintainer/bot '$author': exempt."
  exit 0
fi

failed=0
while read -r sha; do
  [ -n "$sha" ] || continue
  # `%(trailers:...)` chỉ nhận trailer ở khối cuối của message, đúng chỗ `git commit -s` đặt.
  if ! git log -1 --format='%(trailers:key=Signed-off-by,valueonly)' "$sha" | grep -q '<[^>]*@[^>]*>'; then
    echo "::error::commit ${sha:0:12} ($(git log -1 --format=%s "$sha")) has no 'Signed-off-by: Name <email>' line. Fix: git rebase --signoff <base> && git push --force-with-lease (see CONTRIBUTING.md, 'Sign your commits')."
    failed=1
  fi
done < <(git rev-list --no-merges "$base..$head")

exit "$failed"
