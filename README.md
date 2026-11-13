# gameboy-rust

A Rust Game Boy (DMG and CGB) board transcribed from
[gameboy-python](https://github.com/alewman/gameboy-python) at tag `gameboy-rust-pin-2`
(`47a97e6`) and proven equivalent to it in lockstep: every CPU bus M-cycle (address, value,
direction, board time) and every finished picture, slice by slice. The CPU is
[sm83-rust](https://github.com/alewman/sm83-rust) (crate `sm83`), the sound
[gb-apu-rust](https://github.com/alewman/gb-apu-rust) (crate `gb_apu`); both are path
dependencies, so check them out beside this one.

Each `src/*.rs` file is the transcription of one `gameboy_python` module (`board`, `ppu`,
`pixels`, `dma`, `timer`, `joypad`, `serial`, `machines`, `cartridge`, `boot`, `testrom`), with
the same names, the same order of work inside an M-cycle and the hardware comments carried over;
where Rust forced a difference, the module header says so. It is a reference board: correct,
not fast, and there is no window yet.

## Build and run

```sh
git clone https://github.com/alewman/sm83-rust
git clone https://github.com/alewman/gb-apu-rust
git clone https://github.com/alewman/gameboy-rust
cd gameboy-rust
cargo test --release                      # ROM-free board tests, save-state round trip
cargo build --release
target/release/gameboy cpu_instrs.gb --protocol blargg-serial --max-frames 4000
target/release/gameboy game.gb --machine cgb --boot real --frames 600
```

`gameboy ROM [--machine KEY] [--boot skip|real|quick|PATH] [--frames N] [--protocol
mooneye|blargg-serial|blargg-ram|microtest] [--max-frames N]` runs a ROM headless and prints the
serial or protocol text. Machine keys are the rows of `src/machines.rs`: `dmg` (= `dmg-b`),
`dmg-0`, `dmg-a`, `dmg-c`, `mgb`, `sgb`, `sgb2`, `cgb`. Boot ROMs are bring-your-own: `--boot PATH` or `GAMEBOY_BOOT_ROM`, then
a No-Intro `[BIOS]` set under `GAMEBOY_NOINTRO_DIR` (checked by SHA-256); `--boot skip` (the
default) starts from the boot ROM's hand-off state. Nothing is copied into this repository.

The other binaries are the judges' tools: `gameboy-trace` (the board side of the lockstep gate),
`gameboy-suite` (Gambatte, Mealybug and picture suites with the reference's rules) and
`gameboy-jobs` (rung 4's suite jobs).

## Status

Every rung passes (measured 2026-11-12; the last rung and the seeded faults at `8887cea`). Exact
numbers and commands are in [docs/status.md](docs/status.md). No case was found where
gameboy-python looks wrong.

| Rung | What | Result |
| --- | --- | --- |
| 1 | Skeleton: blargg `cpu_instrs` natively | 01-11 and the combined ROM pass |
| 2 | Lockstep gate (`scripts/lockstep.py`, `scripts/cases.tsv`): test ROMs from the real boot ROMs and commercial dumps, DMG and CGB | 36 of 36 identical |
| 3 | PPU: Mealybug Tearoom, acid tests, Gambatte hardware tests | Mealybug DMG 24/24, CGB-D as the reference (20 exact, the 21st off by the same 15 pixels), CPU CGB C 24 of 27 as the reference (on the `cgb-c` row); dmg-acid2, cgb-acid2, cgb-acid-hell exact; Gambatte DMG 1,695 and CGB-04c 2,892 on `cgb-c` (skip-boot), the reference's pass lists exactly; 229 of 229 gate cases, and 418 of 418 Gambatte oamdma ROMs on `cgb-c` |
| 4 | Mooneye, blargg, SameSuite, AGE, GBMicrotest, GB Emulator Shootout | 1,379 suite jobs, every result field identical to the reference's (Shootout 257 PASS) |
| 5 | Every cartridge class under the gate (ROM, MBC1/1M/2/3/30/5, MMM01, MBC3 clock) | 21 of 21 identical |
| 6 | Every machine row; save states | 48 of 48 identical (incl. the CPU CGB C row); save/load round trip identical on every gate case, audio included (871 of 871) |
| 7 | gb-apu-rust in the gate: APU reads and writes in every comparison, a CRC of each slice's samples | 119 of 119 identical; Gambatte audio 82 of 87, as the reference |
| faults | 18 one-line faults seeded across PPU, pixels, DMA, timer, cartridge, serial | 17 caught; missed: MBC5 ROM bank bit 8 (no case reads banks 256 and up) |

**Save states.** `GameBoy::save_state()` / `load_state()` carry the CPU and the whole board,
sound unit included (gb-apu-rust's `serde` feature); the cartridge ROM, boot ROM and machine row
come from the console the state is loaded into. Saved half way and loaded into a new console,
every gate case goes on identical to the reference's uninterrupted run, audio included (871 of
871). gameboy-python has no save states at all; these are this board's own.

## How it is judged

The reference board is the arbiter. `scripts/lockstep.py` runs `gameboy-trace` and the
reference's `scripts/gameboy_trace.py` over a case table and compares one line per 70,224 T
slice: CPU state, board time, timer, PPU and DMA position, a CRC-32 of every bus M-cycle and
of the finished picture. The suites (rungs 3 and 4) are compared result by result with the
reference's runners; `scripts/seeded_faults.py` breaks the board one line at a time and counts
what the gate catches. The reference runs from a clone at the pin in `external/` with its own
PyPy venv (setup in [docs/status.md](docs/status.md)); test ROMs and dumps are read in place
(`GBTR`, `MTS`, `NOINTRO` override the default locations).

CI (main and pull requests) runs `cargo fmt --check`, `cargo clippy -D warnings` and
`cargo test`; the gate and the suites need the reference and the ROMs and run by hand.

## Licence and credits

MIT (`LICENSE`). `src/pixels.rs` is a translation (by way of gameboy-python) of
[docboy](https://github.com/Docheinstein/docboy)'s pixel pipeline and carries its MIT notice.
Nothing third-party is committed: ROMs, boot ROMs and test ROMs are read in place.

The test suites that judge it: Mooneye Test Suite, blargg's test ROMs, Mealybug Tearoom,
the Gambatte hardware tests, SameSuite, AGE, GBMicrotest, dmg-acid2, cgb-acid2, cgb-acid-hell
and the GB Emulator Shootout's method.
