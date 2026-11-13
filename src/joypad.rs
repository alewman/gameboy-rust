//! The joypad matrix at $FF00 (Pan Docs Joypad_Input.md); gameboy-python `joypad.py`.
//!
//! Bits 5 and 4 select the button row and the direction row (active low); bits 3-0 read the
//! four lines of the selected rows, 0 = pressed; bits 7-6 read 1. Any high-to-low transition on
//! a selected input raises IF bit 4.
//!
//! On a Super Game Boy no buttons are wired: the ICD2 drives the four input lines (`external`)
//! and watches every write's select bits for command packets and the multiplayer ID (`on_write`;
//! Pan Docs SGB_Command_Packet.md, SGB_Command_Multiplayer.md; SameBoy `GB_icd_set_joyp`).
//!
//! Where Rust forced a difference: the reference raises IF bit 4 through a callback; here the
//! methods that can raise it return true and the board makes the call. The reference's
//! `on_write` callback is the board's: `write` is split into `write_select` and `fell`, and the
//! board calls the attached `SuperGameBoy::p1_write` between them, where `write` calls
//! `on_write`.

use serde::{Deserialize, Serialize};

pub const BUTTONS: [&str; 4] = ["a", "b", "select", "start"];
pub const DIRECTIONS: [&str; 4] = ["right", "left", "up", "down"];

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Joypad {
    /// bits 5-4: both rows selected at power-on, so P1 reads $CF
    pub select: u8,
    /// pressed buttons: bit i of the low nibble = BUTTONS[i], of the high nibble = DIRECTIONS[i]
    pub pressed: u8,
    /// the input lines' low nibble, driven by an ICD2
    pub external: Option<u8>,
}

impl Joypad {
    pub fn new() -> Joypad {
        Joypad::default()
    }

    fn lines(&self) -> u8 {
        if let Some(lines) = self.external {
            return lines;
        }
        let mut value = 0x0Fu8;
        if self.select & 0x20 == 0 {
            value &= !(self.pressed & 0x0F); // buttons selected
        }
        if self.select & 0x10 == 0 {
            value &= !(self.pressed >> 4); // directions selected
        }
        value & 0x0F
    }

    pub fn read(&self) -> u8 {
        0xC0 | self.select | self.lines()
    }

    #[must_use]
    pub fn write(&mut self, value: u8) -> bool {
        let before = self.write_select(value);
        self.fell(before)
    }

    /// `write`'s first half: the new select bits; returns the input lines before them.
    pub fn write_select(&mut self, value: u8) -> u8 {
        let before = self.lines();
        self.select = value & 0x30;
        before
    }

    /// `write`'s second half: an input line fell since `before` (IF bit 4).
    #[must_use]
    pub fn fell(&self, before: u8) -> bool {
        before & !self.lines() != 0
    }

    /// The ICD2 sets the input lines (low nibble, 0 = low); a line falling raises IF bit 4.
    #[must_use]
    pub fn set_external(&mut self, lines: u8) -> bool {
        let before = self.lines();
        self.external = Some(lines & 0x0F);
        before & !(lines & 0x0F) != 0
    }

    /// The bit of `pressed` for a button name.
    pub fn bit(name: &str) -> Option<u8> {
        if let Some(i) = BUTTONS.iter().position(|b| *b == name) {
            return Some(1 << i);
        }
        DIRECTIONS
            .iter()
            .position(|b| *b == name)
            .map(|i| 1 << (4 + i))
    }

    /// A host-side button change; raises the joypad interrupt on a selected press.
    #[must_use]
    pub fn set_pressed(&mut self, name: &str, down: bool) -> bool {
        let before = self.lines();
        if let Some(bit) = Self::bit(name) {
            if down {
                self.pressed |= bit;
            } else {
                self.pressed &= !bit;
            }
        }
        before & !self.lines() != 0
    }
}
