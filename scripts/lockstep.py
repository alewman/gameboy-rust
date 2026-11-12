#!/usr/bin/env python3
"""The lockstep gate: run each case on both boards and compare the traces.

Each case of ``scripts/cases.tsv`` runs ``gameboy-trace`` (this board) and
``scripts/gameboy_trace.py`` on the reference's interpreter (``external/venv``, gameboy-python
at ``gameboy-rust-pin-1``), into ``external/traces/``, several at a time under ``nice``; the two
traces are compared line for line (one line per 70,224 T slice: CPU state, board time, timer,
PPU and DMA position, a CRC of every bus M-cycle and a CRC of the finished picture). A case
passes when they are identical; otherwise the first differing line is reported. Rerun the case
with ``--detail SLICE`` on both tools to see the M-cycles of that slice.

    lockstep.py [--only REGEX] [--jobs N] [--cases FILE] [--rust PATH] [--keep]
                [--reference-cache DIR] [--extra ARGS]

Case variables (``$GBTR``, ``$NOINTRO``, ``$INPUTS``) come from the environment, with the
defaults below. Stdlib only.
"""

from __future__ import annotations

import argparse
import os
import re
import shlex
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
DEFAULTS = {
    "GBTR": "/data/emu/gameboy-python/tests/test_roms/gbtr",
    "MTS": "/data/emu/gameboy-python/tests/test_roms/mts-20260714-0944-31510e1",
    "NOINTRO": "/data/emu/source/myrient.erista.me/files/No-Intro",
    "INPUTS": str(HERE / "inputs"),
}
PYTHON = Path(os.environ.get("GAMEBOY_REFERENCE_PYTHON", ROOT / "external" / "venv" / "bin" / "python"))


def cases(table: Path) -> list[tuple[str, int, str, list[str]]]:
    env = {**DEFAULTS, **{k: os.environ[k] for k in DEFAULTS if k in os.environ}}
    rows = []
    for line in table.read_text().splitlines():
        if not line or line.startswith("#"):
            continue
        name, slices, rom, args = (line.split("\t") + [""])[:4]
        for key, value in env.items():
            rom = rom.replace(f"${key}", value)
            args = args.replace(f"${key}", value)
        rows.append((name, int(slices), rom, shlex.split(args)))
    return rows


def one(case, rust: Path, traces: Path, keep: bool, cache: Path | None, extra: list[str],
        save_half: bool = False):
    name, slices, rom, args = case
    safe = re.sub(r"[^A-Za-z0-9._-]+", "_", name)
    ref, ours = traces / f"{safe}.ref", traces / f"{safe}.rust"
    if cache is not None:
        # --save-at is this board's alone (the reference runs uninterrupted): not part of the tag
        shown = re.sub(r"--save-at\s+\S+", "", " ".join(extra)).strip()
        tag = re.sub(r"[^A-Za-z0-9._-]+", "_", shown) or "plain"
        ref = cache / f"{safe}.{slices}.{tag}.ref"  # the reference's trace does not change
    common = [rom, "--slices", str(slices), *args, *extra]
    if save_half:
        common += ["--save-at", str(slices // 2)]
    began = time.monotonic()
    if cache is None or not ref.is_file():
        partial = ref.with_suffix(".part")
        reference = subprocess.run(
            ["nice", str(PYTHON), str(HERE / "gameboy_trace.py"), *common, "--out", str(partial)],
            capture_output=True,
            text=True,
            cwd="/",
            env={**os.environ, "PYPY_GC_NURSERY": os.environ.get("PYPY_GC_NURSERY", "1M")},
        )
        if reference.returncode:
            partial.unlink(missing_ok=True)
            return name, False, "reference failed: " + reference.stderr.strip()[-300:]
        partial.rename(ref)
    board = subprocess.run(
        ["nice", str(rust), *common, "--out", str(ours)], capture_output=True, text=True
    )
    if board.returncode:
        return name, False, "rust failed: " + board.stderr.strip()[-300:]
    seconds = time.monotonic() - began
    with ref.open() as a, ours.open() as b:
        for number, (left, right) in enumerate(zip(a, b)):
            if left != right:
                return (
                    name,
                    False,
                    f"line {number}: reference {left.strip()!r}\n      rust      {right.strip()!r}",
                )
    if ref.stat().st_size != ours.stat().st_size:
        return name, False, "one trace is longer"
    if not keep:
        if cache is None:
            ref.unlink()
        ours.unlink()
    return name, True, f"identical, {slices} slices ({seconds:.0f} s)"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--only", help="a regular expression on the case name")
    parser.add_argument("--jobs", type=int, default=10)
    parser.add_argument("--cases", type=Path, default=HERE / "cases.tsv")
    parser.add_argument("--rust", type=Path, help="a trace binary to judge instead of this tree's")
    parser.add_argument("--traces", type=Path, default=ROOT / "external" / "traces")
    parser.add_argument("--keep", action="store_true", help="keep the traces of identical cases")
    parser.add_argument("--reference-cache", type=Path, help="reuse the reference's traces here")
    parser.add_argument("--extra", default="", help="arguments added to every case (both sides); "
                        "use --extra=... for ones that start with --")
    parser.add_argument("--save-at-half", action="store_true",
                        help="the save-state round trip: this board saves and reloads its state "
                             "half way through each case (gameboy-trace --save-at)")
    args = parser.parse_args()
    rust = args.rust
    if rust is None:
        manifest = str(ROOT / "Cargo.toml")
        build = subprocess.run(["cargo", "build", "--release", "-q", "--manifest-path", manifest])
        if build.returncode:
            return 1
        rust = ROOT / "target" / "release" / "gameboy-trace"
    args.traces = args.traces.resolve()  # the reference runs from / (no stray imports)
    args.traces.mkdir(parents=True, exist_ok=True)
    if args.reference_cache:
        args.reference_cache = args.reference_cache.resolve()
        args.reference_cache.mkdir(parents=True, exist_ok=True)
    todo = [c for c in cases(args.cases) if not args.only or re.search(args.only, c[0])]
    todo.sort(key=lambda c: -c[1])  # the longest first, so the pool does not end on one long case
    extra = shlex.split(args.extra)
    failed = 0

    def run(case):
        return one(case, rust, args.traces, args.keep, args.reference_cache, extra,
                   args.save_at_half)

    with ThreadPoolExecutor(args.jobs) as pool:
        for name, ok, text in pool.map(run, todo):
            print(f"{name}: {text}", flush=True)
            failed += not ok
    print(f"{len(todo) - failed} of {len(todo)} cases identical")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
