#!/usr/bin/env python3
"""Gộp kết quả mutation testing của các shard và so với ngưỡng (D-20261126-independent-review, 71-G9).

    scripts/mutants-report.py <thư mục chứa các mutants-<nhóm>-<k> đã tải về> [docs/quality/mutation.json]

Mỗi thư mục con là một artifact `mutants.out` của một shard (có `outcomes.json`). Chỉ dùng thư viện chuẩn.
Ghi báo cáo Markdown ra stdout (và vào $GITHUB_STEP_SUMMARY nếu có). Thoát mã 1 khi: một nhóm thiếu shard
(job bị huỷ / quá giờ, không được tính là đạt), hoặc số mutant sống vượt ngưỡng `baseline`. Ngưỡng `null`
(chưa đo) không làm đỏ, nhưng in số đo để điền vào file cấu hình.
"""
import json
import os
import sys
from pathlib import Path


def main() -> int:
    results_dir = Path(sys.argv[1])
    config_path = Path(sys.argv[2] if len(sys.argv) > 2 else "docs/quality/mutation.json")
    config = json.loads(config_path.read_text())
    totals = {}
    for unit in config["units"]:
        totals[unit["name"]] = {"shards": set(), "missed": [], "caught": 0, "timeout": 0, "unviable": 0}
    for outcomes_file in sorted(results_dir.glob("**/outcomes.json")):
        # Tên thư mục artifact: mutants-<nhóm>-<k>
        parts = outcomes_file.relative_to(results_dir).parts
        label = next((p for p in parts if p.startswith("mutants-")), None)
        if label is None:
            continue
        unit, _, shard = label.removeprefix("mutants-").rpartition("-")
        if unit not in totals:
            continue
        totals[unit]["shards"].add(shard)
        for outcome in json.loads(outcomes_file.read_text())["outcomes"]:
            summary = outcome["summary"]
            scenario = outcome["scenario"]
            if scenario == "Baseline":
                continue
            name = scenario["Mutant"]["name"]
            if summary == "MissedMutant":
                totals[unit]["missed"].append(name)
            elif summary == "CaughtMutant":
                totals[unit]["caught"] += 1
            elif summary == "Timeout":
                totals[unit]["timeout"] += 1
            elif summary == "Unviable":
                totals[unit]["unviable"] += 1

    failed = False
    lines = ["## Mutation testing", "", "| Nhóm | Bị bắt | Sống | Quá giờ | Không biên dịch | Ngưỡng sống | Kết luận |",
             "| ---- | -----: | ---: | ------: | --------------: | ----------: | -------- |"]
    measured = {}
    for unit in config["units"]:
        name = unit["name"]
        data = totals[name]
        missed = len(data["missed"])
        measured[name] = missed
        limit = config["baseline"].get(name)
        if len(data["shards"]) < unit["shards"]:
            verdict = f"THIẾU SHARD ({len(data['shards'])}/{unit['shards']})"
            failed = True
        elif limit is None:
            verdict = "chưa có ngưỡng"
        elif missed > limit:
            verdict = f"TỆ ĐI (+{missed - limit})"
            failed = True
        elif missed < limit:
            verdict = "tốt lên: hạ ngưỡng"
        else:
            verdict = "đạt"
        shown = "—" if limit is None else str(limit)
        lines.append(f"| {name} | {data['caught']} | {missed} | {data['timeout']} | {data['unviable']} | {shown} | {verdict} |")

    lines += ["", "Số mutant sống đo được (dán vào `baseline` của `docs/quality/mutation.json` nếu thấp hơn ngưỡng hiện có):",
              "", "```json", json.dumps(measured, indent=2), "```", ""]
    for unit in config["units"]:
        missed = totals[unit["name"]]["missed"]
        if missed:
            lines += [f"<details><summary>Mutant sống: {unit['name']} ({len(missed)})</summary>", ""]
            lines += [f"- `{name}`" for name in sorted(missed)]
            lines += ["", "</details>", ""]
    report = "\n".join(lines)
    print(report)
    summary_path = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary_path:
        with open(summary_path, "a", encoding="utf-8") as handle:
            handle.write(report + "\n")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
