//! The Super Game Boy, high-level: what the SGB's SNES program does with a Game Boy's packets
//! (gameboy-python `sgb.py`, branch `sgb-hle` `a8bdf5e`; its docs/sgb.md is the transcriber's
//! guide).
//!
//! A real Super Game Boy is a Game Boy on a SNES cartridge. The ICD2 chip between them hands the
//! SNES each LCD line and each 16-byte command packet the Game Boy sends through P1, and drives
//! the Game Boy's joypad input lines; the SNES program (the "SGB BIOS", Nintendo's, in the SGB's
//! ROM) reads the packets, keeps the palettes, the attribute map and the border, and draws the
//! picture. snes-python runs that program for real (its `sgb.py`, the low-level SGB). This module
//! does its job directly, for a player that cannot run a SNES.
//!
//! Use:
//!
//! ```ignore
//! let mut gb = GameBoy::new(cartridge, machine("sgb").unwrap(), boot_rom)?;
//! gb.attach_sgb(GREY_RAMP)?;     // takes the joypad and PPU taps; nothing else changes
//! gb.run_frames(600, u64::MAX);
//! let words = gb.hw.sgb.as_ref().unwrap().render(); // 256 x 224 SNES colours (BGR555)
//! ```
//!
//! Only a Game Boy with an attached `SuperGameBoy` runs any of this: the DMG and CGB rows, and an
//! `sgb` row without one, behave exactly as before.
//!
//! Two layers, kept apart:
//!
//! - **ICD2** (hardware; the same rules as snes-python's `sgb.py`, which passes Mooneye's SGB
//!   boot tests and shows the real BIOS's pictures): the packet receiver on P1 writes (Pan Docs
//!   SGB_Command_Packet.md: a both-low start pulse, 128 bits LSB first, P14 low = 0, P15 low = 1,
//!   both lines high between bits, a 0 stop bit) and the multiplayer ID (Pan Docs
//!   SGB_Command_Multiplayer.md: with no packet coming, each P15 low-to-high edge selects the
//!   next player; with both lines high the input lines read $F minus the player).
//! - **BIOS** (high level): packets gathered into commands (the first packet's low 3 bits count
//!   them, 1-7) and each command's effect (Pan Docs SGB_Command_*.md, SGB_VRAM_Transfer.md,
//!   SGB_Color_Palettes.md, CC0). Behaviour seen in SameBoy's HLE SGB (Core/sgb.c, Expat; read,
//!   not copied) is cited where it settled a detail the documents leave open.
//!
//! The picture is the SNES's: 256 x 224, the Game Boy's 160 x 144 at (48, 40), colourised through
//! four 4-colour palettes picked per 8 x 8 cell by the attribute map, under a border of 4bpp
//! tiles in palettes 4-6. Colours are SNES words, `0bbbbbgggggrrrrr`. Colour 0 is the backdrop:
//! every shade-0 Game Boy pixel and every transparent border pixel shows `cgram[0]`.
//!
//! Not modelled (docs/sgb.md "Out of scope"): the BIOS's own border artwork, built-in palettes,
//! menus, boot animation and fades; SOUND/SOU_TRN; DATA_SND/DATA_TRN/JUMP (SNES code and data);
//! OBJ_TRN; ICON_EN, ATRC_EN, TEST_EN, PAL_PRI (accepted, no effect). Commands act the moment
//! their last packet's stop bit arrives; the real BIOS takes frames.
//!
//! Where Rust forced a difference:
//! - the reference keeps `self.gb` and `self.ppu` and installs itself as three callbacks
//!   (`joypad.on_write`, `ppu.line_out`, `ppu.frame_start`). Here the board owns it
//!   (`Hw::sgb: Option<Box<SuperGameBoy>>`, so it is part of the board's save state) and calls
//!   it: `p1_write` from the P1 write between the select change and the interrupt check, as the
//!   reference's `Joypad.write` does, and `lcd_line`/`lcd_frame` from the PPU's tap queue right
//!   after each PPU tick (`ppu::Tap`). `lcd_line` takes the LY the reference reads from the PPU.
//!   What the reference reaches through `self.gb` (the joypad's input lines, `boot_mapped`) comes
//!   in an [`Icd`]; the joypad interrupt `set_external` raises is returned through it.
//! - `GameBoy::attach_sgb` is the constructor's attaching half (taps on, `_update_lines`).

use serde::{Deserialize, Serialize};

use crate::joypad::Joypad;
use crate::machines::Machine;

pub const SNES_W: usize = 256;
pub const SNES_H: usize = 224;
pub const GB_W: usize = 160;
pub const GB_H: usize = 144;
/// the Game Boy picture's top-left corner in the border's 256 x 232 layer (tiles 6-25, rows 5-22)
pub const WINDOW_X: i32 = 48;
pub const WINDOW_Y: i32 = 40;
/// the screen's line 0 shows the layers' line 1: the BIOS leaves BG1VOFS at 0 and the S-PPU
/// draws lines VOFS+1 to VOFS+224 (Pan Docs SGB_Command_Border.md). snes-python running the
/// real BIOS agrees: its picture is this one's, one line up, 0 pixels off (docs/sgb.md).
pub const FIRST_LINE: usize = 1;
/// ... so the last screen line would show the map's 29th row; the BIOS blanks it (INIDISP $80)
/// almost always (Pan Docs SGB_Command_Border.md; snes-python: black in every settled frame of
/// Pokemon Red and Kirby's Dream Land 2). It is drawn black here.
pub const BLANK_LINES: usize = 1;
/// attribute cells (8 x 8 pixels each)
pub const CELLS_W: usize = 20;
pub const CELLS_H: usize = 18;
/// PCT_TRN's map: 28 rows and the extra one (Pan Docs SGB_Command_Border.md)
pub const BORDER_ROWS: usize = 29;

/// the default palettes (all four): a neutral grey ramp, white to black. The real BIOS starts
/// from its built-in palette 1-A, which is Nintendo's data and not reproduced (docs/sgb.md); a
/// frontend may pass other colours (`GameBoy::attach_sgb(palette)`). Games that never send a
/// palette (Tetris DX) keep this one, so their Game Boy picture differs from a real SGB's.
pub const GREY_RAMP: [u16; 4] = [0x7FFF, 0x56B5, 0x294A, 0x0000];

/// MLT_REQ's byte 1 (bits 1-0) -> the mask on the player number: one, two, (two), four players.
/// 2 is undocumented and taken as 3 (as snes-python's ICD2 model does for $6003 bits 5-4).
pub const PLAYER_MASK: [u8; 4] = [0, 1, 3, 3];

/// the joypad's button names -> bit of an ICD2 pad byte (Start Select B A Down Up Left Right,
/// 0 = pressed; snes-python sgb.py $6004-$6007)
pub const PAD_BITS: [(&str, u8); 8] = [
    ("right", 0),
    ("left", 1),
    ("up", 2),
    ("down", 3),
    ("a", 4),
    ("b", 5),
    ("select", 6),
    ("start", 7),
];

// command codes (Pan Docs SGB_Command_Summary.md)
pub const PAL01: u8 = 0x00;
pub const PAL23: u8 = 0x01;
pub const PAL03: u8 = 0x02;
pub const PAL12: u8 = 0x03;
pub const ATTR_BLK: u8 = 0x04;
pub const ATTR_LIN: u8 = 0x05;
pub const ATTR_DIV: u8 = 0x06;
pub const ATTR_CHR: u8 = 0x07;
pub const PAL_SET: u8 = 0x0A;
pub const PAL_TRN: u8 = 0x0B;
pub const MLT_REQ: u8 = 0x11;
pub const CHR_TRN: u8 = 0x13;
pub const PCT_TRN: u8 = 0x14;
pub const ATTR_TRN: u8 = 0x15;
pub const ATTR_SET: u8 = 0x16;
pub const MASK_EN: u8 = 0x17;
/// the commands from $1E up are the boot ROM's header packets ($F1, $F3 ... $FB), which the
/// BIOS reads to decide whether the game may use SGB commands at all
pub const HEADER_CODE: u8 = 0x1E;

// MASK_EN modes (Pan Docs SGB_Command_System.md)
pub const MASK_OFF: u8 = 0;
pub const MASK_FREEZE: u8 = 1;
pub const MASK_BLACK: u8 = 2;
pub const MASK_COLOR0: u8 = 3;

// what a pending VRAM transfer fills
pub const TRN_NONE: u8 = 0;
pub const TRN_PAL: u8 = 1;
pub const TRN_ATTR: u8 = 2;
pub const TRN_CHR_LOW: u8 = 3;
pub const TRN_CHR_HIGH: u8 = 4;
pub const TRN_PCT: u8 = 5;
/// frame starts from the command to the transfer: the frame in progress when the packet ends,
/// then one whole frame drawn after it, whose picture is the data (Pan Docs SGB_VRAM_Transfer.md:
/// "the actual transfer starts at the beginning of the next frame after the command")
pub const TRANSFER_DELAY: u8 = 2;
/// 256 tiles of the screen, 16 bytes each
pub const TRANSFER_BYTES: usize = 4096;
/// one attribute file: 18 rows of 20 two-bit cells
pub const ATF_BYTES: usize = 90;
/// $00-$2C
pub const ATF_COUNT: usize = 45;

/// A field kind of `STATE_LAYOUT`: one integer (or bool), bytes, or u16 words (little-endian).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    U8,
    Bytes,
    Words,
}

/// save_state's layout, in order: (name, kind, length). Plain data for a transcriber.
pub const STATE_LAYOUT: [(&str, Kind, usize); 26] = [
    ("p1", Kind::U8, 1),
    ("receiving", Kind::U8, 1),
    ("wait_high", Kind::U8, 1),
    ("bit_count", Kind::U8, 1),
    ("packet", Kind::Bytes, 16),
    ("players", Kind::U8, 1),
    ("player", Kind::U8, 1),
    ("pads", Kind::Bytes, 4),
    ("command", Kind::Bytes, 112),
    ("packets_due", Kind::U8, 1),
    ("packets_got", Kind::U8, 1),
    ("header", Kind::Bytes, 84),
    ("enabled", Kind::U8, 1),
    ("lcd", Kind::Bytes, GB_W * GB_H),
    ("shown", Kind::Bytes, GB_W * GB_H),
    ("mask", Kind::U8, 1),
    ("transfer", Kind::U8, 1),
    ("transfer_wait", Kind::U8, 1),
    ("cgram", Kind::Words, 128),
    ("system_palettes", Kind::Words, 512 * 4),
    ("attr_map", Kind::Bytes, CELLS_W * CELLS_H),
    ("attr_files", Kind::Bytes, ATF_BYTES * ATF_COUNT),
    ("tiles", Kind::Bytes, 256 * 32),
    ("tiles_pending", Kind::Bytes, 256 * 32),
    ("border_map", Kind::Words, 32 * BORDER_ROWS),
    ("border_loaded", Kind::U8, 1),
];
pub const STATE_VERSION: u8 = 1;

/// What the reference reaches through `self.gb`: the joypad whose input lines the ICD2 drives,
/// and whether the boot ROM is mapped (header packets). `irq` is set when `set_external` raised
/// the joypad interrupt; the board then makes the call.
pub struct Icd<'a> {
    pub joypad: &'a mut Joypad,
    pub boot_mapped: bool,
    pub irq: bool,
}

/// One field's value as `save_state` reads it.
enum Value<'a> {
    U8(u8),
    Bytes(&'a [u8]),
    Words(&'a [u16]),
}

/// One field as `load_state` writes it.
enum Slot<'a> {
    U8(&'a mut u8),
    Bool(&'a mut bool),
    Bytes(&'a mut [u8]),
    Words(&'a mut [u16]),
}

/// The ICD2 and an HLE of the SGB's SNES program, attached to one Game Boy.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SuperGameBoy {
    // -- the ICD2: packet receiver, players, pads
    /// the last select bits written (bits 5-4)
    pub p1: u8,
    pub receiving: bool,
    /// a pulse was seen: both lines must go high before the next bit
    pub wait_high: bool,
    pub bit_count: u8,
    pub packet: [u8; 16],
    /// MLT_REQ's byte 1, bits 1-0
    pub players: u8,
    /// the selected player, 0-3
    pub player: u8,
    /// per player, 0 = pressed
    pub pads: [u8; 4],
    // -- the BIOS: commands
    pub command: Vec<u8>,
    /// packets in the command being gathered
    pub packets_due: u8,
    pub packets_got: u8,
    /// $0104-$0157 as the boot ROM's header packets carry it
    pub header: Vec<u8>,
    /// commands are obeyed only for a header with the SGB flag ($0146 = 3) and the new
    /// licensee code ($014B = $33); the header packets decide it again when a boot ROM runs
    pub enabled: bool,
    // -- the picture
    /// the LCD's shades as lines end
    pub lcd: Vec<u8>,
    /// the last whole frame, or the frozen one
    pub shown: Vec<u8>,
    pub mask: u8,
    pub transfer: u8,
    pub transfer_wait: u8,
    /// the SNES's palettes: 0-3 the Game Boy's (colours 0-3 used), 4-6 the border's, 7 unused
    pub cgram: Vec<u16>,
    /// PAL_TRN's 512 x 4 colours
    pub system_palettes: Vec<u16>,
    /// palette 0-3 per cell
    pub attr_map: Vec<u8>,
    /// ATTR_TRN's 45 files
    pub attr_files: Vec<u8>,
    /// the border's 4bpp tiles, as shown
    pub tiles: Vec<u8>,
    /// CHR_TRN's, shown from the next PCT_TRN
    pub tiles_pending: Vec<u8>,
    pub border_map: Vec<u16>,
    /// before any PCT_TRN the border area is black
    pub border_loaded: bool,
}

impl SuperGameBoy {
    /// The constructor's state half (`GameBoy::attach_sgb` attaches it): `select` is the
    /// joypad's select bits, `boot_mapped` whether a boot ROM is mapped, `rom` the cartridge
    /// ROM.
    pub fn new(
        machine: &Machine,
        rom: &[u8],
        select: u8,
        boot_mapped: bool,
        palette: [u16; 4],
    ) -> Result<SuperGameBoy, String> {
        if machine.model != "sgb" && machine.model != "sgb2" {
            return Err(format!(
                "a Super Game Boy needs an sgb or sgb2 machine, not {}",
                machine.name
            ));
        }
        let mut cgram = vec![0u16; 128];
        for number in 0..4 {
            for shade in 0..4 {
                cgram[number * 16 + shade] = palette[shade];
            }
        }
        Ok(SuperGameBoy {
            // the last select bits written (bits 5-4). The SGB boot ROM leaves the ICD2 at $30
            // (both lines high after its header packets); skip-boot starts after the boot ROM, so
            // it starts there too: from the board's skip-boot select ($00) a game's first start
            // pulse ($00) would look like no change and its first packet (Pokemon Red's MLT_REQ
            // check) be lost
            p1: if boot_mapped { select } else { 0x30 },
            receiving: false,
            wait_high: false,
            bit_count: 0,
            packet: [0; 16],
            players: 0,
            player: 0,
            pads: [0xFF; 4],
            command: vec![0; 16 * 7],
            packets_due: 0,
            packets_got: 0,
            header: vec![0; 84],
            enabled: rom[0x146] == 0x03 && rom[0x14B] == 0x33,
            lcd: vec![0; GB_W * GB_H],
            shown: vec![0; GB_W * GB_H],
            mask: MASK_OFF,
            transfer: TRN_NONE,
            transfer_wait: 0,
            cgram,
            system_palettes: vec![0; 512 * 4],
            attr_map: vec![0; CELLS_W * CELLS_H],
            attr_files: vec![0; ATF_BYTES * ATF_COUNT],
            tiles: vec![0; 256 * 32],
            tiles_pending: vec![0; 256 * 32],
            border_map: vec![0; 32 * BORDER_ROWS],
            border_loaded: false,
        })
    }

    // ----- the ICD2 --------------------------------------------------------------------

    /// Every P1 write's select bits (bits 5-4): packets, and the next player.
    pub fn p1_write(&mut self, select: u8, icd: &mut Icd) {
        let before = self.p1;
        if select == before {
            return;
        }
        self.p1 = select;
        if select == 0x00 {
            // the start pulse
            self.receiving = true;
            self.wait_high = true;
            self.bit_count = 0;
            // (the reference's `for i in range(16): self.packet[i] = 0`)
            self.packet.fill(0);
        } else if self.wait_high {
            self.wait_high = false;
            if select != 0x30 {
                // a bit without both lines high first: not a packet
                self.receiving = false;
            }
        } else if self.receiving {
            let bit = if select == 0x10 { 1 } else { 0 }; // P15 low: 1; P14 low: 0
            if self.bit_count == 128 {
                // the stop bit
                self.receiving = false;
                if bit == 0 {
                    let packet = self.packet;
                    self.receive_packet(&packet, icd);
                }
            } else {
                if bit != 0 {
                    self.packet[(self.bit_count >> 3) as usize] |= 1 << (self.bit_count & 7);
                }
                self.bit_count += 1;
                self.wait_high = true;
            }
        } else if before & 0x20 == 0 && select & 0x20 != 0 {
            // P15 low to high: the next player
            self.player = (self.player + 1) & PLAYER_MASK[self.players as usize];
        }
        self.update_lines(icd);
    }

    /// Drive P1's input lines: the selected player's pad, or its ID with both lines high.
    pub fn update_lines(&mut self, icd: &mut Icd) {
        let pad = self.pads[self.player as usize];
        let select = self.p1;
        let mut lines = if select == 0x30 {
            0x0F - self.player
        } else {
            0x0F
        };
        if select & 0x10 == 0 {
            // directions
            lines &= pad & 0x0F;
        }
        if select & 0x20 == 0 {
            // buttons
            lines &= pad >> 4;
        }
        if icd.joypad.set_external(lines) {
            icd.irq = true;
        }
    }

    /// A host button change on one of the SNES pads (names as `joypad`'s).
    pub fn set_pressed(&mut self, name: &str, down: bool, player: usize, icd: &mut Icd) {
        let Some(&(_, number)) = PAD_BITS.iter().find(|(n, _)| *n == name) else {
            panic!("unknown button {name}");
        };
        let bit = 1u8 << number;
        if down {
            self.pads[player] &= !bit;
        } else {
            self.pads[player] |= bit;
        }
        self.update_lines(icd);
    }

    // ----- the LCD ---------------------------------------------------------------------

    /// PPU.line_out: one visible line's 160 shades, as it ends (`ly`: the PPU's LY, its line).
    pub fn lcd_line(&mut self, ly: usize, shades: &[u8]) {
        let start = ly * GB_W;
        self.lcd[start..start + GB_W].copy_from_slice(shades);
    }

    /// PPU.frame_start: line 0 begins, so the last frame is whole in `lcd`.
    pub fn lcd_frame(&mut self) {
        if self.mask != MASK_FREEZE {
            self.shown.copy_from_slice(&self.lcd);
        }
        if self.transfer_wait != 0 {
            self.transfer_wait -= 1;
            if self.transfer_wait == 0 {
                self.finish_transfer();
            }
        }
    }

    /// The first 4 KiB of the screen read back as Game Boy tiles (Pan Docs
    /// SGB_VRAM_Transfer.md): tile n is cell (n mod 20, n div 20), 16 bytes, two bit planes per
    /// row with the leftmost pixel in bit 7, as the ICD2 packs its row buffers.
    pub fn screen_data(&self) -> Vec<u8> {
        let mut data = vec![0u8; TRANSFER_BYTES];
        let lcd = &self.lcd;
        for tile in 0..TRANSFER_BYTES / 16 {
            let x0 = (tile % CELLS_W) * 8;
            let y0 = (tile / CELLS_W) * 8;
            for row in 0..8 {
                let mut low = 0u8;
                let mut high = 0u8;
                let at = (y0 + row) * GB_W + x0;
                for x in 0..8 {
                    let shade = lcd[at + x];
                    low = low << 1 | shade & 1;
                    high = high << 1 | shade >> 1 & 1;
                }
                data[tile * 16 + row * 2] = low;
                data[tile * 16 + row * 2 + 1] = high;
            }
        }
        data
    }

    // ----- commands --------------------------------------------------------------------

    /// One 16-byte packet from the ICD2: the first of a command, or the next of one.
    pub fn receive_packet(&mut self, packet: &[u8; 16], icd: &mut Icd) {
        if self.packets_got == self.packets_due {
            // a new command
            let mut count = packet[0] & 7;
            if packet[0] >> 3 >= HEADER_CODE {
                count = 1; // a header packet is one packet: its low bits are its index
            }
            if count == 0 {
                return; // a zero length: ignored (SameBoy ignores it too)
            }
            for i in 0..self.command.len() {
                self.command[i] = 0;
            }
            self.packets_due = count;
            self.packets_got = 0;
        }
        let at = self.packets_got as usize * 16;
        self.command[at..at + 16].copy_from_slice(packet);
        self.packets_got += 1;
        if self.packets_got == self.packets_due {
            self.packets_due = 0;
            self.packets_got = 0;
            let command = self.command.clone();
            self.execute(&command, icd);
        }
    }

    /// A whole command: header byte (code x 8 + packets), then its data.
    pub fn execute(&mut self, c: &[u8], icd: &mut Icd) {
        let code = c[0] >> 3;
        if code >= HEADER_CODE {
            self.header_packet(c, icd);
            return;
        }
        if !self.enabled {
            return;
        }
        match code {
            PAL01 => self.pal_pair(c, 0, 1),
            PAL23 => self.pal_pair(c, 2, 3),
            PAL03 => self.pal_pair(c, 0, 3),
            PAL12 => self.pal_pair(c, 1, 2),
            ATTR_BLK => self.attr_blk(c),
            ATTR_LIN => self.attr_lin(c),
            ATTR_DIV => self.attr_div(c),
            ATTR_CHR => self.attr_chr(c),
            PAL_SET => self.pal_set(c),
            PAL_TRN => self.start_transfer(TRN_PAL),
            MLT_REQ => {
                self.players = c[1] & 3;
                self.player &= PLAYER_MASK[self.players as usize];
                self.update_lines(icd);
            }
            CHR_TRN => self.start_transfer(if c[1] & 1 != 0 {
                TRN_CHR_HIGH
            } else {
                TRN_CHR_LOW
            }),
            PCT_TRN => self.start_transfer(TRN_PCT),
            ATTR_TRN => self.start_transfer(TRN_ATTR),
            ATTR_SET => {
                self.apply_atf((c[1] & 0x3F) as usize);
                if c[1] & 0x40 != 0 {
                    self.mask = MASK_OFF;
                }
            }
            MASK_EN => self.mask = c[1] & 3,
            // anything else (SOUND, SOU_TRN, DATA_SND, DATA_TRN, JUMP, OBJ_TRN, ICON_EN, ATRC_EN,
            // TEST_EN, PAL_PRI, $1A-$1D): no effect here
            _ => {}
        }
    }

    /// $F1, $F3 ... $FB: the boot ROM sends $0104-$014F, 14 bytes a packet (byte 1 is a
    /// checksum, not checked here). After $FB the BIOS knows whether to obey the game.
    fn header_packet(&mut self, c: &[u8], icd: &Icd) {
        if c[0] & 0xF1 != 0xF1 || !icd.boot_mapped {
            return;
        }
        let index = (c[0] >> 1 & 7) as usize;
        if index > 5 {
            return;
        }
        self.header[index * 14..index * 14 + 14].copy_from_slice(&c[2..16]);
        if c[0] == 0xFB {
            self.enabled = self.header[0x146 - 0x104] == 0x03 && self.header[0x14B - 0x104] == 0x33;
        }
    }

    /// Colour 0 is shared: it is the backdrop for every palette (Pan Docs
    /// SGB_Color_Palettes.md "Color 0 Restriction").
    fn set_color0(&mut self, color: u16) {
        for palette in 0..4 {
            self.cgram[palette * 16] = color;
        }
    }

    /// PAL01/PAL23/PAL03/PAL12: colour 0, then colours 1-3 of two palettes.
    fn pal_pair(&mut self, c: &[u8], first: usize, second: usize) {
        self.set_color0(c[1] as u16 | (c[2] as u16) << 8);
        for i in 0..3 {
            self.cgram[first * 16 + 1 + i] =
                (c[3 + 2 * i] as u16 | (c[4 + 2 * i] as u16) << 8) & 0x7FFF;
            self.cgram[second * 16 + 1 + i] =
                (c[9 + 2 * i] as u16 | (c[10 + 2 * i] as u16) << 8) & 0x7FFF;
        }
    }

    /// PAL_SET: four system palettes (9-bit IDs) into palettes 0-3; palette 0's colour 0
    /// becomes the shared one. Byte 9: bit 7 apply ATF (bits 5-0), bit 6 cancel MASK_EN.
    fn pal_set(&mut self, c: &[u8]) {
        for palette in 0..4 {
            let number =
                ((c[1 + 2 * palette] as usize) | (c[2 + 2 * palette] as usize) << 8) & 0x1FF;
            for shade in 1..4 {
                self.cgram[palette * 16 + shade] = self.system_palettes[number * 4 + shade];
            }
            if palette == 0 {
                self.set_color0(self.system_palettes[number * 4]);
            }
        }
        let flags = c[9];
        if flags & 0x80 != 0 {
            self.apply_atf((flags & 0x3F) as usize);
        }
        if flags & 0x40 != 0 {
            self.mask = MASK_OFF;
        }
    }

    /// ATTR_SET (and PAL_SET's flag): attribute file `number` becomes the map; numbers
    /// above $2C are invalid and change nothing (Pan Docs).
    fn apply_atf(&mut self, number: usize) {
        if number >= ATF_COUNT {
            return;
        }
        let base = number * ATF_BYTES;
        for cell in 0..CELLS_W * CELLS_H {
            let byte = self.attr_files[base + cell / 4];
            self.attr_map[cell] = byte >> (6 - 2 * (cell % 4)) & 3; // first cell in bits 7-6
        }
    }

    /// ATTR_BLK: rectangles, each with palettes for its inside, its edge (the surrounding
    /// character line) and its outside. Changing only the inside or only the outside changes
    /// the edge too, to the same palette (Pan Docs). At most 18 data sets fit in 7 packets.
    fn attr_blk(&mut self, c: &[u8]) {
        let count = (c[1] as usize).min(18);
        for n in 0..count {
            let at = 2 + n * 6;
            let (control, palettes) = (c[at], c[at + 1]);
            let (left, top, right, bottom) = (
                (c[at + 2] & 31) as usize,
                (c[at + 3] & 31) as usize,
                (c[at + 4] & 31) as usize,
                (c[at + 5] & 31) as usize,
            );
            let (inside, mut edge, outside) = (control & 1, control & 2, control & 4);
            let (inside_pal, mut edge_pal, outside_pal) =
                (palettes & 3, palettes >> 2 & 3, palettes >> 4 & 3);
            if inside != 0 && edge == 0 && outside == 0 {
                (edge, edge_pal) = (2, inside_pal);
            } else if outside != 0 && edge == 0 && inside == 0 {
                (edge, edge_pal) = (2, outside_pal);
            }
            for y in 0..CELLS_H {
                for x in 0..CELLS_W {
                    if x < left || x > right || y < top || y > bottom {
                        if outside != 0 {
                            self.attr_map[y * CELLS_W + x] = outside_pal;
                        }
                    } else if left < x && x < right && top < y && y < bottom {
                        if inside != 0 {
                            self.attr_map[y * CELLS_W + x] = inside_pal;
                        }
                    } else if edge != 0 {
                        self.attr_map[y * CELLS_W + x] = edge_pal;
                    }
                }
            }
        }
    }

    /// ATTR_LIN: whole rows or columns, one byte each (bits 4-0 line, 6-5 palette, bit 7
    /// horizontal); later ones overwrite earlier ones; lines off the screen do nothing.
    fn attr_lin(&mut self, c: &[u8]) {
        let count = (c[1] as usize).min(c.len() - 2);
        for n in 0..count {
            let data = c[2 + n];
            let (line, palette) = ((data & 31) as usize, data >> 5 & 3);
            if data & 0x80 != 0 {
                // a horizontal line: a row of cells
                if line < CELLS_H {
                    for x in 0..CELLS_W {
                        self.attr_map[line * CELLS_W + x] = palette;
                    }
                }
            } else if line < CELLS_W {
                // a vertical line: a column of cells
                for y in 0..CELLS_H {
                    self.attr_map[y * CELLS_W + line] = palette;
                }
            }
        }
    }

    /// ATTR_DIV: split at one row (bit 6 set) or column; cells before the line take bits
    /// 3-2, the line bits 5-4, cells after it bits 1-0.
    fn attr_div(&mut self, c: &[u8]) {
        let (after, before, on) = (c[1] & 3, c[1] >> 2 & 3, c[1] >> 4 & 3);
        let rows = c[1] & 0x40 != 0;
        let line = (c[2] & 31) as usize;
        for y in 0..CELLS_H {
            for x in 0..CELLS_W {
                let at = if rows { y } else { x };
                let palette = if at < line {
                    before
                } else if at == line {
                    on
                } else {
                    after
                };
                self.attr_map[y * CELLS_W + x] = palette;
            }
        }
    }

    /// ATTR_CHR: two-bit palettes for consecutive cells from (x, y), the first in bits 7-6,
    /// left to right (byte 5 = 0) or top to bottom (1), wrapping at the screen's edge. It stops
    /// at the last cell; a start off the screen does nothing (both as SameBoy, unverified).
    fn attr_chr(&mut self, c: &[u8]) {
        let (mut x, mut y) = (c[1] as usize, c[2] as usize);
        let count = (c[3] as usize | (c[4] as usize) << 8)
            .min(CELLS_W * CELLS_H)
            .min((c.len() - 6) * 4);
        let down = c[5] & 1 != 0;
        if x >= CELLS_W || y >= CELLS_H {
            return;
        }
        for n in 0..count {
            self.attr_map[y * CELLS_W + x] = c[6 + n / 4] >> (6 - 2 * (n % 4)) & 3;
            if down {
                y += 1;
                if y == CELLS_H {
                    (y, x) = (0, x + 1);
                    if x == CELLS_W {
                        return;
                    }
                }
            } else {
                x += 1;
                if x == CELLS_W {
                    (x, y) = (0, y + 1);
                    if y == CELLS_H {
                        return;
                    }
                }
            }
        }
    }

    // ----- VRAM transfers ----------------------------------------------------------------

    fn start_transfer(&mut self, kind: u8) {
        self.transfer = kind;
        self.transfer_wait = TRANSFER_DELAY;
    }

    fn finish_transfer(&mut self) {
        let data = self.screen_data();
        let kind = self.transfer;
        self.transfer = TRN_NONE;
        if kind == TRN_PAL {
            // 512 palettes x 4 colours
            for i in 0..512 * 4 {
                self.system_palettes[i] =
                    (data[2 * i] as u16 | (data[2 * i + 1] as u16) << 8) & 0x7FFF;
            }
        } else if kind == TRN_ATTR {
            // 45 attribute files, 90 bytes each ($FD2-$FFF unused)
            self.attr_files
                .copy_from_slice(&data[..ATF_BYTES * ATF_COUNT]);
        } else if kind == TRN_CHR_LOW {
            // tiles $00-$7F
            self.tiles_pending[0..4096].copy_from_slice(&data);
        } else if kind == TRN_CHR_HIGH {
            // tiles $80-$FF
            self.tiles_pending[4096..8192].copy_from_slice(&data);
        } else if kind == TRN_PCT {
            // $000-$73F: the map, 29 rows of 32 entries; $800-$85F: palettes 4-6
            for i in 0..32 * BORDER_ROWS {
                self.border_map[i] = data[2 * i] as u16 | (data[2 * i + 1] as u16) << 8;
            }
            for i in 0..48 {
                self.cgram[64 + i] =
                    (data[0x800 + 2 * i] as u16 | (data[0x801 + 2 * i] as u16) << 8) & 0x7FFF;
            }
            self.tiles.copy_from_slice(&self.tiles_pending);
            self.border_loaded = true;
        }
    }

    // ----- the picture -------------------------------------------------------------------

    /// The border's colour index at SNES pixel (x, y): 0 transparent, else palette * 16 +
    /// colour (map entry: bits 7-0 tile, 12-10 palette, 14 X flip, 15 Y flip; Pan Docs
    /// SGB_Command_Border.md). Tile numbers from $100 up are taken as transparent.
    pub fn border_pixel(&self, x: usize, y: usize) -> usize {
        let entry = self.border_map[(y >> 3) * 32 + (x >> 3)];
        if entry & 0x0300 != 0 {
            return 0;
        }
        let mut row = y & 7;
        if entry & 0x8000 != 0 {
            row = 7 - row;
        }
        let mut bit = 7 - (x & 7);
        if entry & 0x4000 != 0 {
            bit = 7 - bit;
        }
        let base = (entry & 0xFF) as usize * 32 + row * 2;
        let tiles = &self.tiles;
        let color = (tiles[base] >> bit & 1)
            | (tiles[base + 1] >> bit & 1) << 1
            | (tiles[base + 16] >> bit & 1) << 2
            | (tiles[base + 17] >> bit & 1) << 3;
        if color == 0 {
            return 0;
        }
        (entry >> 10 & 7) as usize * 16 + color as usize
    }

    /// The SNES picture: 256 x 224 BGR555 words, rows top to bottom.
    pub fn render(&self) -> Vec<u16> {
        let mut out = vec![0u16; SNES_W * SNES_H];
        let cgram = &self.cgram;
        let backdrop = cgram[0];
        let shown = &self.shown;
        for screen_y in 0..SNES_H - BLANK_LINES {
            // the blanked lines stay 0
            let y = screen_y + FIRST_LINE;
            let gy = y as i32 - WINDOW_Y;
            for x in 0..SNES_W {
                let index = if self.border_loaded {
                    self.border_pixel(x, y)
                } else {
                    0
                };
                let gx = x as i32 - WINDOW_X;
                let color = if index != 0 {
                    cgram[index]
                } else if (0..GB_W as i32).contains(&gx) && (0..GB_H as i32).contains(&gy) {
                    if self.mask == MASK_BLACK {
                        0
                    } else if self.mask == MASK_COLOR0 {
                        backdrop
                    } else {
                        let (gx, gy) = (gx as usize, gy as usize);
                        let shade = shown[gy * GB_W + gx] as usize;
                        let palette = self.attr_map[(gy >> 3) * CELLS_W + (gx >> 3)] as usize;
                        if shade != 0 {
                            cgram[palette * 16 + shade]
                        } else {
                            backdrop
                        }
                    }
                } else if self.border_loaded {
                    backdrop
                } else {
                    0
                };
                out[screen_y * SNES_W + x] = color;
            }
        }
        out
    }

    // ----- save states -------------------------------------------------------------------

    /// The field `name` of STATE_LAYOUT, read (the reference's `getattr`).
    fn get(&self, name: &str) -> Value<'_> {
        match name {
            "p1" => Value::U8(self.p1),
            "receiving" => Value::U8(self.receiving as u8),
            "wait_high" => Value::U8(self.wait_high as u8),
            "bit_count" => Value::U8(self.bit_count),
            "packet" => Value::Bytes(&self.packet),
            "players" => Value::U8(self.players),
            "player" => Value::U8(self.player),
            "pads" => Value::Bytes(&self.pads),
            "command" => Value::Bytes(&self.command),
            "packets_due" => Value::U8(self.packets_due),
            "packets_got" => Value::U8(self.packets_got),
            "header" => Value::Bytes(&self.header),
            "enabled" => Value::U8(self.enabled as u8),
            "lcd" => Value::Bytes(&self.lcd),
            "shown" => Value::Bytes(&self.shown),
            "mask" => Value::U8(self.mask),
            "transfer" => Value::U8(self.transfer),
            "transfer_wait" => Value::U8(self.transfer_wait),
            "cgram" => Value::Words(&self.cgram),
            "system_palettes" => Value::Words(&self.system_palettes),
            "attr_map" => Value::Bytes(&self.attr_map),
            "attr_files" => Value::Bytes(&self.attr_files),
            "tiles" => Value::Bytes(&self.tiles),
            "tiles_pending" => Value::Bytes(&self.tiles_pending),
            "border_map" => Value::Words(&self.border_map),
            "border_loaded" => Value::U8(self.border_loaded as u8),
            _ => unreachable!("no SGB field {name}"),
        }
    }

    /// The field `name` of STATE_LAYOUT, to be written (the reference's `setattr`).
    fn slot(&mut self, name: &str) -> Slot<'_> {
        match name {
            "p1" => Slot::U8(&mut self.p1),
            "receiving" => Slot::Bool(&mut self.receiving),
            "wait_high" => Slot::Bool(&mut self.wait_high),
            "bit_count" => Slot::U8(&mut self.bit_count),
            "packet" => Slot::Bytes(&mut self.packet),
            "players" => Slot::U8(&mut self.players),
            "player" => Slot::U8(&mut self.player),
            "pads" => Slot::Bytes(&mut self.pads),
            "command" => Slot::Bytes(&mut self.command),
            "packets_due" => Slot::U8(&mut self.packets_due),
            "packets_got" => Slot::U8(&mut self.packets_got),
            "header" => Slot::Bytes(&mut self.header),
            "enabled" => Slot::Bool(&mut self.enabled),
            "lcd" => Slot::Bytes(&mut self.lcd),
            "shown" => Slot::Bytes(&mut self.shown),
            "mask" => Slot::U8(&mut self.mask),
            "transfer" => Slot::U8(&mut self.transfer),
            "transfer_wait" => Slot::U8(&mut self.transfer_wait),
            "cgram" => Slot::Words(&mut self.cgram),
            "system_palettes" => Slot::Words(&mut self.system_palettes),
            "attr_map" => Slot::Bytes(&mut self.attr_map),
            "attr_files" => Slot::Bytes(&mut self.attr_files),
            "tiles" => Slot::Bytes(&mut self.tiles),
            "tiles_pending" => Slot::Bytes(&mut self.tiles_pending),
            "border_map" => Slot::Words(&mut self.border_map),
            "border_loaded" => Slot::Bool(&mut self.border_loaded),
            _ => unreachable!("no SGB field {name}"),
        }
    }

    /// Everything above as one blob, in STATE_LAYOUT's order, after a version byte. (The board's
    /// own save state carries the SGB through serde; this is the reference's format, which the
    /// lockstep gate hashes.)
    pub fn save_state(&self) -> Vec<u8> {
        let mut out = vec![STATE_VERSION];
        for (name, _kind, length) in STATE_LAYOUT {
            match self.get(name) {
                Value::U8(value) => out.push(value),
                Value::Bytes(value) => {
                    assert_eq!(value.len(), length);
                    out.extend_from_slice(value);
                }
                Value::Words(value) => {
                    // words
                    assert_eq!(value.len(), length);
                    for &word in value {
                        out.extend_from_slice(&[(word & 0xFF) as u8, (word >> 8) as u8]);
                    }
                }
            }
        }
        out
    }

    pub fn load_state(&mut self, data: &[u8], icd: &mut Icd) -> Result<(), String> {
        if data[0] != STATE_VERSION {
            return Err(format!(
                "SGB state version {}, expected {}",
                data[0], STATE_VERSION
            ));
        }
        let mut at = 1;
        for (name, _kind, length) in STATE_LAYOUT {
            match self.slot(name) {
                Slot::U8(value) => {
                    *value = data[at];
                    at += 1;
                }
                Slot::Bool(value) => {
                    *value = data[at] != 0;
                    at += 1;
                }
                Slot::Bytes(value) => {
                    value.copy_from_slice(&data[at..at + length]);
                    at += length;
                }
                Slot::Words(words) => {
                    for i in 0..length {
                        words[i] = data[at + 2 * i] as u16 | (data[at + 2 * i + 1] as u16) << 8;
                    }
                    at += 2 * length;
                }
            }
        }
        self.update_lines(icd);
        Ok(())
    }
}

/// SNES words as 8-bit RGB, each 5-bit channel widened `c << 3 | c >> 2` (as snes-python's
/// video.picture does, so the two pictures compare byte for byte).
pub fn rgb_bytes(words: &[u16]) -> Vec<u8> {
    let widen: Vec<u8> = (0..32u8).map(|c| c << 3 | c >> 2).collect();
    let mut out = vec![0u8; words.len() * 3];
    for (i, &word) in words.iter().enumerate() {
        out[3 * i] = widen[(word & 31) as usize];
        out[3 * i + 1] = widen[(word >> 5 & 31) as usize];
        out[3 * i + 2] = widen[(word >> 10 & 31) as usize];
    }
    out
}
