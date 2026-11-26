#!/usr/bin/env bash
# Đăng website tài liệu của một bản phát hành lên nhánh `gh-pages` (D-20261126-docs-site-mdbook): thêm thư mục
# /<tag>/ (en, vi), cập nhật /latest/ khi đây là bản chính thức cao nhất, ghi versions.json và trang chuyển hướng gốc.
# Bản cũ giữ nguyên. Chạy bởi .github/workflows/docs-site.yml.
#
#   scripts/publish-docs-site.sh <tag> <built-dir> <site-dir>
#
# <built-dir> chứa <tag>/{en,vi} do build-docs-site.py dựng. <site-dir> là một bản checkout git CÓ quyền đẩy (workflow dùng
# actions/checkout với persist-credentials: không đưa token vào URL hay dòng lệnh): đang ở nhánh `gh-pages`, hoặc ở nhánh khác
# khi `gh-pages` chưa có (lần đầu) — khi đó script tạo nhánh gh-pages rỗng (orphan). Đẩy bằng `git push origin`.
set -euo pipefail

tag="${1:?usage: publish-docs-site.sh <tag> <built-dir> <site-dir>}"
built="$(cd "${2:?usage: publish-docs-site.sh <tag> <built-dir> <site-dir>}" && pwd)"
cd "${3:?usage: publish-docs-site.sh <tag> <built-dir> <site-dir>}"

if [ "$(git rev-parse --abbrev-ref HEAD)" != gh-pages ]; then
  git switch --orphan gh-pages
  git rm -rfq --ignore-unmatch . >/dev/null
  git clean -fdxq
fi

# Một lần làm: thêm thư mục của tag, tính lại latest / versions.json, commit, đẩy. Đẩy thất bại vì người khác vừa đẩy (hai bản phát
# hành sát nhau) thì lấy lại gh-pages mới nhất rồi làm lại từ đầu trên đó — cả hai thư mục phiên bản đều vào.
publish_once() {
  # Trong `if publish_once` thì `set -e` không có tác dụng: mỗi lệnh phải tự `|| return 1`, không thì lỗi bị bỏ qua và
  # versions.json hỏng vẫn được commit.
  rm -rf "$tag" || return 1
  cp -r "$built/$tag" "$tag" || return 1

  # Danh sách phiên bản đã đăng (thư mục dạng vX.Y.Z...), mới nhất trước; bản thử (-rc) có trong danh sách.
  versions="$(find . -maxdepth 1 -type d -name 'v[0-9]*' -printf '%f\n' | sort -rV)" || return 1
  # `latest` là bản chính thức cao nhất (không phải bản thử, không phải bản vá của dòng cũ).
  latest="$(grep -v -- '-' <<<"$versions" | head -n 1 || true)"
  if [ "$latest" = "$tag" ]; then
    rm -rf latest || return 1
    cp -r "$tag" latest || return 1
  fi
  versions_json="$(jq -R . <<<"$versions" | jq -s .)" || return 1
  jq -n --arg latest "${latest:-}" --argjson versions "$versions_json" \
    '{latest: $latest, versions: $versions}' > versions.json || return 1
  touch .nojekyll || return 1
  cat > index.html <<'HTML'
<!doctype html>
<meta charset="utf-8">
<title>Autoxide documentation</title>
<meta http-equiv="refresh" content="0; url=latest/en/">
<p><a href="latest/en/">Autoxide documentation</a> · <a href="latest/vi/">Tài liệu tiếng Việt</a></p>
HTML

  git add -A || return 1
  if git diff --cached --quiet; then
    echo "nothing to publish"
    return 0
  fi
  git -c user.name="github-actions[bot]" -c user.email="41898282+github-actions[bot]@users.noreply.github.com" \
    commit --quiet -m "docs: publish $tag" || return 1
  git push --quiet origin HEAD:gh-pages || return 1
}

for attempt in 1 2 3; do
  if publish_once; then
    echo "published $tag"
    exit 0
  fi
  echo "push to gh-pages failed (attempt $attempt): fetching the latest gh-pages and trying again" >&2
  git fetch --quiet origin gh-pages
  git reset --quiet --hard origin/gh-pages
done
echo "could not publish $tag after 3 attempts" >&2
exit 1
