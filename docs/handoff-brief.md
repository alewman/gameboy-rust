> Historical: the brief this board was built from, 2026-11-12; current state in [status.md](status.md).

# Handoff brief: gameboy-rust

## The request

Aubrey, 2026-11-12: the family's Rust path for the Game Boy — sm83-rust
(done: `/data/emu/sm83-rust`, private `alewman/sm83-rust` 105b8c1, every
rung passed) → gameboy-rust (this brief) and gb-apu-rust (another agent,
same time) → gameboy-rust-fast → a play-gb browser page.

## What you are inheriting

`gameboy-python` (`/data/emu/gameboy-python`, PRIVATE `alewman/gameboy-python`)
is the family's reference DMG/CGB board. Transcribe it at the tag
**`gameboy-rust-pin-1`** (commit `efdde44`, 2026-11-12). Its master keeps
moving (another session is fixing Color games); fixes after the pin queue for
`pin-2`; do not chase master.

State at the pin: Mealybug Tearoom DMG 24/24 and CGB-D 20/20, Gambatte
hardware tests DMG 1,695 / CGB 2,864 (`tests/gambatte_*_pass.txt`), GB
Emulator Shootout 256/258, Mooneye, blargg, SameSuite, cgb-acid2/hell,
strikethrough; the picture is drawn by `pixels.py` (docboy-derived, MIT,
see its docstring) running `DRAW_LAG` dots behind the timing PPU `ppu.py`.

The CPU exists: crate `sm83` at `/data/emu/sm83-rust`, transcribed from
sm83-python `95f2b8b`, which is the revision gameboy-python pins. Read its
`docs/status.md` "how a board drives it" (Bus trait, `request_interrupt(bit,
halt_sees, late)`, `if_`, `stop`, `idle`). Depend on it by path.

Sound: the board uses gb-apu-python `f0a7f48`. A gb-apu-rust crate is being
built in parallel at `/data/emu/gb-apu-rust` (brief `/data/emu/gb-apu-rust-HANDOFF.md`).
Build the board with the APU behind the same interface the Python board
uses; adopt the crate by path when its ladder passes (check its
`docs/status.md`); until then a stub that answers register reads as the
Python core would is not acceptable for lockstep — instead exclude APU
register reads from the gate until the crate lands, and say so.

Read before writing any Rust, in this order: `docs/start-here.md`,
`docs/timing.md` (the board advances inside the CPU's M-cycles; system
counter; interrupts; PPU dot by dot; OAM DMA; OAM bug; APU clock; CGB double
speed and speed switch), `docs/undocumented-behavior.md`, `docs/cartridge.md`,
`docs/validation.md`; then `src/gameboy_python/` module by module
(`board.py`, `ppu.py`, `pixels.py`, `dma.py`, `timer.py`, `joypad.py`,
`serial.py`, `machines.py`, `state.py`, the cartridges). Skip `frontend.py`
(pygame window) — a minimal SDL2 `play` window like a2600-rust-fast's comes
later, not in this brief.

## The task, in rungs (do not reorder)

1. **Skeleton:** board + sm83 + ROM-only/MBC1; blargg cpu_instrs passes
   natively.
2. **Lockstep gate:** run the Python board and the Rust board side by side
   and compare per M-cycle (address, value, direction, board time `t`) and
   the finished frame picture. Build the gate first and keep it as the
   arbiter, as megadrive-rust and famicom-rust did
   (`/data/emu/megadrive-rust/scripts/lockstep.py`, its cases.tsv). Cases:
   test ROMs plus commercial dumps from the local No-Intro sets (read in
   place, never committed: `/data/emu/source/myrient.erista.me/files/No-Intro`).
3. **PPU (timing + pixels):** Mealybug DMG 24/24 and CGB-D 20/20 pictures
   exact; cgb-acid2, cgb-acid-hell, dmg-acid2 exact; the Gambatte pass lists
   reproduce exactly (same verdicts, every ROM, DMG and CGB); lockstep
   identical on all cases.
4. **Everything else the Python board passes:** Mooneye (DMG + CGB), blargg,
   SameSuite, gbmicrotest, the Shootout score (scripts/shootout.py's method).
5. **Cartridges:** every mapper gameboy-python supports, each under the
   lockstep gate on a dump that uses it (MBC3 RTC included).
6. **Machines and save states:** every row in `machines.py`; save/load state.
7. **APU:** adopt gb-apu-rust; Gambatte audio tests and the board's audio
   test outputs identical; APU reads back in the gate.

Seed faults at the end (~15 single-line mutations across PPU, DMA, timer,
cart) and report how many the gate catches.

## Traps

- The board advances *inside* the CPU's M-cycles (timing.md); keep that
  shape and the ordering inside a cycle, or the Gambatte lists will move.
- `pixels.py` reads registers live, with CGB history delays
  (`CGB_REG_DELAY`); ppu.py's interrupt/STAT machinery has many FITTED
  constants with Gambatte names in comments — transcribe values exactly.
- The Python PPU keeps under 80 attributes for PyPy; that does not bind Rust,
  but do not "tidy" the Python side.
- SameBoy randomises RAM: never use SameBoy as the gate; the Python board is.

## Working rules

- Do not modify gameboy-python or sm83-rust. If the reference looks wrong,
  stop and show the divergence (sm83 issues go to its repo as notes).
- Transcribe; do not redesign. Correct, not fast (a -fast fork comes later).
- Never copy GPL code; nothing third-party committed (ROMs, boot ROMs,
  test ROMs fetched or read in place; gameboy-python's `boot.py` shows where
  boot ROMs live).
- `nice`, at most about 12 processes (shared machine; other agents run).
- Local `cargo clippy` may be too old to build; CI runs it.
- No `rm -rf`, `sed -i` or `ln -sfn` one-liners; use the Edit tool and new
  directories instead.
- Commit locally at good points without asking. Create a **private** GitHub
  remote `alewman/gameboy-rust` once rung 2 passes and push at milestones;
  minimal CI (main + PRs, concurrency cancel; fmt, clippy, test, cheap rungs).
- Commit trailer: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Keep `docs/status.md` current (passed, open, exact commands) — you may run
  out of context; the next agent starts from it.
