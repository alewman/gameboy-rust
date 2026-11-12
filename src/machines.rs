//! The machines, as data: one row per model and revision (gameboy-python `machines.py`).
//!
//! DMG-01 with a DMG-CPU B is the reference model (decided 2026-10-05; gameboy-python
//! docs/start-here.md "Why DMG-CPU B"): it is the only revision with die-level models and it is
//! in Mooneye's fleet. Every other row is present so that adding it later is data, not a
//! refactor, and is marked `certified: false` until a suite has judged it. Each quirk field names
//! its source and tier.
//!
//! Revision letters follow Mooneye's naming (`mooneye-test-suite/README.markdown`).

/// 4.194304 MHz crystal (2**22 Hz); a dot (PPU clock) is one T-cycle, an M-cycle is four.
pub const CLOCK_HZ: u32 = 4_194_304;
pub const DOTS_PER_LINE: i32 = 456;
pub const LINES_PER_FRAME: i32 = 154;
/// 70,224 (Pan Docs Rendering.md)
pub const DOTS_PER_FRAME: i32 = DOTS_PER_LINE * LINES_PER_FRAME;

/// What $FEA0-$FEFF read (Pan Docs Memory_Map.md "FEA0-FEFF range").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fea0Read {
    /// DMG/MGB/SGB
    ZeroWhenFree,
    /// CGB 0-D
    RevisionMaskedRam,
    /// CGB E
    HighNibbleTwice,
}

/// One Game Boy model and revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Machine {
    pub name: &'static str,
    /// "dmg", "mgb", "sgb", "sgb2", "cgb", "agb"
    pub model: &'static str,
    /// "0", "A", "B", "C"; CGB "0".."E"
    pub revision: &'static str,
    /// which No-Intro boot ROM image boots it (gameboy-python docs/start-here.md "Boot ROMs")
    pub boot_rom: &'static str,
    /// monochrome PPU model; CGB hardware runs a DMG-compatible mode (KEY0) on top
    pub cgb_hardware: bool,
    /// the OAM corruption bug (DMG/MGB/SGB yes, CGB/AGB no: Pan Docs OAM_Corruption_Bug.md;
    /// hardware-tested by blargg oam_bug on DMG-C, hardware-verified)
    pub oam_bug: bool,
    /// disabling the timer while the selected counter bit is 1 ticks TIMA (DMG yes, CGB no:
    /// Pan Docs Timer_Obscure_Behaviour.md; Mooneye timer/ tests, hardware-verified)
    pub tac_disable_glitch: bool,
    /// on monochrome models a STAT write acts as if $FF were written for one M-cycle (Pan Docs
    /// STAT.md "Spurious STAT interrupts"; research documentation)
    pub stat_write_glitch: bool,
    /// wave RAM readable/writable only on the cycle the channel reads it while on (DMG yes, CGB
    /// no: blargg dmg_sound 09/12 vs cgb_sound, hardware-verified)
    pub wave_ram_while_on_glitch: bool,
    /// what $FEA0-$FEFF read
    pub fea0_read: Fea0Read,
    /// stat_irq_blocking (Mooneye ppu/stat_irq_blocking) fails on DMG 0: the STAT line is
    /// re-triggered while another source holds it (hardware-verified in the test's header)
    pub stat_blocking: bool,
    /// the boot ROM writes $FF (MGB) or $01 (DMG) to $FF50
    pub boot_disable_value: u8,
    /// has the machine been judged by a suite? Rows below are data only until then.
    pub certified: bool,
}

const BASE: Machine = Machine {
    name: "",
    model: "",
    revision: "",
    boot_rom: "",
    cgb_hardware: false,
    oam_bug: true,
    tac_disable_glitch: true,
    stat_write_glitch: true,
    wave_ram_while_on_glitch: true,
    fea0_read: Fea0Read::ZeroWhenFree,
    stat_blocking: true,
    boot_disable_value: 0x01,
    certified: false,
};

const fn dmg(name: &'static str, revision: &'static str, certified: bool) -> Machine {
    let zero = matches!(revision.as_bytes(), [b'0']);
    Machine {
        name,
        model: "dmg",
        revision,
        boot_rom: if zero { "DMG0" } else { "DMG" },
        stat_blocking: !zero,
        certified,
        ..BASE
    }
}

pub const DMG_B: Machine = dmg("DMG-CPU B", "B", true);
pub const DMG_0: Machine = dmg("DMG-CPU 0", "0", false);
pub const DMG_A: Machine = dmg("DMG-CPU A", "A", false);
pub const DMG_C: Machine = dmg("DMG-CPU C", "C", false);
pub const MGB: Machine = Machine {
    name: "MGB",
    model: "mgb",
    revision: "-",
    boot_rom: "MGB",
    boot_disable_value: 0xFF,
    ..BASE
};
pub const SGB: Machine = Machine {
    name: "SGB",
    model: "sgb",
    revision: "-",
    boot_rom: "SGB",
    ..BASE
};
pub const SGB2: Machine = Machine {
    name: "SGB2",
    model: "sgb2",
    revision: "-",
    boot_rom: "SGB2",
    ..BASE
};
/// CGB hardware: no OAM bug, no STAT write glitch. Reference revision CGB E (Aubrey
/// 2026-10-05; SameSuite's APU suite passes in full only on E). Phase 2.
pub const CGB: Machine = Machine {
    name: "CPU CGB E",
    model: "cgb",
    revision: "E",
    boot_rom: "CGB",
    cgb_hardware: true,
    oam_bug: false,
    // Mooneye timer/rapid_toggle passes on CGB with it (Pan Docs says no)
    tac_disable_glitch: true,
    stat_write_glitch: false,
    wave_ram_while_on_glitch: false,
    fea0_read: Fea0Read::HighNibbleTwice,
    ..BASE
};

/// The machine table's keys, in the reference's order.
pub const MACHINES: [(&str, Machine); 9] = [
    ("dmg", DMG_B),
    ("dmg-b", DMG_B),
    ("dmg-0", DMG_0),
    ("dmg-a", DMG_A),
    ("dmg-c", DMG_C),
    ("mgb", MGB),
    ("sgb", SGB),
    ("sgb2", SGB2),
    ("cgb", CGB),
];

/// The row for a key of `MACHINES`.
pub fn machine(key: &str) -> Option<Machine> {
    MACHINES.iter().find(|(k, _)| *k == key).map(|(_, m)| *m)
}
