//! The sound unit, behind the interface board.py drives (gb-apu-python's `APU`).
//!
//! PLACEHOLDER until gb-apu-rust passes its ladder (docs/handoff-brief.md rung 7): it keeps the
//! registers as written and wave RAM, answers NR52 with the power bit only (no channel status),
//! and makes no sound. Its register reads are NOT the reference's; the lockstep gate leaves the
//! values of reads of $FF10-$FF3F and $FF76-$FF77 out of its comparison until the crate lands
//! (docs/status.md). The interface is the one `board.py` calls: `read`, `write`,
//! `frame_tick`, `secondary_tick`, `tick`, `pcm`, `skip_boot`, `frame_clock_high`.

use serde::{Deserialize, Serialize};

/// bits that always read 1 per register $FF10-$FF26 (Pan Docs Audio_Registers.md, DMG)
const READ_MASKS: [u8; 0x17] = [
    0x80, 0x3F, 0x00, 0xFF, 0xBF, // NR10-NR14
    0xFF, 0x3F, 0x00, 0xFF, 0xBF, // $FF15 (unused), NR21-NR24
    0x7F, 0xFF, 0x9F, 0xFF, 0xBF, // NR30-NR34
    0xFF, 0xFF, 0x00, 0x00, 0xBF, // $FF1F (unused), NR41-NR44
    0x00, 0x00, 0x70, // NR50, NR51, NR52
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Apu {
    pub cgb: bool,
    pub registers: [u8; 0x17],
    pub wave_ram: [u8; 16],
    pub power: bool,
    /// the system counter's bit 12 at a register write (board.py sets it before each write)
    pub frame_clock_high: bool,
    pub capture: bool,
}

impl Apu {
    pub fn new(cgb: bool) -> Apu {
        Apu {
            cgb,
            registers: [0; 0x17],
            wave_ram: [0; 16],
            power: false,
            frame_clock_high: false,
            capture: false,
        }
    }

    pub fn read(&mut self, address: u16) -> u8 {
        if address >= 0xFF30 {
            return self.wave_ram[(address - 0xFF30) as usize];
        }
        if address >= 0xFF27 {
            return 0xFF;
        }
        if address == 0xFF26 {
            return 0x70 | if self.power { 0x80 } else { 0 };
        }
        let index = (address - 0xFF10) as usize;
        self.registers[index] | READ_MASKS[index]
    }

    pub fn write(&mut self, address: u16, value: u8) {
        if address >= 0xFF30 {
            self.wave_ram[(address - 0xFF30) as usize] = value;
            return;
        }
        if address >= 0xFF27 {
            return;
        }
        if address == 0xFF26 {
            self.power = value & 0x80 != 0;
            if !self.power {
                self.registers = [0; 0x17];
            }
            return;
        }
        if self.power {
            self.registers[(address - 0xFF10) as usize] = value;
        }
    }

    pub fn frame_tick(&mut self) {}

    pub fn secondary_tick(&mut self) {}

    pub fn tick(&mut self, _t: i32) {}

    /// PCM12/PCM34 ($FF76/$FF77, CGB).
    pub fn pcm(&self, _pair: usize) -> u8 {
        0
    }

    /// The state the DMG boot ROM leaves: powered, the registers it writes.
    pub fn skip_boot(&mut self) {
        self.power = true;
        for (address, value) in [
            (0x10, 0x80),
            (0x11, 0xBF),
            (0x12, 0xF3),
            (0x13, 0xFF),
            (0x14, 0xBF),
            (0x16, 0x3F),
            (0x17, 0x00),
            (0x18, 0xFF),
            (0x19, 0xBF),
            (0x1A, 0x7F),
            (0x1B, 0xFF),
            (0x1C, 0x9F),
            (0x1D, 0xFF),
            (0x1E, 0xBF),
            (0x20, 0xFF),
            (0x21, 0x00),
            (0x22, 0x00),
            (0x23, 0xBF),
            (0x24, 0x77),
            (0x25, 0xF3),
        ] {
            self.registers[address - 0x10] = value;
        }
    }

    /// Interleaved signed 16-bit stereo since the last call (none: a placeholder).
    pub fn take_samples(&mut self) -> Vec<i16> {
        Vec::new()
    }
}
