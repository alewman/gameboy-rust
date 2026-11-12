#!/usr/bin/env python3
"""Seeded faults: build the board with one single-line mutation at a time and check that the
lockstep gate catches each (a fault is caught when any case's trace differs from the reference's).

    python3 scripts/seeded_faults.py [--cases FILE ...] [--only NAME] [--jobs N]

The mutations are applied to a copy of the tree (a git worktree at external/faults-tree, built
into external/faults-target), never to this checkout. The reference's traces come from
--reference-cache external/refcache (run the gate once first). Stdlib only.
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TREE = ROOT / "external" / "faults-tree"
TARGET = ROOT / "external" / "faults-target"

# (name, file, old line text, new line text): each old text must occur exactly once.
FAULTS = [
    ("ppu: OAM scan height", "src/ppu.rs",
     "let height = if self.lcdc & 4 != 0 { 16 } else { 8 };\n        if y <= self.ly",
     "let height = if self.lcdc & 4 != 0 { 16 } else { 7 };\n        if y <= self.ly"),
    ("ppu: first object discount", "src/ppu.rs",
     "pub const FIRST_OBJECT_DISCOUNT: i32 = 3;", "pub const FIRST_OBJECT_DISCOUNT: i32 = 2;"),
    ("ppu: mode read lag", "src/ppu.rs",
     "pub const MODE0_READ_LAG: i32 = 1;", "pub const MODE0_READ_LAG: i32 = 2;"),
    ("ppu: LY=LYC flag at line 153", "src/ppu.rs",
     "return if self.dot < 8 { LAST_LINE } else { 0 };", "return if self.dot < 12 { LAST_LINE } else { 0 };"),
    ("ppu: VBlank IF lateness", "src/ppu.rs",
     "pub const VBLANK_IF_LATE: i32 = 4;", "pub const VBLANK_IF_LATE: i32 = 0;"),
    ("ppu: window line counter", "src/ppu.rs",
     "self.window_line = (self.window_line + 1) & 255;\n        }\n        self.window_used = false;",
     "self.window_line = (self.window_line + 2) & 255;\n        }\n        self.window_used = false;"),
    ("pixels: DMG BGP glitch", "src/pixels.rs",
     "self.framebuffer[i] = (((p.bgp | self.last_bgp) >> (colour * 2)) & 3) as u16;",
     "self.framebuffer[i] = ((p.bgp >> (colour * 2)) & 3) as u16;"),
    ("pixels: CGB register delay", "src/pixels.rs",
     "pub const CGB_REG_DELAY: usize = 2;", "pub const CGB_REG_DELAY: usize = 1;"),
    ("dma: start delay", "src/dma.rs",
     "self.countdown = 2;", "self.countdown = 1;"),
    ("dma: length", "src/dma.rs",
     "if self.index == 160 {", "if self.index == 159 {"),
    ("board: DMA bus conflict", "src/board.rs",
     "return self.ppu.dma.byte; // bus conflict", "return self.peek(address, cpu); // bus conflict"),
    ("timer: TIMA bit for TAC=01", "src/timer.rs",
     "pub const TIMER_BITS: [u32; 4] = [9, 3, 5, 7];", "pub const TIMER_BITS: [u32; 4] = [9, 4, 5, 7];"),
    ("timer: reload cycle write", "src/timer.rs",
     "            return; // TMA wins during the reload cycle", "            // TMA wins during the reload cycle"),
    ("timer: DIV write edge", "src/timer.rs",
     "if Self::signal(self.counter, self.tac) != 0 {\n            self.increment();\n        }\n        self.counter = 0;",
     "if Self::signal(self.counter, self.tac) == 2 {\n            self.increment();\n        }\n        self.counter = 0;"),
    ("cart: MBC1 bank 0 maps to 1", "src/cartridge/mod.rs",
     "(value & 0x1F) as usize\n                    } else {\n                        1\n                    };\n                } else if a < 0x6000 {\n                    m.bank2",
     "(value & 0x1F) as usize\n                    } else {\n                        0\n                    };\n                } else if a < 0x6000 {\n                    m.bank2"),
    ("cart: MBC5 high bank bit", "src/cartridge/mod.rs",
     "m.romb1 = (value & 1) as usize;", "m.romb1 = 0;"),
    ("cart: MBC3 RTC seconds carry", "src/cartridge/mod.rs",
     "        if live[0] == 59 {\n            live[0] = 0;",
     "        if live[0] == 58 {\n            live[0] = 0;"),
    ("serial: clock bit", "src/serial.rs",
     "pub const SERIAL_CLOCK_BIT: u32 = 7;", "pub const SERIAL_CLOCK_BIT: u32 = 8;"),
]


def run(cmd, **kw) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, capture_output=True, text=True, **kw)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--cases", action="append", type=Path)
    parser.add_argument("--only")
    parser.add_argument("--jobs", type=int, default=10)
    args = parser.parse_args()
    tables = args.cases or [ROOT / "scripts" / "cases.tsv", ROOT / "scripts" / "cases-rung5.tsv"]
    if not TREE.exists():
        run(["git", "-C", str(ROOT), "worktree", "add", "--detach", str(TREE), "HEAD"], check=True)
    run(["git", "-C", str(TREE), "checkout", "--detach", run(["git", "-C", str(ROOT), "rev-parse", "HEAD"]).stdout.strip()], check=True)
    caught = 0
    tried = 0
    for name, file, old, new in FAULTS:
        if args.only and args.only not in name:
            continue
        path = TREE / file
        run(["git", "-C", str(TREE), "checkout", "--", "."], check=True)
        text = path.read_text()
        if text.count(old) != 1:
            print(f"{name}: SKIPPED (the old text occurs {text.count(old)} times)")
            continue
        path.write_text(text.replace(old, new))
        cargo = TREE / "Cargo.toml"  # the path dependencies, from the worktree's place
        cargo.write_text(cargo.read_text().replace('path = "../', f'path = "{ROOT.parent}/'))
        build = run(["cargo", "build", "--release", "-q", "--bin", "gameboy-trace",
                     "--manifest-path", str(TREE / "Cargo.toml"), "--target-dir", str(TARGET)])
        if build.returncode:
            print(f"{name}: BUILD FAILED {build.stderr[-300:]}")
            continue
        tried += 1
        found = []
        for table in tables:
            gate = run([sys.executable, str(ROOT / "scripts" / "lockstep.py"), "--cases", str(table),
                        "--rust", str(TARGET / "release" / "gameboy-trace"), "--jobs", str(args.jobs),
                        "--reference-cache", str(ROOT / "external" / "refcache"),
                        "--traces", str(ROOT / "external" / "faults-traces")])
            found += [line.split(":")[0] for line in gate.stdout.splitlines()
                      if ": " in line and ": identical" not in line and not line.startswith(" ")]
        caught += bool(found)
        print(f"{name}: {'CAUGHT' if found else 'MISSED'} ({len(found)} cases{': ' + ', '.join(found[:3]) if found else ''})", flush=True)
    run(["git", "-C", str(TREE), "checkout", "--", "."])
    print(f"{caught} of {tried} faults caught")
    return 0 if caught == tried else 1


if __name__ == "__main__":
    raise SystemExit(main())
