<!-- SPDX-License-Identifier: MIT -->
# Provenance

This board transcribes gameboy-python (its `external/gameboy-python` pin); the licence findings live upstream (gameboy-python
`docs/validation.md`, "Closed-box sources removed") and in emulator-lineage `docs/sources.md`.
This file holds what is kept apart from them.

## Author respect (2026-11-26)

This is respect, not law. furrtek objects to AI use of his work, and the family sets his work
and its lineage aside for that wish (Aubrey, 2026-11-26; emulator-lineage `docs/sources.md` at
`d9b8ce2`): DMG-CPU-Inside, msinger's dmg-schematics and gbdoc, the dmg-sim built on them, and
GateBoy. The audit of the facts is upstream, in gameboy-python's and sm83-python's
`docs/provenance.md` "Author respect" (branch `author-respect`, 2026-11-26); this repository
only inherits them.

**What this repository inherited** (checked 2026-11-26 by searching every tracked file):

| Where | What | Set-aside source | Other evidence (upstream audit) | Status |
| --- | --- | --- | --- | --- |
| `src/board.rs` skip-boot (counter `$ABC8`, comment l.412) | the counter phase at the DMG hand-off, "what dmg-sim's quickboot leaves" | dmg-sim's quickboot | TEST ROM: Mooneye `boot_div-dmgABCmgb`; OBSERVED: the Python board on the real DMG boot ROM reaches the same counter | backed; comment to re-cite on carry-down |
| `src/board.rs` skip-boot (`place(153, 396, 1)`, l.424) | the PPU phase at the hand-off, "as quickboot leaves it" | dmg-sim's quickboot | OBSERVED: the Python board on the real boot ROM reaches the same phase; TEST ROM: Mooneye bounds it within 4 dots | backed; comment to re-cite |
| `src/serial.rs` (l.12) | `SERIAL_PHASE_T` "not read off the die (dmg-schematics)" | named only | TEST ROM, FITTED: Mooneye `boot_sclk_align` with `boot_div` | backed; comment to re-cite |
| `src/boot.rs`, `src/bin/gameboy.rs` (`--boot quick`), `src/bin/gameboy-jobs.rs` | dmg-sim's `quickboot.bin` as the second boot ROM choice | dmg-sim | upstream removed the choice: real ROM, then SameBoy's open ROM (labelled), else skip-boot | **owed: carry-down** |
| `scripts/suite_jobs.py`, `scripts/suite_ref.py`, docs/status.md rung 4 | rung 4's `mooneye-dmg-quick/*` jobs (the boot_* ROMs booted from quickboot) | dmg-sim | the same ROMs run from the real DMG, MGB and DMG-0 boot ROMs (`mooneye-dmg-real/*` and the others) | **owed: carry-down** |
| the core (sm83-rust, transcribing sm83-python) | the halted `$0000` bus and the HALT wake | dmg-sim | DIE: Gekkio's VHDL; TEST ROM: one extra wake cycle fails Mooneye `halt_ime1_timing2-GS` and `di_timing-GS` | backed |

Counts: 6 inherited items; 4 facts, all backed by other evidence; 0 needing new evidence here;
2 tooling items owed. No document in `docs/` claimed that furrtek's work was unused.

**What changed on 2026-11-26, and what did not.** No file in this repository changed except
this one: code is changed by carrying gameboy-python's `author-respect` branch down, in its own
gated session (the ladder, which tonight's compute rules do not allow). But the fetched dmg-sim
tree moved out of gameboy-python's `reference/` (to
`/data/backup/gameboy-python-furrtek-lineage-2026-11-26/`), and `external/gameboy-python/reference`
links there, so **until the carry-down, rung 4's `mooneye-dmg-quick/*` jobs cannot load their
boot ROM** (on both boards) and `--boot quick` fails. The carry-down: drop the `quick` boot
from `boot.rs`, `gameboy.rs`, `gameboy-jobs.rs`, `suite_jobs.py` and `suite_ref.py`, re-cite
the three comments above as upstream did, rerun rung 4 (one fewer job group) and record it in
docs/status.md. The die rung does not exist in this tier; nothing else in the ladder used the
lineage.

**SonoSooS's *Gameboy CPU internals*: set aside (Aubrey, 2026-11-26).** It credits
emu-russia/dmgcpu, which builds on msinger's dmg-schematics (furrtek-derived), so it is in the
lineage; the question sm83-python's provenance.md left open is answered. This repository never
pinned it. Its one citation, the `stop` doc comment in `src/board.rs`, now names Pan Docs' STOP
page with LIJI's flowchart and SameBoy `stop()`, as gameboy-python `ff3c643` does. Comment
only; no cycle changes, so no ladder run is owed for it.
