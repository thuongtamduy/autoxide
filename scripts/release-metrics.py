#!/usr/bin/env python3
"""Bảng chỉ số của một bản phát hành (71-G9, ngưỡng cho 1.0 ở docs/RELEASING.md, mục "Chỉ số của mỗi bản phát hành").

    scripts/release-metrics.py --version X.Y.Z --sha <commit của tag> [--first-request time.json]
                               [--agent-eval results.json] [--repo owner/repo]

Đọc số từ nơi đã sinh ra nó, không tự đo lại: thời gian tới request đầu tiên (file JSON của
scripts/release-time-to-first-request.sh), job `upgrade` của CI trên đúng commit phát hành (G3), lần chạy gần nhất
của load.yml (p95), bộ eval agent (G4, file `{"tasks": N, "passed": M}`), và số đo người mới làm module đầu
(docs/quality/first-module-timing.json). Thiếu nguồn nào thì ghi rõ "chưa đo" / "chờ …", không bịa số. Ghi bảng Markdown
ra stdout (và $GITHUB_STEP_SUMMARY); thoát mã 3 khi có chỉ số ĐÃ ĐO mà không đạt ngưỡng (mã 0 nếu không; lỗi một nguồn chỉ làm chỉ số đó "chưa đo") (chỉ số chưa đo không làm đỏ:
ngưỡng 1.0 do chủ dự án và phiên điều phối kiểm tay lúc chốt 1.0). Cần `gh` đã đăng nhập cho các nguồn trên GitHub.
"""
import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

FIRST_REQUEST_LIMIT_SECONDS = 300
FIRST_MODULE_LIMIT_MINUTES = 30
AGENT_EVAL_MIN_RATIO = 0.90


def gh_json(*args: str):
    out = subprocess.run(["gh", *args], check=True, capture_output=True, text=True).stdout
    return json.loads(out) if out.strip() else None


def first_request(path: str | None):
    if not path or not Path(path).exists():
        return "chưa đo", None, "không có file số đo"
    try:
        data = json.loads(Path(path).read_text())
        seconds = data["seconds"]
        detail = ", ".join(f"{name} {value}s" for name, value in data["steps"].items())
    except (ValueError, KeyError, TypeError):
        return "chưa đo (file số đo rỗng hoặc hỏng: bước đo thời gian đã lỗi)", None, None
    ok = seconds < FIRST_REQUEST_LIMIT_SECONDS
    return f"{seconds:.0f} s (ngưỡng < {FIRST_REQUEST_LIMIT_SECONDS} s; {detail}; chỉ phần máy)", ok, None


def first_module():
    data = json.loads(Path("docs/quality/first-module-timing.json").read_text())
    parts, verdicts = [], []
    for kind, label in (("human", "người thật"), ("agent", "agent")):
        runs = [run for run in data["runs"] if run["kind"] == kind]
        if not runs:
            parts.append(f"{label}: chưa đo")
            continue
        last = runs[-1]
        ok = last["minutes"] < FIRST_MODULE_LIMIT_MINUTES
        verdicts.append(ok)
        parts.append(f"{label}: {last['minutes']} phút (bản {last['version']}, {last['date']}; ngưỡng < {FIRST_MODULE_LIMIT_MINUTES})")
    # Hai số riêng biệt: một loại đã đo mà trượt thì trượt; chưa đo thì không có kết luận.
    return "; ".join(parts), (all(verdicts) if verdicts else None)


def upgrade(repo: str, sha: str):
    runs = gh_json("run", "list", "--repo", repo, "--workflow", "ci.yml", "--commit", sha,
                   "--json", "databaseId,conclusion,status", "--limit", "5") or []
    done = [run for run in runs if run["status"] == "completed"]
    if not done:
        return "chờ CI của commit phát hành", None
    jobs = gh_json("run", "view", str(done[0]["databaseId"]), "--repo", repo, "--json", "jobs")["jobs"]
    ups = [job for job in jobs if job["name"].lower().startswith("upgrade")]
    if not ups:
        return "chờ G3 (không thấy job `upgrade`)", None
    passed = sum(1 for job in ups if job["conclusion"] == "success")
    failed = sum(1 for job in ups if job["conclusion"] == "failure")
    # Job bị bỏ qua (PR không chạm dữ liệu đã lưu), bị huỷ hay chưa có kết luận không phải một lần chạy xong: "chưa đo",
    # không tính là không đạt. Ở commit phát hành trên main phải chạy thật.
    if passed + failed < len(ups):
        return f"chưa đo: {len(ups) - passed - failed}/{len(ups)} job `upgrade` bị bỏ qua, huỷ hay chưa xong (đạt {passed}, hỏng {failed})", (False if failed else None)
    return f"{passed}/{len(ups)} job nâng cấp thành công (ngưỡng 100%)", failed == 0


def agent_eval(path: str | None):
    if not path or not Path(path).exists():
        return "chờ G4 (chưa có file kết quả eval)", None
    data = json.loads(Path(path).read_text())
    # Định dạng: docs/quality/agent-eval.schema.json. Thiếu trường bắt buộc là lỗi (không đoán).
    for key in ("model", "date", "tasks", "passed", "failed"):
        if key not in data:
            raise SystemExit(f"{path}: thiếu trường `{key}` (xem docs/quality/agent-eval.schema.json)")
    ratio = data["passed"] / max(data["tasks"], 1)
    cost = f", {data['cost_usd']:.2f} USD" if "cost_usd" in data else ""
    return (f"{data['passed']}/{data['tasks']} việc mẫu = {100 * ratio:.0f}% ({data['model']}, {data['date']}{cost}; "
            f"ngưỡng > {100 * AGENT_EVAL_MIN_RATIO:.0f}%)"), ratio > AGENT_EVAL_MIN_RATIO


def p95(repo: str):
    runs = gh_json("run", "list", "--repo", repo, "--workflow", "load.yml", "--branch", "main",
                   "--status", "completed", "--json", "databaseId,conclusion,createdAt,headSha", "--limit", "1") or []
    if not runs:
        return "chưa có lần chạy load.yml", None
    run = runs[0]
    where = f"lần chạy load.yml gần nhất ({run['createdAt'][:10]}, commit {run['headSha'][:7]})"
    if run["conclusion"] not in ("success", "failure"):
        return f"chưa đo: {where} kết thúc là `{run['conclusion']}`", None
    ok = run["conclusion"] == "success"
    return f"{where}: {'không tệ đi' if ok else 'TỆ ĐI hoặc lỗi'} (so với docs/quality/load-baseline.json)", ok


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--version", required=True)
    parser.add_argument("--sha", required=True)
    parser.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY", "thuongtamduy/autoxide"))
    parser.add_argument("--first-request")
    parser.add_argument("--agent-eval")
    args = parser.parse_args()

    def safe(label, source):
        """Một nguồn lỗi (mạng, `gh`, file hỏng) chỉ làm chỉ số đó thành "chưa đo", không làm hỏng cả bảng."""
        try:
            return source()
        except (Exception, SystemExit) as error:  # noqa: BLE001 - mọi lỗi của nguồn đều thành "chưa đo"
            reason = str(error).splitlines()[0][:160] if str(error) else type(error).__name__
            print(f"cảnh báo: nguồn `{label}` lỗi: {reason}", file=sys.stderr)
            return f"chưa đo: {reason}", None

    rows = []
    text, ok = safe("first-request", lambda: first_request(args.first_request)[:2])
    rows.append(("Tải về → request đầu tiên < 5 phút", text, ok))
    text, ok = safe("first-module", first_module)
    rows.append(("Người mới xong module đầu tiên < 30 phút", text, ok))
    text, ok = safe("upgrade", lambda: upgrade(args.repo, args.sha))
    rows.append(("Nâng cấp trong CI 100%", text, ok))
    text, ok = safe("agent-eval", lambda: agent_eval(args.agent_eval))
    rows.append(("Agent thật xong việc mẫu > 90%", text, ok))
    text, ok = safe("p95", lambda: p95(args.repo))
    rows.append(("p95 đường nóng không tệ đi", text, ok))

    mark = {True: "đạt", False: "KHÔNG ĐẠT", None: "chưa có kết luận"}
    lines = [f"## Chỉ số bản {args.version}", "", "| Chỉ số | Số đo | Kết luận |", "| ------ | ----- | -------- |"]
    lines += [f"| {name} | {text} | {mark[ok]} |" for name, text, ok in rows]
    lines.append("")
    report = "\n".join(lines)
    print(report)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as handle:
            handle.write(report + "\n")
    # Mã thoát riêng cho "có chỉ số đã đo mà không đạt" (3), để workflow phân biệt với script hỏng (mã khác).
    return 3 if any(ok is False for _, _, ok in rows) else 0


if __name__ == "__main__":
    sys.exit(main())
