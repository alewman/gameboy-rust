# Status

Every rung passes. The ladder was rerun on 2026-11-12 against the new reference pin, at
`e31d681`; rung 4 on 2026-11-13; the Super Game Boy row (below) and every gate table again on
2026-11-13 at `518eeb0`. Reference: gameboy-python at tag
`gameboy-rust-pin-2` (`47a97e6`) for the board; since 2026-11-13 the clone is checked out at
`c5e9568` (branch `sgb-hle`), which adds `sgb.py` and differs from the pin in `src` only by
comments (`2b0b40f`) and a new PNG helper (`image.rgb24_png_bytes`), so the reference traces
cached at the pin stay valid (checked: see "Super Game Boy"). Cloned to
`external/gameboy-python` (not committed); its interpreter is `external/venv` (PyPy 3.11 with
sm83-python `95f2b8b` and gb-apu-python `f0a7f48`, gameboy-python editable from the clone).
The crate was built from [handoff-brief.md](handoff-brief.md) (historical).

At `518eeb0`: `cargo test --release` 19 pass (3 board, 16 SGB), `cargo fmt --check` and `cargo
clippy --release --all-targets -- -D warnings` (stable toolchain clippy) clean.

## Copyleft clean-up (2026-11-12)

gameboy-python's licence audit found two pieces that had come from Gambatte's GPL-2 source and
replaced them (its docs/validation.md "Copyleft clean-up (2026-11-12)", commit `47a97e6`). This
board carried the same code and now transcribes the replacements: the CGB OAM-DMA conflict rules
(`board.rs`: SameBoy's bus model plus the `oamdma_src*_busypush*/busypop*` verdict fits; the
Gambatte-derived `DmaKind` table is gone) and `gameboy-suite`'s Gambatte reader (digit glyphs and
CGB colours read from `tests/data/`, copies of the tables gameboy-python learned from the tests
themselves; an RGB555 word not in the table matches no colour). The window-undo comment in
`ppu.rs` now says the idea was first read in Gambatte's source and the dots are fitted. Gambatte's
test ROMs and their hardware verdicts remain the judge; nothing under its `libgambatte/` or
`testrunner.cpp` was read for this. Pin-2 also brought the CPU CGB C row (`cgb-c`), the CGB D/E
line-153 shift, and the SCY/palette revision fields, transcribed from `git diff
gameboy-rust-pin-1 gameboy-rust-pin-2 -- src`.

## Rungs

| Rung | Result | Command |
| --- | --- | --- |
| 1 | blargg `cpu_instrs` 01-11 pass natively (and the combined ROM, 3,191 frames) | see below |
| 2 | Lockstep gate built: 36 of 36 cases identical (23 test-ROM cases incl. real DMG and CGB boot, 13 commercial dumps on DMG and CGB, 1,200-1,800 slices each, with `scripts/inputs/poke.txt`), APU register values included | `python3 scripts/lockstep.py --reference-cache external/refcache` |
| 3 | Mealybug DMG 24 of 24 pixel-exact; CGB-D 20 of 21 exact, the 21st (`m3_lcdc_obj_size_change_scx`, 15 pixels: the reference's documented photo match) differing by the same 15 pixels on the reference; CPU CGB C (`--revision cgb_c`, `cgb-c` row) 24 of 27 exact, the three the reference leaves open differing by its pixel counts (`m3_lcdc_obj_size_change_scx` 15, `m3_lcdc_tile_sel_change2` and `m3_lcdc_tile_sel_win_change2` 166 each); cgb-acid-hell, cgb-acid2, dmg-acid2, halt_bug, strikethrough exact. Gambatte from skip-boot: DMG-08 1,695 of 1,784 and CGB-04c on `cgb-c` (`--cgb --machine cgb-c`) 2,892 of 3,221, both the reference's `tests/gambatte_*_pass.txt` exactly. Lockstep: `cases-rung3.tsv` 229 of 229 identical (all Mealybug DMG and CGB-D from the real boot ROMs, acid pictures, every 20th/30th Gambatte listed ROM); `cases-oamdma-cgb.tsv` 418 of 418 (every Gambatte oamdma ROM with a CGB-04c verdict, on `cgb-c`: the rewritten DMA conflict rules) | `gameboy-suite pictures`, `gameboy-suite mealybug-dmg`, `gameboy-suite cgb-pictures`, `gameboy-suite cgb-pictures --revision cgb_c`, `gameboy-suite gambatte [--cgb --machine cgb-c] --skip-boot --write F` and diff with `external/gameboy-python/tests/gambatte_*_pass.txt`; `lockstep.py --cases scripts/cases-rung3.tsv`, `--cases scripts/cases-oamdma-cgb.tsv` |
| 4 | 1,379 suite jobs (rerun at pin-2 on 2026-11-13: the same jobs, 0 differ), every result field identical on both boards (verdict, text, B-L registers, T-cycles, frames; shootout verdict and measured startup time): Mooneye acceptance/emulator-only/misc on DMG-B and CGB E (111 each: 94 and 87 pass), misc from the real CGB boot ROM, the boot_* ROMs from the real DMG/MGB/DMG-0 boot ROMs and quickboot; blargg 47 of 49 (cpu_instrs, instr_timing, mem_timing, mem_timing-2, oam_bug, interrupt_time, dmg_sound, cgb_sound); SameSuite + AGE on CGB 78 of 125 and DMG 11 of 125; GBMicrotest 481 of 513 (= the reference's list); GB Emulator Shootout 257 PASS (pin-1: 256; daid `ppu_scanline_bgp` on GBC now passes, as the reference's CGB line-153 change says it should) | `external/venv/bin/python scripts/suite_jobs.py --out external/rung4-pin2/suite-jobs.jsonl`; `target/release/gameboy-jobs external/rung4-pin2/suite-jobs.jsonl --out external/rung4-pin2/suite-rust.jsonl`; `external/venv/bin/python scripts/suite_ref.py external/rung4-pin2/suite-jobs.jsonl --out external/rung4-pin2/suite-ref.jsonl --jobs 9` (about 90 min); `python3 scripts/suite_compare.py external/rung4-pin2/suite-ref.jsonl external/rung4-pin2/suite-rust.jsonl` |
| 5 | Every board class under the gate on a dump that uses it: `cases-rung5.tsv` 21 of 21 identical (ROM-only, MBC1, MBC1+RAM+battery, MBC1M multicart x2, MBC2, MBC3+RAM+battery, MBC3+timer x3 incl. CGB, MBC30, MBC5, MBC5+rumble, MMM01 x2; rtc3test basic/range/sub-second and mbc3-tester on DMG and CGB). The MBC3 clock runs on board time on both sides. (No No-Intro dump uses ROM+RAM, $08/$09.) Dumps found by `scripts/mapper_dumps.py` | `lockstep.py --cases scripts/cases-rung5.tsv` |
| 6 | Every row of `machines.py` under the gate: `cases-rung6.tsv` 48 of 48 identical (DMG-0/A/B/C and MGB from their real boot ROMs, SGB/SGB2 from skip-boot, CGB; dmg-acid2, boot_regs, boot_hwio, stat_irq_blocking, oam_bug 5, Tetris; CPU CGB C: Tetris and stat_irq_blocking from the real boot ROM, Mealybug m3_scy/bgp/obp0_change, Gambatte ly0 lycint152 x3). Save states (`src/state.rs`, the reference has none): the round trip (save half way, load into a new console, go on) identical to the reference's uninterrupted trace on every case of `cases.tsv`, `cases-rung5.tsv` and `cases-rung6.tsv` (105 of 105; with the APU in the state since 2026-11-13, every table: 871 of 871, see "Save states"); `tests/board.rs` checks it without ROMs | `lockstep.py --cases scripts/cases-rung6.tsv`; add `--save-at-half` for the round trip |
| 7 | gb-apu-rust adopted (by path, 0c2c240). APU register reads and writes are in every gate comparison (all case tables above, identical). `cases-rung7.tsv` with `--extra=--audio` (a CRC of each slice's 44.1 kHz samples, `apu.capture`): 119 of 119 identical (blargg dmg_sound and cgb_sound singles, SameSuite apu, every 12th Gambatte outaudio ROM from the real boot ROM, the 13 game cases). Gambatte audio (`gameboy-suite gambatte-audio`, real DMG boot ROM, `APU.audible()` in the last of 15 frames): 82 of 87 agree with the DMG-08, the same 82 as the reference at the pin (its committed `tests/gambatte_audio_pass.txt` has 76: stale by 6). blargg dmg_sound/cgb_sound verdicts are in rung 4's jobs, identical | `lockstep.py --cases scripts/cases-rung7.tsv --extra=--audio`; `gameboy-suite gambatte-audio --write F` and diff with the reference's `scripts/gambatte_audio.py --write` |
| SGB | `src/sgb.rs`, the transcription of the reference's HLE Super Game Boy (`sgb.py` at `c5e9568`), under the gate: with `--sgb` both trace tools attach it, send the inputs to its pad and add `sgb=PIC/STATE` to each slice (CRC-32 of `render()`'s 256 x 224 words and of `save_state()`'s blob). `cases-sgb.tsv` 17 of 17 identical (22,500 slices): the eight games the reference validated against the real BIOS (Donkey Kong, Kirby's Dream Land 2, Pokemon Red, Pokemon Yellow, Link's Awakening DX, Wario Land II, Mario's Picross, Tetris DX), 1,500 slices each from the SGB boot ROM (every border is sent by about frame 780: Tetris DX's is the last), Donkey Kong on SGB2 from its boot ROM, and all eight from skip-boot (Pokemon Red 900 slices, Donkey Kong and Mario's Picross 1,200, the rest 1,500: every one loads its border). Save/load round trip on the table 17 of 17. PNGs from `gameboy-sgb` and the reference's `scripts/sgb_picture.py`: pixel-identical on all five compared (Donkey Kong skip-boot 600 = the reference test's SHA-1 `0cf50643...`; Pokemon Red 900, Kirby 2 1200, Tetris DX 1000, Link's Awakening DX 800 from the boot ROM). Seeded one-line faults in `sgb.rs`, over `cases-sgb.tsv`: 4 of 5 caught (first screen line 12 cases, VRAM transfer delay 11, border palette bits 11, PAL_SET shared colour 0 5); MISSED: ATTR_BLK's inside-only edge palette (`inside_pal -> outside_pal`): none of the twelve cases sends an inside-only block whose outside palette differs; `tests/sgb.rs` `attr_blk_inside_edge_outside_and_the_exception` fails on it | `lockstep.py --cases scripts/cases-sgb.tsv [--save-at-half]`; `gameboy-sgb ROM --boot mame --frames N --png F`; `seeded_faults.py --cases scripts/cases-sgb.tsv --only sgb:` |
| faults | 17 of 18 seeded one-line faults caught over `cases.tsv`, `cases-rung3/5/6.tsv` and `cases-targeted.tsv` (168 cases: Mooneye mbc/timer/oam_dma/ppu, GBMicrotest vblank/lyc/win/dma, all identical on the true board): PPU (OAM scan height, first-object discount, mode-0 read lag, VBlank IF lateness, HBlank IRQ lead), pixels (DMG BGP glitch, CGB register delay, window row counter), DMA (start delay, length, bus conflict), timer (TAC=01 bit, reload-cycle write, DIV-write edge), cartridge (MBC1 bank 0, MBC3 RTC carry), serial clock bit. MISSED: MBC5 ROM bank bit 8 (no case reads banks 256+ differently: Mooneye `mbc5/rom_64Mb` passes with the bit stuck at 0, on both boards; a >4 MB MBC5 dump case would close it). Two first-draft mutants were equivalent and were replaced (LY=LYC compare at line 153 `dot < 8 -> 12`, masked by `lyc_match`; the timing PPU's `window_line`, written but never read here or in the reference) | `python3 scripts/seeded_faults.py --cases scripts/cases.tsv --cases scripts/cases-rung3.tsv --cases scripts/cases-rung5.tsv --cases scripts/cases-rung6.tsv --cases scripts/cases-targeted.tsv` (worktree at external/faults-tree) |

Setup, once (the reference repositories beside this one). The test ROMs are the ones the
reference's `scripts/fetch_references.py` puts in its `tests/test_roms/`; the case tables and
`gameboy-suite` read them in place, by default from `/data/emu/gameboy-python/tests/test_roms`
and No-Intro dumps from `/data/emu/source/...`: set `GBTR`, `MTS` and `NOINTRO` to move them.

```sh
git clone ../gameboy-python external/gameboy-python && git -C external/gameboy-python checkout c5e9568  # sgb-hle: pin-2 + sgb.py
pypy3 -m venv external/venv
external/venv/bin/pip install "git+file://$PWD/../sm83-python@95f2b8b39c1a1b3890d3b47b39cb1f698e948eca" \
    "git+file://$PWD/../gb-apu-python@f0a7f48b72dab9cf019cefc3968a09d0227fd189"
external/venv/bin/pip install --no-deps -e external/gameboy-python
# rung 1
B=${GBTR:-/data/emu/gameboy-python/tests/test_roms/gbtr}/blargg
for f in $B/cpu_instrs/individual/*.gb; do target/release/gameboy $f --protocol blargg-serial --max-frames 2000; done
```

## Super Game Boy

`src/sgb.rs` transcribes `sgb.py` line by line (docs/sgb.md in the reference is the guide). The
board owns the unit: `Hw::sgb: Option<Box<SuperGameBoy>>`, `None` unless `GameBoy::attach_sgb`
is called on an `sgb`/`sgb2` row; it is serde, so it is part of the board's save state
(header `GBRUST03`). The reference's three callbacks are board calls: a P1 write is
`Hw::joypad_write` (`Joypad::write_select`, then `SuperGameBoy::p1_write`, then
`Joypad::fell`: the order of the reference's `Joypad.write` with `on_write`), and the PPU,
with `taps` set, queues `Tap::Line(ly, shades)` / `Tap::Frame` where the reference calls
`line_out` / `frame_start`; `Hw::sgb_taps` hands them over right after each PPU tick. With no
SGB attached the PPU queues nothing and the P1 write is the old one: every DMG/CGB table is
unchanged (cases 36, rung 3 229, rung 5 21, rung 6 48, oamdma 418, rung 7 `--audio` 119, each
also with `--save-at-half`: 871 + 871 identical). The reference traces of `cases.tsv` were also
regenerated from the `a8bdf5e` clone (`external/refcache-sgbhle`): byte-identical to the
pin-2 cache.

The SGB boot ROM is bring-your-own: the case table uses `$SGBBOOT` / `$SGB2BOOT`, which
`lockstep.py` finds as MAME's `sns_rom_sgb.zip` / `sns_rom_sgb2.zip` in `$MAME_ROMPATH`; the
games come from `$NOINTRO` (`NO_INTRO_DIR`).

## Findings about the reference

**`sgb.py` from skip-boot lost the game's first packet (fixed in the reference, c5e9568, and here).** The ICD2 starts with
`p1 = gb.joypad.select`, which is $00 in the skip-boot state (the board's P1 power-on value);
after the real SGB boot ROM it is $30 (its header packets end with both lines high). A game's
first packet starts with a write of $00, which `p1_write` then ignores as "no change", so the
start pulse is missed and the packet is lost. Pokemon Red's first packet is its SGB check
(`MLT_REQ` two players, `89 01 ...`): from skip-boot the game decides it is not on an SGB and
never sends a border. Repro on the reference: `scripts/sgb_picture.py "Pokemon - Red Version
(USA, Europe) (SGB Enhanced).zip" --frames 900 --packets` prints one packet (`frame 43:
8900...`); with `--boot mame` it prints the header packets, `89 01` at frame 25 and the border
transfers. From skip-boot Pokemon Yellow, Kirby's Dream Land 2, Wario Land II, Link's
Awakening DX and Tetris DX show no border by frame 1,500 either (likely the same cause, not
traced one by one); Donkey Kong and Mario's Picross are not affected. The reference's
validation ran from the boot ROM, where this does not arise. A possible fix there: start the
ICD2 at $30 when no boot ROM is mapped. Done that way in gameboy-python c5e9568 and transcribed
here (`SuperGameBoy::new` takes `boot_mapped`); with it all eight games load their borders from
skip-boot, and the five that did not before are now cases.



No divergence where gameboy-python looks wrong; nothing for sm83-rust either. At pin-1 two of
its committed pass lists were stale; at pin-2 `tests/gambatte_cgb04c_pass.txt` is current (2,892
on `cgb-c`), and `tests/gambatte_audio_pass.txt` (76) still trails what the board agrees on (82
of 87). The PPU's `window_line` is dead state (written, never read). Three comments that named
Gambatte's source functions were reworded in gameboy-python `2b0b40f` (comments only, after
pin-2) and here to match.

## Open

- An MBC5 case with more than 4 MB of ROM, for the one missed fault.
- The SGB skip-boot finding above, for gameboy-python; when it changes there, the SGB cases move
  with it.
- The SGB is not yet in a player: the speed fork and the browser player can take `sgb.rs` as is.
- Later, in other repositories: a speed fork (gameboy-rust-fast) with an SDL2 window.

## APU

gb-apu-rust (4032227, its ladder passed; the `serde` feature on) is used by path: `src/apu.rs`
re-exports `gb_apu::APU`, and APU register values are compared in every gate case (`--mask-apu`
on both trace tools zeroes them, to compare against traces recorded before the APU was in the
gate). Rung 7 above covers the audio itself. `gb_apu::APU` is not `Clone`, so `Hw` and
`GameBoy` are not `Clone`.

## Save states

`GameBoy::save_state()` / `load_state()` carry the CPU (`CpuState`) and all of `Hw`, the APU
included (gb-apu-rust's `serde` feature, 2026-11-13), except the cartridge ROM/header, the boot
ROM and the machine row, which come from the console the state is loaded into. The header is
`GBRUST02` (`GBRUST01` states had no APU and are refused). The gate's round trip
(`--save-at-half`: save before the middle slice, load into a new console, go on there) is
identical to the reference's uninterrupted trace on every table: cases 36, rung 3 229, rung 5
21, rung 6 48, oamdma 418, rung 7 with `--audio` 119 (871 of 871, sample CRCs included).

## The gate

`scripts/lockstep.py` runs `target/release/gameboy-trace` and `scripts/gameboy_trace.py` (on
`external/venv`) per case of `scripts/cases.tsv`, one line per 70,224 T slice: CPU state, board
time, timer, PPU/DMA position, serial length, CRC-32 of every bus M-cycle (`t<<26|kind<<24|
address<<8|value`) and CRC-32 of the finished picture. On a difference rerun both tools with
`--detail SLICE` and diff. `--reference-cache DIR` keeps the reference's traces (empty it when the pin
moves; pin-1's are in `external/refcache-pin1`). MBC3 clocks run on board time on both sides.
