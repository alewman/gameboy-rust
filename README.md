# gameboy-rust

A Rust Game Boy (DMG and CGB) board transcribed from
[gameboy-python](https://github.com/alewman/gameboy-python) at tag `gameboy-rust-pin-1`
(`efdde44`) and proven equivalent to it in lockstep: every CPU bus M-cycle (address, value,
direction, board time) and every finished picture, slice by slice. The CPU is
[sm83-rust](https://github.com/alewman/sm83-rust) (crate `sm83`), the sound
[gb-apu-rust](https://github.com/alewman/gb-apu-rust) (crate `gb_apu`), both by path
(private repositories, checked out beside this one).

Each `src/*.rs` file is the transcription of one `gameboy_python` module (`board`, `ppu`,
`pixels`, `dma`, `timer`, `joypad`, `serial`, `machines`, `cartridge`, `boot`, `testrom`), with
the same names, the same order of work inside an M-cycle and the hardware comments carried over;
where Rust forced a difference, the module header says so.

Binaries: `gameboy` (run a ROM, test-ROM protocols), `gameboy-trace` (the board side of the
lockstep gate), `gameboy-suite` (Gambatte, Mealybug and picture suites with the reference's
rules). The gate is `scripts/lockstep.py` over `scripts/cases.tsv`.

Ladder status and exact commands: [docs/status.md](docs/status.md); the brief:
[docs/handoff-brief.md](docs/handoff-brief.md).

## Licence

MIT. `src/pixels.rs` carries docboy's MIT notice (its pipeline, by way of gameboy-python).
Nothing third-party is committed: ROMs, boot ROMs and test ROMs are read in place.
