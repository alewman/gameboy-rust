#!/usr/bin/env python3
"""Compare the reference's and this board's suite results (scripts/suite_ref.py, gameboy-jobs).

    python3 scripts/suite_compare.py REF.jsonl RUST.jsonl

Every field must be equal (verdict, text, registers, T-cycles, frames, the shootout's verdict and
startup time); an error or a load failure counts as equal when both sides have one. Prints the
differences and per-suite pass counts (the reference's), and exits 1 on any difference.
"""

from __future__ import annotations

import json
import sys
from collections import defaultdict


def load(path: str) -> dict[str, dict]:
    out = {}
    for line in open(path):
        row = json.loads(line)
        if "error" in row:
            row["error"] = True
        if str(row.get("verdict", "")).startswith("FAIL (load"):
            row["verdict"] = "FAIL (load)"
        out[row["id"]] = row
    return out


def main() -> int:
    ref, rust = load(sys.argv[1]), load(sys.argv[2])
    bad = 0
    for key in sorted(set(ref) | set(rust)):
        if ref.get(key) != rust.get(key):
            bad += 1
            print(f"DIFF {key}\n  ref  {ref.get(key)}\n  rust {rust.get(key)}")
    tally: dict[str, list[int]] = defaultdict(lambda: [0, 0])
    for key, row in ref.items():
        suite = key.split("/")[0]
        tally[suite][1] += 1
        tally[suite][0] += bool(row.get("passed") or row.get("verdict") == "PASS")
    for suite, (won, total) in sorted(tally.items()):
        print(f"{suite:24s} {won:5d} of {total} pass on both" if not bad else f"{suite:24s} {won:5d} of {total} pass (reference)")
    print(f"{len(ref)} reference results, {len(rust)} rust results, {bad} differ")
    return 1 if bad else 0


if __name__ == "__main__":
    raise SystemExit(main())
