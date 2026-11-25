#!/usr/bin/env bash
# Báo kết quả một workflow chạy theo lịch bằng issue (D-20261125-cloud-dev-friction): đỏ thì mở issue (đã có issue mở
# cùng tiêu đề thì chỉ thêm bình luận), xanh lại thì đóng. Gọi từ job `report` của workflow theo lịch.
#
#   scripts/nightly-issue.sh "<tiêu đề issue>" <kết quả của job: success|failure|cancelled|skipped> <link lần chạy>
#
# Cần `gh` (có sẵn trên runner), `GH_TOKEN` và `GH_REPO` trong môi trường; quyền `issues: write`.
# Kết quả khác success/failure (huỷ, bỏ qua, repo riêng tư nên job không chạy) thì không làm gì.
set -euo pipefail

title="${1:?thiếu tiêu đề}"
result="${2:?thiếu kết quả}"
run_url="${3:?thiếu link lần chạy}"

# Tìm issue mở có đúng tiêu đề (tìm theo từ khoá, rồi so chính xác).
existing="$(gh issue list --state open --search "in:title \"$title\"" --json number,title \
  --jq "map(select(.title == \"$title\")) | .[0].number // empty")"

case "$result" in
  failure)
    body="Lần chạy theo lịch đỏ: $run_url"
    if [ -n "$existing" ]; then
      gh issue comment "$existing" --body "$body (vẫn đỏ)"
    else
      gh issue create --title "$title" --body "$body

Issue này tự mở khi lần chạy theo lịch đỏ và tự đóng khi lần chạy kế tiếp xanh (D-20261125-cloud-dev-friction)."
    fi
    ;;
  success)
    if [ -n "$existing" ]; then
      gh issue close "$existing" --comment "Lần chạy theo lịch đã xanh lại: $run_url"
    fi
    ;;
  *)
    echo "nightly-issue: kết quả '$result', không làm gì"
    ;;
esac
