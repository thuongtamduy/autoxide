#!/usr/bin/env python3
"""Dựng website tài liệu có phiên bản (D-20261126-docs-site-mdbook).

    scripts/build-docs-site.py <version> <out-dir> [--repo OWNER/REPO] [--ref GIT_REF]

Với mỗi ngôn ngữ (en, vi) tạo một cuốn mdBook từ các file Markdown sẵn có và chạy `mdbook build`; kết quả ở
<out-dir>/<version>/<lang>/. Tài liệu cho người ngoài là các file có cặp `X.md` / `X.vi.md` (luật `docs-translation`),
cộng `docs/ROADMAP.public.md` (chỉ tiếng Anh). Tài liệu nội bộ (tiếng Việt: lộ trình, quyết định, đặc tả bước) không lên site.
Chỉ dùng thư viện chuẩn của Python; cần `mdbook` trong PATH.
"""
import argparse
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SKIP_DIRS = {"target", "node_modules", ".git", "examples", "sdk", "fuzz", ".github", "changelog.d"}
# Chỉ EN (không có bản dịch) nhưng vẫn là tài liệu cho người ngoài.
EN_ONLY = ["docs/ROADMAP.public.md"]
TITLES = {"en": "Autoxide documentation", "vi": "Tài liệu Autoxide"}


def pairs():
    """(đường dẫn bản EN, đường dẫn bản VI) của mọi cặp tài liệu, tính từ gốc repo."""
    found = []
    for vi in sorted(ROOT.rglob("*.vi.md")):
        rel = vi.relative_to(ROOT)
        if SKIP_DIRS & set(rel.parts):
            continue
        en = rel.with_name(rel.name[: -len(".vi.md")] + ".md")
        if (ROOT / en).is_file():
            found.append((en, rel))
    return found


def first_heading(path: Path) -> str:
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith("# "):
            return line[2:].strip()
    return path.stem


def stage(lang: str, work: Path, repo: str, ref: str):
    """Chép tài liệu của ngôn ngữ vào work/src, giữ bố cục repo để liên kết tương đối vẫn đúng."""
    src = work / "src"
    shutil.rmtree(work, ignore_errors=True)
    src.mkdir(parents=True)
    chosen = [vi if lang == "vi" else en for en, vi in pairs()]
    if lang == "en":
        chosen += [Path(p) for p in EN_ONLY if (ROOT / p).is_file()]
    # Cả hai bản của mọi cặp được chép (liên kết chéo EN <-> VI trong tài liệu vẫn tồn tại), nhưng chỉ bản của ngôn ngữ
    # này vào mục lục.
    for en, vi in pairs():
        for rel in (en, vi):
            (src / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(ROOT / rel, src / rel)
    for rel in EN_ONLY:
        if (ROOT / rel).is_file():
            (src / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(ROOT / rel, src / rel)
    return src, chosen


LINK = re.compile(r"(\]\()([^)\s#]+)((?:#[^)\s]*)?)(\))")


def fix_links(src: Path, repo: str, ref: str):
    """Liên kết tương đối tới file không có trên site (mã nguồn, workflow, tài liệu nội bộ) trỏ sang GitHub theo đúng tag."""
    for md in src.rglob("*.md"):
        text = md.read_text(encoding="utf-8")

        def repl(m):
            target = m.group(2)
            if re.match(r"^[a-z][a-z0-9+.-]*:", target) or target.startswith("/"):
                return m.group(0)
            resolved = (md.parent / target).resolve()
            try:
                relative = resolved.relative_to(src.resolve())
            except ValueError:
                relative = None
            if relative is not None and resolved.exists():
                return m.group(0)
            # Đường dẫn tính từ gốc repo.
            repo_rel = (md.relative_to(src).parent / target)
            normalized = Path(*[p for p in repo_rel.parts if p != "."])
            parts = []
            for p in normalized.parts:
                if p == "..":
                    if parts:
                        parts.pop()
                else:
                    parts.append(p)
            kind = "tree" if (ROOT / Path(*parts)).is_dir() else "blob"
            return f"{m.group(1)}https://github.com/{repo}/{kind}/{ref}/{'/'.join(parts)}{m.group(3)}{m.group(4)}"

        new = LINK.sub(repl, text)
        if new != text:
            md.write_text(new, encoding="utf-8")


def reference_pages(src: Path, lang: str):
    """Tham chiếu API (từ docs/openapi.json) và tool MCP (docs/mcp-tools.json): Markdown sinh cùng lần dựng."""
    out = src / "reference"
    out.mkdir(exist_ok=True)
    spec = json.loads((ROOT / "docs/openapi.json").read_text(encoding="utf-8"))
    by_tag = {}
    for path, item in sorted(spec["paths"].items()):
        for method, op in item.items():
            if not isinstance(op, dict) or "responses" not in op:
                continue
            tag = (op.get("tags") or ["other"])[0]
            by_tag.setdefault(tag, []).append((path, method.upper(), op))
    lines = [
        "# REST API reference" if lang == "en" else "# Tham chiếu REST API",
        "",
        (
            f"Generated from `docs/openapi.json` (OpenAPI {spec['openapi']}, version {spec['info']['version']}). "
            "The full machine-readable contract is served at `/api/v1/openapi.json`."
            if lang == "en"
            else f"Sinh từ `docs/openapi.json` (OpenAPI {spec['openapi']}, phiên bản {spec['info']['version']}). "
            "Hợp đồng đầy đủ cho máy được phục vụ ở `/api/v1/openapi.json`. Mô tả endpoint bằng tiếng Anh."
        ),
        "",
    ]
    for tag in sorted(by_tag):
        lines += [f"## {tag}", ""]
        for path, method, op in by_tag[tag]:
            dep = " (deprecated)" if op.get("deprecated") else ""
            lines.append(f"- **`{method} {path}`**{dep}: {op.get('summary', '').strip()}")
        lines.append("")
    (out / "api.md").write_text("\n".join(lines), encoding="utf-8")
    tools = json.loads((ROOT / "docs/mcp-tools.json").read_text(encoding="utf-8"))["tools"]
    lines = [
        "# MCP tools" if lang == "en" else "# Tool MCP",
        "",
        "Generated from `docs/mcp-tools.json`." if lang == "en" else "Sinh từ `docs/mcp-tools.json`. Mô tả bằng tiếng Anh.",
        "",
    ]
    for tool in sorted(tools, key=lambda t: t["name"]):
        flags = tool.get("annotations", {})
        mark = "read-only" if flags.get("read_only") else ("destructive" if flags.get("destructive") else "changes data")
        lines.append(f"- **`{tool['name']}`** ({mark}): {tool.get('description', '').strip().splitlines()[0]}")
    lines.append("")
    (out / "mcp-tools.md").write_text("\n".join(lines), encoding="utf-8")
    return ["reference/api.md", "reference/mcp-tools.md"]


def summary(src: Path, chosen, extra, lang: str):
    """SUMMARY.md: README trước, rồi tài liệu theo thư mục, rồi tham chiếu sinh tự động."""
    groups = {}
    top = []
    for rel in chosen:
        name = rel.name
        if name.startswith("README") and len(rel.parts) == 1:
            continue
        (groups.setdefault(rel.parent.as_posix(), []) if len(rel.parts) > 1 else top).append(rel)
    readme = Path("README.vi.md" if lang == "vi" else "README.md")
    lines = ["# Summary", "", f"[{first_heading(src / readme)}]({readme.as_posix()})", ""]
    sections = [("Project" if lang == "en" else "Dự án", top)] + [
        (("Documents: " if lang == "en" else "Tài liệu: ") + k, v) for k, v in sorted(groups.items())
    ]
    for title, items in sections:
        if not items:
            continue
        lines += [f"# {title}", ""]
        for rel in sorted(items):
            lines.append(f"- [{first_heading(src / rel)}]({rel.as_posix()})")
        lines.append("")
    lines += ["# Reference" if lang == "en" else "# Tham chiếu", ""]
    for rel in extra:
        lines.append(f"- [{first_heading(src / rel)}]({rel})")
    (src / "SUMMARY.md").write_text("\n".join(lines) + "\n", encoding="utf-8")


def build(lang: str, version: str, out: Path, repo: str, ref: str):
    work = out.parent / f".work-{lang}"
    src, chosen = stage(lang, work, repo, ref)
    extra = reference_pages(src, lang)
    fix_links(src, repo, ref)
    summary(src, chosen, extra, lang)
    shutil.copy(ROOT / "docs/site/version-switcher.js", work / "version-switcher.js")
    (work / "book.toml").write_text(
        f"""[book]
title = "{TITLES[lang]} {version}"
language = "{lang}"
src = "src"

[build]
build-dir = "book"
create-missing = false

[output.html]
git-repository-url = "https://github.com/{repo}"
additional-js = ["version-switcher.js"]

[output.html.search]
enable = true
""",
        encoding="utf-8",
    )
    subprocess.run(["mdbook", "build"], cwd=work, check=True)
    dest = out / lang
    shutil.rmtree(dest, ignore_errors=True)
    shutil.copytree(work / "book", dest)
    shutil.rmtree(work)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("version")
    parser.add_argument("out")
    parser.add_argument("--repo", default="thuongtamduy/autoxide")
    parser.add_argument("--ref", default=None)
    args = parser.parse_args()
    ref = args.ref or ("v" + args.version.lstrip("v") if re.match(r"^v?\d", args.version) else "main")
    out = Path(args.out) / args.version
    out.mkdir(parents=True, exist_ok=True)
    for lang in ("en", "vi"):
        build(lang, args.version, out, args.repo, ref)
    print(f"built {args.version} into {out}")


if __name__ == "__main__":
    sys.exit(main())
