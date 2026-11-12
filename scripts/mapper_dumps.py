#!/usr/bin/env python3
"""Find a No-Intro dump for each board class gameboy-python supports (read in place).

    python3 scripts/mapper_dumps.py [NOINTRO_DIR]

Classes: plain ROM, ROM+RAM, MBC1, MBC1+RAM+battery, MBC1M multicart (8 Mbit, logo again at
$40104), MBC2, MBC3, MBC3+RAM+battery, MBC3+timer (RTC), MBC30 (MBC3 with more than 32 KiB RAM),
MBC5, MBC5+RAM+battery, MBC5+rumble, MMM01 (a header with the logo in the last 32 KiB). Prints
`class<TAB>path` for the first (sorted) dump of each class, preferring USA/World/Europe releases.
"""

from __future__ import annotations

import os
import sys
import zipfile
from pathlib import Path

LOGO = bytes.fromhex("CEED6666CC0D000B03730083000C000D")


def classify(rom: bytes) -> list[str]:
    if len(rom) < 0x150:
        return []
    t, ram = rom[0x147], rom[0x149]
    out = []
    if len(rom) >= 0x8000 and rom[len(rom) - 0x8000 + 0x104 : len(rom) - 0x8000 + 0x114] == LOGO:
        if rom[len(rom) - 0x8000 + 0x147] in (0x0B, 0x0C, 0x0D):
            out.append("mmm01")
    if t == 0x00:
        out.append("rom")
    if t in (0x08, 0x09):
        out.append("rom+ram")
    if t in (1, 2, 3):
        out.append("mbc1")
        if t == 3:
            out.append("mbc1+ram+battery")
        if len(rom) == 0x100000 and rom[0x40104:0x40114] == LOGO:
            out.append("mbc1m")
    if t in (5, 6):
        out.append("mbc2")
    if t in (0x0F, 0x10, 0x11, 0x12, 0x13):
        out.append("mbc3")
        if t in (0x10, 0x13):
            out.append("mbc3+ram+battery")
        if t in (0x0F, 0x10):
            out.append("mbc3+timer")
        if ram in (4, 5) or (ram == 3 and False):
            out.append("mbc30")
    if t in (0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E):
        out.append("mbc5")
        if t == 0x1B:
            out.append("mbc5+ram+battery")
        if t in (0x1C, 0x1D, 0x1E):
            out.append("mbc5+rumble")
    return out


def main() -> int:
    base = Path(sys.argv[1] if len(sys.argv) > 1 else os.environ.get("NO_INTRO_DIR", "No-Intro"))
    found: dict[str, list[Path]] = {}
    for folder in ("Nintendo - Game Boy", "Nintendo - Game Boy Color"):
        for path in sorted((base / folder).glob("*.zip")):
            if any(tag in path.name for tag in ("(Beta", "(Proto", "(Demo", "(Pirate", "(Unl")):
                continue
            try:
                with zipfile.ZipFile(path) as z:
                    rom = z.read(z.infolist()[0])
            except (zipfile.BadZipFile, OSError):
                continue
            for c in classify(rom):
                found.setdefault(c, []).append(path)
    for c, paths in found.items():
        good = [p for p in paths if any(r in p.name for r in ("(USA", "(World", "(Europe"))] or paths
        print(f"{c}\t{len(paths)}\t{good[0]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
