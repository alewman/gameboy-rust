# Status

Every rung passes; the ladder below was run on 2026-11-12, the last rung and the seeded faults
at `8887cea`. Reference: gameboy-python at tag `gameboy-rust-pin-1` (`efdde44`), cloned to
`external/gameboy-python` (not committed); its interpreter is `external/venv` (PyPy 3.11 with
sm83-python `95f2b8b` and gb-apu-python `f0a7f48`, gameboy-python editable from the clone).
The crate was built from [handoff-brief.md](handoff-brief.md) (historical).

Re-checked at `8887cea` (2026-11-12, docs-only changes since): `cargo test --release` 3 pass,
`cargo fmt --check` and `cargo clippy --release --all-targets -- -D warnings` (clippy 0.1.99)
clean, the gate on `cases.tsv` 36 of 36 identical and 36 of 36 with `--save-at-half`.

## Rungs

| Rung | Result | Command |
| --- | --- | --- |
| 1 | blargg `cpu_instrs` 01-11 pass natively (and the combined ROM, 3,191 frames) | see below |
| 2 | Lockstep gate built: 36 of 36 cases identical (23 test-ROM cases incl. real DMG and CGB boot, 13 commercial dumps on DMG and CGB, 1,200-1,800 slices each, with `scripts/inputs/poke.txt`), APU register values included | `python3 scripts/lockstep.py --reference-cache external/refcache` |
| 3 | Mealybug DMG 24 of 24 pixel-exact; CGB-D 20 of 21 exact, the 21st (`m3_lcdc_obj_size_change_scx`, 15 pixels: the reference's documented photo match) differing by the same 15 pixels on the reference; cgb-acid-hell, cgb-acid2, dmg-acid2, halt_bug, strikethrough exact. Gambatte: DMG-08 1,695 of 1,784, the pass list reproduced exactly (skip-boot and real boot); CGB-04c 2,867 of 3,221 from skip-boot = the list's 2,864 plus 3 the list is stale on (the reference at the pin passes them too: `enable_display/frame1_m2stat_count_ds_2`, `lcd_offset/offset1_lyc99int_m2stat_count_ds_2`, `ly0/lycint152_ly0stat_ds_3`). Lockstep: `cases-rung3.tsv` 229 of 229 identical (all Mealybug DMG and CGB-D from the real boot ROMs, acid pictures, every 20th/30th Gambatte listed ROM) | `gameboy-suite pictures`, `gameboy-suite mealybug-dmg`, `gameboy-suite cgb-pictures`, `gameboy-suite gambatte [--cgb] [--skip-boot] --write F` and diff with `external/gameboy-python/tests/gambatte_*_pass.txt`; `lockstep.py --cases scripts/cases-rung3.tsv` |
| 4 | 1,379 suite jobs, every result field identical on both boards (verdict, text, B-L registers, T-cycles, frames; shootout verdict and measured startup time): Mooneye acceptance/emulator-only/misc on DMG-B and CGB E (111 each: 94 and 87 pass), misc from the real CGB boot ROM, the boot_* ROMs from the real DMG/MGB/DMG-0 boot ROMs and quickboot; blargg 47 of 49 (cpu_instrs, instr_timing, mem_timing, mem_timing-2, oam_bug, interrupt_time, dmg_sound, cgb_sound); SameSuite + AGE on CGB 78 of 125 and DMG 11 of 125; GBMicrotest 481 of 513 (= the reference's list); GB Emulator Shootout 256 PASS (the reference's 256/258 score) | `external/venv/bin/python scripts/suite_jobs.py --out external/suite-jobs.jsonl`; `target/release/gameboy-jobs external/suite-jobs.jsonl --out external/suite-rust.jsonl`; `external/venv/bin/python scripts/suite_ref.py external/suite-jobs.jsonl --out external/suite-ref.jsonl` (about 90 min at 9 jobs); `python3 scripts/suite_compare.py external/suite-ref.jsonl external/suite-rust.jsonl` |
| 5 | Every board class under the gate on a dump that uses it: `cases-rung5.tsv` 21 of 21 identical (ROM-only, MBC1, MBC1+RAM+battery, MBC1M multicart x2, MBC2, MBC3+RAM+battery, MBC3+timer x3 incl. CGB, MBC30, MBC5, MBC5+rumble, MMM01 x2; rtc3test basic/range/sub-second and mbc3-tester on DMG and CGB). The MBC3 clock runs on board time on both sides. (No No-Intro dump uses ROM+RAM, $08/$09.) Dumps found by `scripts/mapper_dumps.py` | `lockstep.py --cases scripts/cases-rung5.tsv` |
| 6 | Every row of `machines.py` under the gate: `cases-rung6.tsv` 40 of 40 identical (DMG-0/A/B/C and MGB from their real boot ROMs, SGB/SGB2 from skip-boot, CGB; dmg-acid2, boot_regs, boot_hwio, stat_irq_blocking, oam_bug 5, Tetris). Save states (`src/state.rs`, the reference has none): the round trip (save half way, load into a new console, go on) identical to the reference's uninterrupted trace on every case of `cases.tsv`, `cases-rung5.tsv` and `cases-rung6.tsv` (97 of 97); `tests/board.rs` checks it without ROMs | `lockstep.py --cases scripts/cases-rung6.tsv`; add `--save-at-half` for the round trip |
| 7 | gb-apu-rust adopted (by path, 0c2c240). APU register reads and writes are in every gate comparison (all case tables above, identical). `cases-rung7.tsv` with `--extra=--audio` (a CRC of each slice's 44.1 kHz samples, `apu.capture`): 119 of 119 identical (blargg dmg_sound and cgb_sound singles, SameSuite apu, every 12th Gambatte outaudio ROM from the real boot ROM, the 13 game cases). Gambatte audio (`gameboy-suite gambatte-audio`, real DMG boot ROM, `APU.audible()` in the last of 15 frames): 82 of 87 agree with the DMG-08, the same 82 as the reference at the pin (its committed `tests/gambatte_audio_pass.txt` has 76: stale by 6). blargg dmg_sound/cgb_sound verdicts are in rung 4's jobs, identical | `lockstep.py --cases scripts/cases-rung7.tsv --extra=--audio`; `gameboy-suite gambatte-audio --write F` and diff with the reference's `scripts/gambatte_audio.py --write` |
| faults | 17 of 18 seeded one-line faults caught over `cases.tsv`, `cases-rung3/5/6.tsv` and `cases-targeted.tsv` (168 cases: Mooneye mbc/timer/oam_dma/ppu, GBMicrotest vblank/lyc/win/dma, all identical on the true board): PPU (OAM scan height, first-object discount, mode-0 read lag, VBlank IF lateness, HBlank IRQ lead), pixels (DMG BGP glitch, CGB register delay, window row counter), DMA (start delay, length, bus conflict), timer (TAC=01 bit, reload-cycle write, DIV-write edge), cartridge (MBC1 bank 0, MBC3 RTC carry), serial clock bit. MISSED: MBC5 ROM bank bit 8 (no case reads banks 256+ differently: Mooneye `mbc5/rom_64Mb` passes with the bit stuck at 0, on both boards; a >4 MB MBC5 dump case would close it). Two first-draft mutants were equivalent and were replaced (LY=LYC compare at line 153 `dot < 8 -> 12`, masked by `lyc_match`; the timing PPU's `window_line`, written but never read here or in the reference) | `python3 scripts/seeded_faults.py --cases scripts/cases.tsv --cases scripts/cases-rung3.tsv --cases scripts/cases-rung5.tsv --cases scripts/cases-rung6.tsv --cases scripts/cases-targeted.tsv` (worktree at external/faults-tree) |

Setup, once (the reference repositories beside this one). The test ROMs are the ones the
reference's `scripts/fetch_references.py` puts in its `tests/test_roms/`; the case tables and
`gameboy-suite` read them in place, by default from `/data/emu/gameboy-python/tests/test_roms`
and No-Intro dumps from `/data/emu/source/...`: set `GBTR`, `MTS` and `NOINTRO` to move them.

```sh
git clone ../gameboy-python external/gameboy-python && git -C external/gameboy-python checkout gameboy-rust-pin-1
pypy3 -m venv external/venv
external/venv/bin/pip install "git+file://$PWD/../sm83-python@95f2b8b39c1a1b3890d3b47b39cb1f698e948eca" \
    "git+file://$PWD/../gb-apu-python@f0a7f48b72dab9cf019cefc3968a09d0227fd189"
external/venv/bin/pip install --no-deps -e external/gameboy-python
# rung 1
B=${GBTR:-/data/emu/gameboy-python/tests/test_roms/gbtr}/blargg
for f in $B/cpu_instrs/individual/*.gb; do target/release/gameboy $f --protocol blargg-serial --max-frames 2000; done
```

## Findings about the reference

No divergence where gameboy-python looks wrong; nothing for sm83-rust either. Two of its
committed pass lists are older than the pin (the reference at `efdde44` agrees with this board):
`tests/gambatte_cgb04c_pass.txt` (2,864; the pin passes 2,867 from skip-boot) and
`tests/gambatte_audio_pass.txt` (76; the pin agrees on 82 of 87). The PPU's `window_line` is
dead state (written, never read).

## Open

- APU state in save states (below): needs a state restore in gb-apu-rust.
- An MBC5 case with more than 4 MB of ROM, for the one missed fault.
- pin-2: gameboy-python past `efdde44` (the CGB revision rows, CGB E line-153 changes), once
  Aubrey tags it.
- Later, in other repositories: a speed fork (gameboy-rust-fast) with an SDL2 window.

## APU

gb-apu-rust (0c2c240, its ladder passed) is used by path: `src/apu.rs` re-exports
`gb_apu::APU`, and APU register values are compared in every gate case (`--mask-apu` on both
trace tools zeroes them, to compare against traces recorded before the APU was in the gate).
Rung 7 above covers the audio itself. `gb_apu::APU` is neither `Clone` nor serde, so `Hw` and
`GameBoy` are not `Clone`.

## Save states

`GameBoy::save_state()` / `load_state()` carry the CPU (`CpuState`) and all of `Hw` except the
APU, the cartridge ROM/header, the boot ROM and the machine row, which come from the console the
state is loaded into. The APU is the gap: gb-apu-rust (like gb-apu-python) has no state
restore and its `APU` is not serde, so a state loaded into another console continues with that
console's APU (audio and APU reads go on from there). The gate's round trip (`--save-at`)
therefore hands the new console the old one's live APU and proves everything else exact. Closing
the gap needs gb-apu-rust to offer serde (or a restore of `capture_state()` plus its internals).

## The gate

`scripts/lockstep.py` runs `target/release/gameboy-trace` and `scripts/gameboy_trace.py` (on
`external/venv`) per case of `scripts/cases.tsv`, one line per 70,224 T slice: CPU state, board
time, timer, PPU/DMA position, serial length, CRC-32 of every bus M-cycle (`t<<26|kind<<24|
address<<8|value`) and CRC-32 of the finished picture. On a difference rerun both tools with
`--detail SLICE` and diff. `--reference-cache DIR` keeps the reference's traces (the pin does not
move). MBC3 clocks run on board time on both sides.
