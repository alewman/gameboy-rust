# Status

Brief: [handoff-brief.md](handoff-brief.md). Reference: gameboy-python at tag
`gameboy-rust-pin-1` (`efdde44`), cloned to `external/gameboy-python` (not committed);
its interpreter is `external/venv` (PyPy 3.11 with sm83-python `95f2b8b` and gb-apu-python
`f0a7f48` installed from the local checkouts, gameboy-python editable from the clone).

## Rungs

| Rung | Result | Command |
| --- | --- | --- |
| 1 | blargg `cpu_instrs` 01-11 pass natively (and the combined ROM, 3,191 frames) | see below |

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

gb-apu-rust is not adopted yet: `src/apu.rs` is a placeholder (registers as written, no
channels, no sound). Its register reads differ from the reference's, so the lockstep gate
leaves the values of reads of $FF10-$FF3F and $FF76-$FF77 out of the comparison until rung 7.
