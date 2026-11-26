#!/usr/bin/env bash
# Đăng website tài liệu của một bản phát hành lên nhánh `gh-pages` (D-20261126-docs-site-mdbook): thêm thư mục
# /<tag>/ (en, vi), cập nhật /latest/ khi đây là bản chính thức cao nhất, ghi versions.json và trang chuyển hướng gốc.
# Bản cũ giữ nguyên. Chạy bởi .github/workflows/docs-site.yml; chạy tay: GH_TOKEN, GITHUB_REPOSITORY, và thư mục đã dựng.
#
#   scripts/publish-docs-site.sh <tag> <built-dir>      <built-dir> chứa <tag>/{en,vi} do build-docs-site.py dựng
set -euo pipefail

tag="${1:?usage: publish-docs-site.sh <tag> <built-dir>}"
built="${2:?usage: publish-docs-site.sh <tag> <built-dir>}"
# `DOCS_REMOTE` (URL of any git remote) is for trying the script locally; CI builds the GitHub URL from the token.
if [ -n "${DOCS_REMOTE:-}" ]; then
  remote="$DOCS_REMOTE"
else
  repo="${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is not set}"
  remote="https://x-access-token:${GH_TOKEN:?GH_TOKEN is not set}@github.com/${repo}.git"
fi
site="$(mktemp -d)"

# Lấy nhánh gh-pages hiện có, hay bắt đầu nhánh mới (lần đầu).
if git ls-remote --exit-code --heads "$remote" gh-pages >/dev/null 2>&1; then
  git clone --quiet --depth 1 --branch gh-pages "$remote" "$site"
else
  git -C "$site" init --quiet --initial-branch gh-pages
fi
cd "$site"

rm -rf "$tag"
cp -r "$built/$tag" "$tag"

# Danh sách phiên bản đã đăng (thư mục dạng vX.Y.Z...), mới nhất trước; bản thử (-rc) có trong danh sách.
versions="$(find . -maxdepth 1 -type d -name 'v[0-9]*' -printf '%f\n' | sort -rV)"
# `latest` là bản chính thức cao nhất (không phải bản thử, không phải bản vá của dòng cũ).
latest="$(grep -v -- '-' <<<"$versions" | head -n 1 || true)"
if [ "$latest" = "$tag" ]; then
  rm -rf latest
  cp -r "$tag" latest
fi
jq -n --arg latest "${latest:-}" --argjson versions "$(jq -R . <<<"$versions" | jq -s .)" \
  '{latest: $latest, versions: $versions}' > versions.json
touch .nojekyll
cat > index.html <<'HTML'
<!doctype html>
<meta charset="utf-8">
<title>Autoxide documentation</title>
<meta http-equiv="refresh" content="0; url=latest/en/">
<p><a href="latest/en/">Autoxide documentation</a> · <a href="latest/vi/">Tài liệu tiếng Việt</a></p>
HTML

git add -A
if git diff --cached --quiet; then
  echo "nothing to publish"
  exit 0
fi
git -c user.name="github-actions[bot]" -c user.email="41898282+github-actions[bot]@users.noreply.github.com" \
  commit --quiet -m "docs: publish $tag"
git push --quiet "$remote" HEAD:gh-pages
echo "published $tag"
