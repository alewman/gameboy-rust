#!/usr/bin/env python3
"""Run the rung-4 suite jobs (scripts/suite_jobs.py) on the reference; one JSON line per job.

    external/venv/bin/python scripts/suite_ref.py JOBS --out FILE [--jobs N] [--only TEXT]

Each line: id, protocol, passed, text, registers, t, frames (the reference's ``RomResult``), or
for a shootout job its verdict and the startup time it used; "error" when the board raises. The
Rust side is ``gameboy-suite jobs``; ``scripts/suite_compare.py`` compares the two files.
"""

from __future__ import annotations

import argparse
import json
import sys
from multiprocessing import Pool
from pathlib import Path

REF = Path(__file__).resolve().parents[1] / "external" / "gameboy-python"
sys.path.insert(0, str(REF))
sys.path.insert(0, str(REF / "scripts"))

from gameboy_python import GameBoy, load_cartridge  # noqa: E402
from gameboy_python.boot import QUICKBOOT, find_boot_rom  # noqa: E402
from gameboy_python.machines import MACHINES  # noqa: E402
from gameboy_python.testrom import (  # noqa: E402
    run_blargg_ram,
    run_blargg_serial,
    run_microtest,
    run_mooneye,
)

STARTUP: dict[str, float] = {}


def boot_image(machine, boot):
    if boot == "skip":
        return None
    if boot == "quick":
        return QUICKBOOT.read_bytes()
    image, label = find_boot_rom(machine.boot_rom)
    if label != "real boot ROM":
        raise RuntimeError(f"no real boot ROM for {machine.boot_rom}")
    return image


def run(job: dict) -> dict:
    out = {"id": job["id"], "protocol": job["protocol"]}
    try:
        machine = MACHINES[job["machine"]]
        boot = boot_image(machine, job["boot"])
        rom = Path(job["rom"]).read_bytes()
        if job["protocol"] == "shootout":
            import shootout

            test = shootout._Test(job["id"], runtime=job["runtime"], rom=job["rom"],
                                  result=job["passes"], model=job["model"])
            test.rom = job["rom"]  # an absolute path: SHOOTOUT / "testroms" / it is it
            model = job["model"]
            if model not in STARTUP:
                STARTUP[model] = shootout.startup_time(model, False)
            name, _model, verdict = shootout.run((test, False, STARTUP))
            out.update(verdict=verdict, startup=STARTUP[model])
            return out
        holder: list[GameBoy] = []
        cart = load_cartridge(rom, clock=lambda: holder[0].t / 4194304 if holder else 0.0)
        gb = GameBoy(cart, machine, boot_rom=boot)
        holder.append(gb)
        protocol, frames = job["protocol"], job["frames"]
        if protocol == "mooneye":
            result = run_mooneye(gb, max_frames=frames)
        elif protocol == "blargg-serial":
            result = run_blargg_serial(gb, max_frames=frames)
        elif protocol == "blargg-ram":
            result = run_blargg_ram(gb, max_frames=frames)
        else:
            result = run_microtest(gb, frames, job.get("vram_flag", False))
        out.update(kind=result.protocol, passed=result.passed, text=result.text,
                   registers=list(result.registers), t=result.t_cycles, frames=result.frames)
    except Exception as exc:  # noqa: BLE001
        out.update(error=f"{type(exc).__name__}: {exc}")
    return out


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("jobs", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--jobs", dest="workers", type=int, default=10)
    parser.add_argument("--only")
    args = parser.parse_args()
    jobs = [json.loads(line) for line in args.jobs.read_text().splitlines()]
    if args.only:
        jobs = [j for j in jobs if args.only in j["id"]]
    with Pool(args.workers) as pool, args.out.open("w") as out:
        for result in pool.imap(run, jobs, chunksize=1):
            out.write(json.dumps(result, sort_keys=True) + "\n")
            out.flush()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
