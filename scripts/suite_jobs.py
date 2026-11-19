#!/usr/bin/env python3
"""Write the rung-4 suite jobs (JSON Lines) that both boards run and compare verdict by verdict.

    external/venv/bin/python scripts/suite_jobs.py --out external/suite-jobs.jsonl

Every ROM of each suite the reference judges, not only the ones it passes: Mooneye acceptance,
misc and emulator-only (on DMG-B from skip-boot, on CGB E from skip-boot, misc from the real CGB
boot ROM, the boot_* tests from the real boot ROMs and quickboot), blargg (cpu_instrs,
instr_timing, mem_timing, mem_timing-2, oam_bug, interrupt_time, dmg_sound, cgb_sound), SameSuite
and AGE on the CGB, GBMicrotest (poweron_* from the real DMG boot ROM), the DMG-0 boot tests and
gbdev's GB Emulator Shootout list (its tests, expected pictures and runtimes, read as data the way
the reference's scripts/shootout.py reads them). Runtimes and frame budgets are the reference
harnesses'. Each job: id, rom, machine, boot (skip/real/quick), protocol, frames, and for the
shootout model, runtime, passes and fails (expected pictures).
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

REF = Path(__file__).resolve().parents[1] / "external" / "gameboy-python"
sys.path.insert(0, str(REF))
sys.path.insert(0, str(REF / "scripts"))

from tests.mooneye_meta import ROMS  # noqa: E402

GBTR = REF / "tests/test_roms/gbtr"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    jobs: list[dict] = []

    def add(id_, rom, protocol, frames, machine="dmg", boot="skip", **extra):
        jobs.append({"id": id_, "rom": str(rom), "machine": machine, "boot": boot,
                     "protocol": protocol, "frames": frames, **extra})

    mts = ROMS.parent
    for group in ("acceptance", "emulator-only", "misc"):
        for rom in sorted((mts / group).rglob("*.gb")):
            name = rom.relative_to(mts).with_suffix("").as_posix()
            add(f"mooneye-dmg/{name}", rom, "mooneye", 1500)
            add(f"mooneye-cgb/{name}", rom, "mooneye", 1500, machine="cgb")
            if group == "misc":
                add(f"mooneye-cgb-real/{name}", rom, "mooneye", 1500, machine="cgb", boot="real")
            if "boot_" in rom.name:
                add(f"mooneye-dmg-real/{name}", rom, "mooneye", 1500, boot="real")
                add(f"mooneye-dmg-quick/{name}", rom, "mooneye", 1500, boot="quick")
                add(f"mooneye-mgb-real/{name}", rom, "mooneye", 1500, machine="mgb", boot="real")
                add(f"mooneye-dmg0-real/{name}", rom, "mooneye", 1500, machine="dmg-0", boot="real")
    b = GBTR / "blargg"
    for rom in sorted((b / "cpu_instrs/individual").glob("*.gb")):
        add(f"blargg/{rom.stem}", rom, "blargg-serial", 2000)
    add("blargg/cpu_instrs", b / "cpu_instrs/cpu_instrs.gb", "blargg-serial", 6000)
    add("blargg/instr_timing", b / "instr_timing/instr_timing.gb", "blargg-serial", 6000)
    add("blargg/mem_timing", b / "mem_timing/mem_timing.gb", "blargg-serial", 6000)
    add("blargg/mem_timing-cgb", b / "mem_timing/mem_timing.gb", "blargg-serial", 6000, machine="cgb")
    add("blargg/mem_timing-2", b / "mem_timing-2/mem_timing.gb", "blargg-ram", 6000)
    for rom in sorted((b / "oam_bug/rom_singles").glob("*.gb")):
        add(f"blargg/oam_bug/{rom.stem}", rom, "blargg-ram", 6000)
    for rom in sorted((b / "interrupt_time").glob("*.gb")):
        add(f"blargg/interrupt_time/{rom.stem}", rom, "blargg-ram", 6000, machine="cgb")
    for rom in sorted((b / "dmg_sound/rom_singles").glob("*.gb")):
        add(f"blargg/dmg_sound/{rom.stem}", rom, "blargg-ram", 3000)
    for rom in sorted((b / "cgb_sound/rom_singles").glob("*.gb")):
        add(f"blargg/cgb_sound/{rom.stem}", rom, "blargg-ram", 3000, machine="cgb")
    for suite in ("same-suite", "age-test-roms"):
        for rom in sorted((GBTR / suite).rglob("*.gb")):
            name = rom.relative_to(GBTR).as_posix()
            add(f"cgb-suites/{name}", rom, "mooneye", 600, machine="cgb")
            add(f"dmg-suites/{name}", rom, "mooneye", 600)
    for rom in sorted((GBTR / "gbmicrotest").glob("*.gb")):
        real = rom.name.startswith("poweron_")
        flagged = rom.name in ("ppu_spritex_vs_scx.gb", "004-tima_boot_phase.gb")
        add(f"gbmicrotest/{rom.name}", rom, "microtest", 30 + (352 if real else 0),
            boot="real" if real else "skip", vram_flag=flagged)
    import shootout  # the reference's own loader (data only)

    for test in shootout.load_tests():
        if test.model == "SGB":
            continue
        shoot = shootout.SHOOTOUT / "testroms"
        passes = [str(shoot / r) for r in test.results]
        fails = [str(shoot / (r.rsplit(".", 1)[0] + ".fail.png")) for r in test.results]
        add(f"shootout/{test.name}", shoot / test.rom, "shootout", 0,
            machine="cgb" if test.model == "CGB" else "dmg-b", boot="real",
            model=test.model, runtime=test.runtime, passes=passes, fails=fails,
            startup_rom=str(shootout.SHOOTOUT / "startup_time_test.gb"),
            startup_png=str(shootout.SHOOTOUT / "startup_time_test.png"))
    with args.out.open("w") as out:
        for job in jobs:
            out.write(json.dumps(job) + "\n")
    print(f"{len(jobs)} jobs")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
