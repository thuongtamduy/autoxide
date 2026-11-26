#!/usr/bin/env python3
"""So kết quả đo tải với số đo gốc (71-G9): chặn khi p95 của một đường nóng tệ đi quá ngưỡng.

    scripts/load-compare.py <results.jsonl> <sqlite|postgres> [docs/quality/load-baseline.json]

`results.jsonl`: mỗi dòng một bài đo (do `LOADTEST_JSON_OUT` của scripts/loadtest.sh ghi). Một đường bị coi là tệ đi khi
p95 > gốc × `tolerance` + `floor_ms` (cộng sàn để đường vài trăm micro giây không đỏ vì nhiễu), hoặc tỉ lệ 2xx dưới
99% (đo ra lỗi thì số p95 vô nghĩa), hoặc thiếu bài đo mà baseline có. Gốc `null` (chưa đo) không làm đỏ nhưng in số đo
để điền vào file. Thoát mã 1 khi có đường tệ đi. Ghi báo cáo Markdown ra stdout (và $GITHUB_STEP_SUMMARY). Chỉ thư viện chuẩn.
"""
import json
import os
import sys
from pathlib import Path


def main() -> int:
    results_path, backend = Path(sys.argv[1]), sys.argv[2]
    baseline_path = Path(sys.argv[3] if len(sys.argv) > 3 else "docs/quality/load-baseline.json")
    baseline = json.loads(baseline_path.read_text())
    tolerance, floor = baseline["tolerance"], baseline["floor_ms"]
    wanted = baseline["p95_ms"][backend]
    measured = {}
    for line in results_path.read_text().splitlines():
        if line.strip():
            row = json.loads(line)
            measured[row["name"]] = row

    failed = False
    lines = [f"## Load test — {backend}", "",
             "| Đường nóng | p95 ms | Gốc ms | Giới hạn ms | 2xx | Kết luận |",
             "| ---------- | -----: | -----: | ----------: | --: | -------- |"]
    for name, base in wanted.items():
        row = measured.get(name)
        if row is None:
            lines.append(f"| {name} | — | {"—" if base is None else base} | — | — | THIẾU BÀI ĐO |")
            failed = True
            continue
        p95, ok = row["p95_ms"], row["ok_ratio"]
        limit = None if base is None else base * tolerance + floor
        if ok < 0.99:
            verdict = "LỖI (dưới 99% thành công)"
            failed = True
        elif limit is None:
            verdict = "chưa có gốc"
        elif p95 > limit:
            verdict = f"TỆ ĐI ({p95 / base:.2f}× gốc)"
            failed = True
        else:
            verdict = "đạt"
        shown_limit = "—" if limit is None else f"{limit:.2f}"
        lines.append(f"| {name} | {p95:.2f} | {"—" if base is None else base} | {shown_limit} | {100 * ok:.1f}% | {verdict} |")
    suggestion = {name: round(row["p95_ms"], 2) for name, row in measured.items() if name in wanted}
    lines += ["", f"Số đo p95 của lần chạy này (điền vào `p95_ms.{backend}` khi đặt hay cập nhật gốc):", "",
              "```json", json.dumps(suggestion, indent=2), "```", ""]
    report = "\n".join(lines)
    print(report)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as handle:
            handle.write(report + "\n")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
