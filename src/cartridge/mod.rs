//! Cartridges: the connector protocol, the header, and the memory bank controllers
//! (gameboy-python `cartridge/__init__.py` and `cartridge/mbc.py`).
//!
//! `read`/`write` take the full 16-bit external-bus address and the board's T-cycle count; a
//! read returns `None` for an open bus. This module imports nothing from the rest of the board.
//!
//! Where Rust forced a difference: the reference's classes (`Cartridge` and its MBC subclasses)
//! are one struct with the mapper's registers in an enum, and the MBC3 clock's time source
//! (`clock`, wall time by default) is a [`Clock`]: `Board` reads the board's T-cycle count, the
//! deterministic source the lockstep gate gives both boards.

pub mod header;

use serde::{Deserialize, Serialize};

pub use header::{parse_header, Board, Header};

/// The MBC3 timer's time source, in seconds.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Clock {
    /// wall time (the reference's default, `time.time`)
    Wall,
    /// the board's T-cycles / 4194304 (the gate: `clock=lambda: gb.t / 4194304` on the reference)
    Board,
}

/// Mapper registers, one variant per board class of the reference.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Mapper {
    Rom,
    Mbc1(Mbc1),
    Mbc2(Mbc2),
    Mbc3(Mbc3),
    Mbc5(Mbc5),
    Mmm01(Mmm01),
}

/// A cartridge: a ROM, optional RAM, and its mapper (the reference's `Cartridge`, whose plain
/// form has no banking: ROM-only boards ignore writes to ROM space).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cartridge {
    #[serde(skip, default = "empty_header")]
    pub header: Header,
    #[serde(skip)]
    pub rom: Vec<u8>,
    pub ram: Vec<u8>,
    pub mapper: Mapper,
}

fn empty_header() -> Header {
    parse_header(&[])
}

fn has_logo_at(rom: &[u8], offset: usize) -> bool {
    rom.len() >= offset + 16 && rom[offset..offset + 16] == header::NINTENDO_LOGO_START
}

/// Identify the board from the header and build it.
///
/// Two boards cannot be told from the header alone and are found by the Nintendo logo
/// (gameboy-python docs/cartridge.md "Measured coverage"): an 8 Mbit MBC1 image with the logo
/// again at $40104 is a multicart (MBC1M wiring), and a ROM whose last 32 KiB hold a header with
/// the logo and an MMM01 type byte is an MMM01 compilation (its first game's header says
/// something else). `clock` is the MBC3 timer's time source.
pub fn load_cartridge(rom: &[u8], clock: Clock) -> Result<Cartridge, String> {
    let header = parse_header(rom);
    if rom.len() >= 0x8000 && has_logo_at(rom, rom.len() - 0x8000 + 0x104) {
        let menu = parse_header(&rom[rom.len() - 0x8000..]);
        if menu.board == Board::Mmm01 {
            return Ok(Cartridge::mmm01(rom, menu));
        }
    }
    Ok(match header.board {
        Board::Rom => Cartridge::plain(rom, header),
        Board::Mbc1 => {
            let multicart = rom.len() == 0x100000 && has_logo_at(rom, 0x40104);
            Cartridge::mbc1(rom, header, multicart)
        }
        Board::Mbc2 => Cartridge::mbc2(rom, header),
        Board::Mbc3 => Cartridge::mbc3(rom, header, clock),
        Board::Mbc5 => Cartridge::mbc5(rom, header),
        _ => {
            return Err(format!(
                "cartridge board {:?} (type ${:02X})",
                header.board.name(),
                header.cartridge_type
            ))
        }
    })
}

fn rom_mask(rom: &[u8]) -> usize {
    (rom.len() / 0x4000).max(1) - 1
}

impl Cartridge {
    fn base(rom: &[u8], header: Header) -> Cartridge {
        let mut padded = rom.to_vec();
        if padded.len() < 0x8000 {
            padded.resize(0x8000, 0);
        }
        let ram = vec![0u8; header.ram_bytes];
        Cartridge {
            header,
            rom: padded,
            ram,
            mapper: Mapper::Rom,
        }
    }

    pub fn plain(rom: &[u8], header: Header) -> Cartridge {
        Cartridge::base(rom, header)
    }

    pub fn mbc1(rom: &[u8], header: Header, multicart: bool) -> Cartridge {
        let mut cart = Cartridge::base(rom, header);
        cart.mapper = Mapper::Mbc1(Mbc1 {
            rom_mask: rom_mask(&cart.rom),
            ram_enabled: false,
            bank1: 1,
            bank2: 0,
            mode: 0,
            multicart,
        });
        cart
    }

    pub fn mbc5(rom: &[u8], header: Header) -> Cartridge {
        let mut cart = Cartridge::base(rom, header);
        cart.mapper = Mapper::Mbc5(Mbc5 {
            rom_mask: rom_mask(&cart.rom),
            ram_enabled: false,
            romb0: 1,
            romb1: 0,
            ramb: 0,
            rumble: false,
        });
        cart
    }

    pub fn mbc2(rom: &[u8], header: Header) -> Cartridge {
        let mut cart = Cartridge::base(rom, header);
        cart.ram = vec![0u8; 512];
        cart.mapper = Mapper::Mbc2(Mbc2 {
            rom_mask: rom_mask(&cart.rom),
            ram_enabled: false,
            romb: 1,
        });
        cart
    }

    pub fn mbc3(rom: &[u8], header: Header, clock: Clock) -> Cartridge {
        let mut cart = Cartridge::base(rom, header);
        // the highest RAM bank the select reaches: on a clock board with up to four banks, 4-7
        // select nothing (an MBC30's eight banks are all there; without a clock they mirror)
        let top_bank = if cart.header.has_timer && cart.ram.len() <= 0x8000 {
            3
        } else {
            7
        };
        let mut m = Mbc3 {
            rom_mask: rom_mask(&cart.rom),
            ram_enabled: false,
            romb: 1,
            select: 0,
            top_bank,
            clock,
            live: [0; 5],
            stamp: 0.0,
            frozen: 0.0,
            latched: [0; 5],
        };
        m.stamp = m.now(0); // a tick boundary: the clock ticks at stamp + 1, + 2, ...
        cart.mapper = Mapper::Mbc3(m);
        cart
    }

    pub fn mmm01(rom: &[u8], header: Header) -> Cartridge {
        let mut cart = Cartridge::base(rom, header);
        cart.mapper = Mapper::Mmm01(Mmm01 {
            banks: (cart.rom.len() / 0x4000).max(2),
            mapped: false,
            ram_enabled: false,
            ram_mask: 0,
            rom_low: 0,
            rom_mid: 0,
            rom_high: 0,
            ram_low: 0,
            ram_high: 0,
            mode_lock: false,
            rom_mask_bits: 0,
            multiplex: false,
            mode: 0,
        });
        cart
    }

    pub fn read(&mut self, address: u16, _t: u64) -> Option<u8> {
        let a = address as usize;
        let ram_len = self.ram.len();
        match &mut self.mapper {
            Mapper::Rom => {
                if a < 0x8000 {
                    return Some(self.rom[a]);
                }
                if (0xA000..0xC000).contains(&a) && ram_len > 0 {
                    return Some(self.ram[(a - 0xA000) % ram_len]);
                }
                None
            }
            Mapper::Mbc1(m) => {
                if a < 0x4000 {
                    return Some(self.rom[(m.rom_bank(false) << 14) | a]);
                }
                if a < 0x8000 {
                    return Some(self.rom[(m.rom_bank(true) << 14) | (a & 0x3FFF)]);
                }
                if (0xA000..0xC000).contains(&a) {
                    if m.ram_enabled && ram_len > 0 {
                        return Some(self.ram[m.ram_offset(a, ram_len)]);
                    }
                    return Some(0xFF);
                }
                None
            }
            Mapper::Mbc5(m) => {
                if a < 0x4000 {
                    return Some(self.rom[a]);
                }
                if a < 0x8000 {
                    let bank = ((m.romb1 << 8) | m.romb0) & m.rom_mask;
                    return Some(self.rom[(bank << 14) | (a & 0x3FFF)]);
                }
                if (0xA000..0xC000).contains(&a) {
                    if m.ram_enabled && ram_len > 0 {
                        return Some(self.ram[((m.ramb << 13) | (a & 0x1FFF)) % ram_len]);
                    }
                    return Some(0xFF);
                }
                None
            }
            Mapper::Mbc2(m) => {
                if a < 0x4000 {
                    return Some(self.rom[a]);
                }
                if a < 0x8000 {
                    return Some(self.rom[((m.romb & m.rom_mask) << 14) | (a & 0x3FFF)]);
                }
                if (0xA000..0xC000).contains(&a) {
                    return Some(if m.ram_enabled {
                        self.ram[a & 0x1FF] | 0xF0
                    } else {
                        0xFF
                    });
                }
                None
            }
            Mapper::Mbc3(m) => {
                if a < 0x4000 {
                    return Some(self.rom[a]);
                }
                if a < 0x8000 {
                    return Some(self.rom[((m.romb & m.rom_mask) << 14) | (a & 0x3FFF)]);
                }
                if (0xA000..0xC000).contains(&a) {
                    if !m.ram_enabled {
                        return Some(0xFF);
                    }
                    if m.select <= m.top_bank {
                        if ram_len > 0 {
                            return Some(self.ram[((m.select << 13) | (a & 0x1FFF)) % ram_len]);
                        }
                        return Some(0xFF);
                    }
                    if (0x08..=0x0C).contains(&m.select) && self.header.has_timer {
                        return Some(m.latched[m.select - 0x08] as u8);
                    }
                    return Some(0xFF);
                }
                None
            }
            Mapper::Mmm01(m) => {
                if a < 0x4000 {
                    return Some(self.rom[(m.rom_bank(false) << 14) | a]);
                }
                if a < 0x8000 {
                    return Some(self.rom[(m.rom_bank(true) << 14) | (a & 0x3FFF)]);
                }
                if (0xA000..0xC000).contains(&a) {
                    if m.ram_enabled && ram_len > 0 {
                        return Some(self.ram[m.ram_offset(a, ram_len)]);
                    }
                    return Some(0xFF);
                }
                None
            }
        }
    }

    pub fn write(&mut self, address: u16, value: u8, t: u64) {
        let a = address as usize;
        let ram_len = self.ram.len();
        let has_rumble = self.header.has_rumble;
        let has_timer = self.header.has_timer;
        match &mut self.mapper {
            Mapper::Rom => {
                if (0xA000..0xC000).contains(&a) && ram_len > 0 {
                    self.ram[(a - 0xA000) % ram_len] = value;
                }
            }
            Mapper::Mbc1(m) => {
                if a < 0x2000 {
                    m.ram_enabled = value & 0x0F == 0x0A;
                } else if a < 0x4000 {
                    m.bank1 = if value & 0x1F != 0 {
                        (value & 0x1F) as usize
                    } else {
                        1
                    };
                } else if a < 0x6000 {
                    m.bank2 = (value & 0x03) as usize;
                } else if a < 0x8000 {
                    m.mode = (value & 1) as usize;
                } else if (0xA000..0xC000).contains(&a) && m.ram_enabled && ram_len > 0 {
                    self.ram[m.ram_offset(a, ram_len)] = value;
                }
            }
            Mapper::Mbc5(m) => {
                if a < 0x2000 {
                    m.ram_enabled = value == 0x0A;
                } else if a < 0x3000 {
                    m.romb0 = value as usize;
                } else if a < 0x4000 {
                    m.romb1 = (value & 1) as usize;
                } else if a < 0x6000 {
                    m.ramb = (value & if has_rumble { 0x07 } else { 0x0F }) as usize;
                    if has_rumble {
                        m.rumble = value & 0x08 != 0;
                    }
                } else if (0xA000..0xC000).contains(&a) && m.ram_enabled && ram_len > 0 {
                    self.ram[((m.ramb << 13) | (a & 0x1FFF)) % ram_len] = value;
                }
            }
            Mapper::Mbc2(m) => {
                if a < 0x4000 {
                    if a & 0x100 != 0 {
                        m.romb = if value & 0x0F != 0 {
                            (value & 0x0F) as usize
                        } else {
                            1
                        };
                    } else {
                        m.ram_enabled = value & 0x0F == 0x0A;
                    }
                } else if (0xA000..0xC000).contains(&a) && m.ram_enabled {
                    self.ram[a & 0x1FF] = value & 0x0F;
                }
            }
            Mapper::Mbc3(m) => {
                if a < 0x2000 {
                    m.ram_enabled = value & 0x0F == 0x0A;
                } else if a < 0x4000 {
                    m.romb = if value & 0x7F != 0 {
                        (value & 0x7F) as usize
                    } else {
                        1
                    };
                } else if a < 0x6000 {
                    m.select = (value & 0x0F) as usize;
                } else if a < 0x8000 {
                    if has_timer {
                        m.latched = m.registers(t);
                    }
                } else if (0xA000..0xC000).contains(&a) && m.ram_enabled {
                    if m.select <= m.top_bank {
                        if ram_len > 0 {
                            self.ram[((m.select << 13) | (a & 0x1FFF)) % ram_len] = value;
                        }
                    } else if (0x08..=0x0C).contains(&m.select) && has_timer {
                        let select = m.select;
                        m.set_register(select, value, t);
                    }
                }
            }
            Mapper::Mmm01(m) => {
                if a < 0x2000 {
                    m.ram_enabled = value & 0x0F == 0x0A;
                    if !m.mapped {
                        m.ram_mask = ((value >> 4) & 3) as usize;
                        m.mapped = value & 0x40 != 0;
                    }
                } else if a < 0x4000 {
                    let keep = m.rom_mask_bits;
                    m.rom_low = (m.rom_low & keep) | ((value as usize) & 0x1F & !keep);
                    if !m.mapped {
                        m.rom_mid = ((value >> 5) & 3) as usize;
                    }
                } else if a < 0x6000 {
                    let lock = !m.ram_mask & 3;
                    m.ram_low = (m.ram_low & !lock & 3) | ((value as usize) & 3 & lock);
                    if !m.mapped {
                        m.ram_high = ((value >> 2) & 3) as usize;
                        m.rom_high = ((value >> 4) & 3) as usize;
                        m.mode_lock = value & 0x40 != 0;
                    }
                } else if a < 0x8000 {
                    if !m.mode_lock || !m.mapped {
                        m.mode = (value & 1) as usize;
                    }
                    if !m.mapped {
                        m.rom_mask_bits = (value & 0x3E) as usize;
                        m.multiplex = value & 0x40 != 0;
                    }
                } else if (0xA000..0xC000).contains(&a) && m.ram_enabled && ram_len > 0 {
                    self.ram[m.ram_offset(a, ram_len)] = value;
                }
            }
        }
    }

    /// Clocked parts (the MBC3 RTC) catch up lazily; nothing to do.
    pub fn tick_to(&mut self, _t: u64) {}

    /// The audio-in pin: no licensed board here drives it.
    pub fn vin(&self, _t: u64) -> i32 {
        0
    }

    /// Battery-backed RAM to persist, or None.
    pub fn save_ram(&mut self, t: u64) -> Option<Vec<u8>> {
        match &mut self.mapper {
            Mapper::Mbc2(_) => {
                if self.header.has_battery {
                    Some(self.ram.clone())
                } else {
                    None
                }
            }
            Mapper::Mbc3(m) => {
                // RAM, then (with a timer) the 48-byte clock trailer BGB, SameBoy and mGBA share
                if !(self.header.has_battery && (!self.ram.is_empty() || self.header.has_timer)) {
                    return None;
                }
                let mut data = self.ram.clone();
                if self.header.has_timer {
                    let live = m.registers(t);
                    for v in live.iter().chain(m.latched.iter()) {
                        data.extend_from_slice(&(*v as u32).to_le_bytes());
                    }
                    data.extend_from_slice(&(m.now(t) as i64).to_le_bytes());
                }
                Some(data)
            }
            _ => {
                if self.header.has_battery && !self.ram.is_empty() {
                    Some(self.ram.clone())
                } else {
                    None
                }
            }
        }
    }

    pub fn load_ram(&mut self, data: &[u8], t: u64) {
        let n = data.len().min(self.ram.len());
        let ram_len = self.ram.len();
        self.ram[..n].copy_from_slice(&data[..n]);
        if let Mapper::Mbc3(m) = &mut self.mapper {
            if self.header.has_timer && data.len() >= ram_len + RTC_TRAILER {
                let raw = &data[ram_len..ram_len + RTC_TRAILER];
                let words: Vec<i64> = (0..40)
                    .step_by(4)
                    .map(|i| u32::from_le_bytes(raw[i..i + 4].try_into().unwrap()) as i64)
                    .collect();
                for i in 0..5 {
                    m.live[i] = words[i] & 0xFF;
                    m.latched[i] = words[5 + i];
                }
                let saved_at = u64::from_le_bytes(raw[40..48].try_into().unwrap()) as f64;
                m.stamp = m.now(t);
                m.frozen = 0.0;
                if !m.halted() {
                    // the battery kept it running while the console was off
                    m.stamp -= (m.stamp - saved_at).max(0.0);
                    m.advance(t);
                }
            }
        }
    }
}

/// MBC1 (GBCTR mbc1 chapter, Pan Docs MBC1.md; Mooneye emulator-only/mbc1/* verify).
///
/// Registers: RAMG ($0000-$1FFF, enable on low nibble $A), BANK1 ($2000-$3FFF, 5 bits, 0 reads
/// as 1), BANK2 ($4000-$5FFF, 2 bits), MODE ($6000-$7FFF, 1 bit). The ROM address is
/// `BANK2:BANK1:A13..A0` for $4000-$7FFF and `BANK2:00000` for $0000-$3FFF in mode 1.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mbc1 {
    pub rom_mask: usize,
    pub ram_enabled: bool,
    pub bank1: usize,
    pub bank2: usize,
    pub mode: usize,
    /// MBC1M wiring: BANK1's bit 4 is not connected and BANK2 sits at address bit 18
    pub multicart: bool,
}

impl Mbc1 {
    fn rom_bank(&self, upper: bool) -> usize {
        let shift = if self.multicart { 4 } else { 5 };
        let bank1 = if self.multicart {
            self.bank1 & 0x0F
        } else {
            self.bank1
        };
        let high = if upper || self.mode != 0 {
            self.bank2 << shift
        } else {
            0
        };
        (high | if upper { bank1 } else { 0 }) & self.rom_mask
    }

    fn ram_offset(&self, address: usize, ram_len: usize) -> usize {
        let bank = if self.mode != 0 { self.bank2 } else { 0 };
        ((bank << 13) | (address & 0x1FFF)) % ram_len
    }
}

/// MBC5 (Pan Docs MBC5.md; Mooneye emulator-only/mbc5/* verify on a genuine chip).
///
/// RAMG ($0000-$1FFF, enabled only by exactly $0A), ROMB0 ($2000-$2FFF, 8 bits), ROMB1
/// ($3000-$3FFF, bit 0 = bank bit 8), RAMB ($4000-$5FFF, 4 bits; bit 3 drives the rumble motor
/// on rumble boards). Unlike MBC1, bank 0 can be selected for $4000-$7FFF.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mbc5 {
    pub rom_mask: usize,
    pub ram_enabled: bool,
    pub romb0: usize,
    pub romb1: usize,
    pub ramb: usize,
    pub rumble: bool,
}

/// MBC2 (Pan Docs MBC2.md; Mooneye emulator-only/mbc2/* verify on a genuine MBC2A chip).
///
/// One register file for $0000-$3FFF, told apart by address bit 8: clear = RAMG (enable on low
/// nibble $A), set = ROMB (4 bits, 0 reads as 1). Built-in 512 x 4-bit RAM at $A000-$A1FF,
/// echoed through $BFFF; the upper nibble of a read is undefined (here: 1s).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mbc2 {
    pub rom_mask: usize,
    pub ram_enabled: bool,
    pub romb: usize,
}

pub const RTC_TRAILER: usize = 48;

/// MBC3 with an optional real-time clock (Pan Docs MBC3.md; rtc3test and MBC3-Tester are
/// detectors only: gameboy-python docs/cartridge.md).
///
/// RAMG ($0000, $0A enables RAM *and* the clock registers), ROMB ($2000, 7 bits, 0 reads as 1),
/// RAMB/RTC select ($4000, 4 bits: $0-$3 a RAM bank, $8-$C a clock register; on a clock board
/// $4-$7 and $D-$F select nothing and read $FF), LATCH ($6000: any write copies the running
/// clock into the readable registers; Pan Docs asks for 0 then 1, but CasualPokePlayer's
/// latch-rtc-test latches with one write and rtc-invalid-banks-test gives the select's width and
/// holes, both as the Emulator Shootout expects them and as SameBoy does). The clock runs on
/// `clock` and stops while the halt bit is set.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mbc3 {
    pub rom_mask: usize,
    pub ram_enabled: bool,
    pub romb: usize,
    pub select: usize,
    pub top_bank: usize,
    pub clock: Clock,
    /// the five clock registers as the chip holds them (they can hold out-of-range values):
    /// seconds, minutes, hours, day low, control (bits 0, 6, 7)
    pub live: [i64; 5],
    /// a tick boundary: the clock ticks at stamp + 1, + 2, ...
    pub stamp: f64,
    /// the sub-second position kept while halted
    pub frozen: f64,
    pub latched: [i64; 5],
}

impl Mbc3 {
    fn now(&self, t: u64) -> f64 {
        match self.clock {
            Clock::Wall => std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0),
            Clock::Board => t as f64 / 4194304.0,
        }
    }

    pub fn halted(&self) -> bool {
        self.live[4] & 0x40 != 0
    }

    /// One second. Valid fields carry as usual; an out-of-range field counts on to its bit
    /// limit and wraps *without* carrying (rtc3test "Invalid rollovers").
    fn tick_once(&mut self) {
        let live = &mut self.live;
        if live[0] == 59 {
            live[0] = 0;
        } else if live[0] == 63 {
            live[0] = 0;
            return;
        } else {
            live[0] += 1;
            return;
        }
        if live[1] == 59 {
            live[1] = 0;
        } else if live[1] == 63 {
            live[1] = 0;
            return;
        } else {
            live[1] += 1;
            return;
        }
        if live[2] == 23 {
            live[2] = 0;
        } else if live[2] == 31 {
            live[2] = 0;
            return;
        } else {
            live[2] += 1;
            return;
        }
        let mut days = (live[3] | (live[4] & 1) << 8) + 1;
        if days == 512 {
            days = 0;
            live[4] |= 0x80; // the overflow flag stays set until a write clears it
        }
        live[3] = days & 0xFF;
        live[4] = (live[4] & !1) | (days >> 8);
    }

    fn advance(&mut self, t: u64) {
        if self.halted() {
            return;
        }
        let now = self.now(t);
        let ticks = (now - self.stamp) as i64;
        if ticks <= 0 {
            return;
        }
        let live = &mut self.live;
        if live[0] < 60 && live[1] < 60 && live[2] < 24 && ticks > 3600 {
            let days = live[3] | (live[4] & 1) << 8;
            let total = ((days * 24 + live[2]) * 60 + live[1]) * 60 + live[0] + ticks;
            let mut days = total / 86400;
            if days >= 512 {
                live[4] |= 0x80;
                days %= 512;
            }
            live[0] = total % 60;
            live[1] = total / 60 % 60;
            live[2] = total / 3600 % 24;
            live[3] = days & 0xFF;
            live[4] = (live[4] & !1) | (days >> 8);
        } else {
            for _ in 0..ticks.min(10_000_000) {
                self.tick_once();
            }
        }
        self.stamp += ticks as f64;
    }

    fn registers(&mut self, t: u64) -> [i64; 5] {
        self.advance(t);
        self.live
    }

    fn set_register(&mut self, register: usize, value: u8, t: u64) {
        self.advance(t);
        let value = value as i64;
        if register == 0x08 {
            self.live[0] = value & 0x3F;
            self.stamp = self.now(t); // a seconds write restarts the sub-second counter
        } else if register == 0x09 {
            self.live[1] = value & 0x3F;
        } else if register == 0x0A {
            self.live[2] = value & 0x1F;
        } else if register == 0x0B {
            self.live[3] = value;
        } else {
            let was_halted = self.halted();
            self.live[4] = value & 0xC1;
            if self.halted() && !was_halted {
                self.frozen = self.now(t) - self.stamp;
            } else if was_halted && !self.halted() {
                self.stamp = self.now(t) - self.frozen;
            }
        }
    }
}

/// MMM01 multi-game boards (Pan Docs MMM01.md; no judge is pinned: documents only).
///
/// Starts "unmapped": the last 32 KiB of the ROM (the menu) sits at $0000-$7FFF. The menu writes
/// the game-select bits and the masks, then sets the Mapping Enable bit, after which the chip
/// acts as an MBC1 over the selected game and ignores the extra bits. Multiplex wiring is
/// included.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mmm01 {
    pub banks: usize,
    pub mapped: bool,
    pub ram_enabled: bool,
    /// RAM Bank Mask (2 bits)
    pub ram_mask: usize,
    /// 5 bits, reads as 1 when 0 after masking
    pub rom_low: usize,
    pub rom_mid: usize,
    pub rom_high: usize,
    pub ram_low: usize,
    pub ram_high: usize,
    pub mode_lock: bool,
    /// ROM Bank Mask (5 bits, bit 0 always 0)
    pub rom_mask_bits: usize,
    pub multiplex: bool,
    pub mode: usize,
}

impl Mmm01 {
    fn bank_low(&self) -> usize {
        let masked = self.rom_low & !self.rom_mask_bits & 0x1F;
        if masked == 0 {
            self.rom_low | 1
        } else {
            self.rom_low
        }
    }

    fn rom_bank(&self, upper: bool) -> usize {
        if !self.mapped {
            return (self.banks - 2 + usize::from(upper)) % self.banks;
        }
        let (mid, low);
        if self.multiplex {
            let mut m = if upper || self.mode != 0 {
                self.ram_low
            } else {
                self.ram_low & !self.ram_mask & 3
            };
            if !upper && self.mode == 0 {
                m = self.ram_low & !self.ram_mask & 3;
            }
            mid = m;
            low = if upper {
                self.bank_low()
            } else {
                self.rom_low & !self.rom_mask_bits & 0x1F
            };
        } else {
            mid = self.rom_mid;
            low = if upper {
                self.bank_low()
            } else {
                self.rom_low & !self.rom_mask_bits & 0x1F
            };
        }
        ((self.rom_high << 7) | (mid << 5) | low) % self.banks
    }

    fn ram_offset(&self, address: usize, ram_len: usize) -> usize {
        let bank = if self.multiplex {
            (self.ram_high << 2) | self.rom_mid
        } else {
            let low = if self.mode != 0 {
                self.ram_low
            } else {
                self.ram_low & !self.ram_mask & 3
            };
            (self.ram_high << 2) | low
        };
        ((bank << 13) | (address & 0x1FFF)) % ram_len
    }
}
