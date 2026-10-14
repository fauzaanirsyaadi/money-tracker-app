#!/usr/bin/env python3
"""Report (and optionally enforce) the size of the built web bundle.

    bundle_size.py [build_dir] [--max-js-gzip-kb N] [--max-total-gzip-kb N]

Sizes are what ships in the APK's assets: raw bytes and gzip -9 (a stand-in for
what a compressing transport would send; assets in an APK are served locally).
"""

from __future__ import annotations

import gzip
import sys
from pathlib import Path


def collect(build: Path) -> dict[str, list[tuple[str, int, int]]]:
    groups: dict[str, list[tuple[str, int, int]]] = {"js": [], "css": [], "other": []}
    for path in sorted(build.rglob("*")):
        if not path.is_file():
            continue
        data = path.read_bytes()
        entry = (str(path.relative_to(build)), len(data), len(gzip.compress(data, 9)))
        kind = {".js": "js", ".css": "css"}.get(path.suffix, "other")
        groups[kind].append(entry)
    return groups


def main(argv: list[str]) -> int:
    args = [a for a in argv[1:] if not a.startswith("--")]
    options = {a.split("=")[0]: a.split("=")[1] for a in argv[1:] if a.startswith("--") and "=" in a}
    build = Path(args[0]) if args else Path(__file__).resolve().parent.parent / "build"
    if not build.is_dir():
        print(f"bundle size: {build} does not exist (run `pnpm build` first)")
        return 2

    groups = collect(build)
    totals = {k: (sum(e[1] for e in v), sum(e[2] for e in v)) for k, v in groups.items()}
    for kind in ("js", "css", "other"):
        raw, packed = totals[kind]
        print(f"{kind.upper():5} {len(groups[kind]):3} files  {raw / 1024:8.1f} KB raw  {packed / 1024:8.1f} KB gzip")
    all_raw = sum(t[0] for t in totals.values())
    all_gz = sum(t[1] for t in totals.values())
    print(f"TOTAL {sum(len(v) for v in groups.values()):3} files  {all_raw / 1024:8.1f} KB raw  {all_gz / 1024:8.1f} KB gzip")
    print("largest JS files:")
    for name, raw, packed in sorted(groups["js"], key=lambda e: -e[2])[:5]:
        print(f"  {packed / 1024:7.1f} KB gzip  {raw / 1024:7.1f} KB raw  {name}")

    failed = False
    js_limit = options.get("--max-js-gzip-kb")
    if js_limit and totals["js"][1] / 1024 > float(js_limit):
        print(f"FAIL: JS is {totals['js'][1] / 1024:.1f} KB gzip, budget {js_limit} KB")
        failed = True
    total_limit = options.get("--max-total-gzip-kb")
    if total_limit and all_gz / 1024 > float(total_limit):
        print(f"FAIL: bundle is {all_gz / 1024:.1f} KB gzip, budget {total_limit} KB")
        failed = True
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
