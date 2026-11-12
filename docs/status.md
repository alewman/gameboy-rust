# Status

Brief: [handoff-brief.md](handoff-brief.md). Reference: gameboy-python at tag
`gameboy-rust-pin-1` (`efdde44`), cloned to `external/gameboy-python` (not committed);
its interpreter is `external/venv` (PyPy 3.11 with sm83-python `95f2b8b` and gb-apu-python
`f0a7f48` installed from the local checkouts, gameboy-python editable from the clone).

## Rungs

| Rung | Result | Command |
| --- | --- | --- |
| 1 | blargg `cpu_instrs` 01-11 pass natively (and the combined ROM, 3,191 frames) | see below |
| 2 | Lockstep gate built: 36 of 36 cases identical (23 test-ROM cases incl. real DMG and CGB boot, 13 commercial dumps on DMG and CGB, 1,200-1,800 slices each, with `scripts/inputs/poke.txt`), APU register values included | `python3 scripts/lockstep.py --reference-cache external/refcache` |

```sh
# setup (once)
git clone /data/emu/gameboy-python external/gameboy-python && git -C external/gameboy-python checkout gameboy-rust-pin-1
~/.local/opt/pypy/bin/pypy3 -m venv external/venv
external/venv/bin/pip install "git+file:///data/emu/sm83-python@95f2b8b39c1a1b3890d3b47b39cb1f698e948eca" \
    "git+file:///data/emu/gb-apu-python@f0a7f48b72dab9cf019cefc3968a09d0227fd189"
external/venv/bin/pip install --no-deps -e external/gameboy-python
# rung 1
B=/data/emu/gameboy-python/tests/test_roms/gbtr/blargg
for f in $B/cpu_instrs/individual/*.gb; do target/release/gameboy $f --protocol blargg-serial --max-frames 2000; done
```

## APU

gb-apu-rust (`/data/emu/gb-apu-rust` 0c2c240, its ladder passed) is adopted by path since rung
2: `src/apu.rs` re-exports `gb_apu::APU`. Before that a placeholder APU made two cases
(Pokemon Crystal, SMB Deluxe) diverge on NR52/NR30 reads; with the real core every case is
identical with APU values compared (`--mask-apu` on both trace tools zeroes them, for history).
Rung 7's audio checks (Gambatte audio tests, the board's audio outputs) are still to do.
`gb_apu::APU` is neither `Clone` nor serde: `Hw` and `GameBoy` are not `Clone`, and save states
(rung 6) need APU state that the crate does not expose (nor does gb-apu-python).

## The gate

`scripts/lockstep.py` runs `target/release/gameboy-trace` and `scripts/gameboy_trace.py` (on
`external/venv`) per case of `scripts/cases.tsv`, one line per 70,224 T slice: CPU state, board
time, timer, PPU/DMA position, serial length, CRC-32 of every bus M-cycle (`t<<26|kind<<24|
address<<8|value`) and CRC-32 of the finished picture. On a difference rerun both tools with
`--detail SLICE` and diff. `--reference-cache DIR` keeps the reference's traces (the pin does not
move). MBC3 clocks run on board time on both sides.
