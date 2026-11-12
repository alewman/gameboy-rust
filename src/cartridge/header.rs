//! The cartridge header at $0100-$014F (Pan Docs The_Cartridge_Header.md); gameboy-python
//! `cartridge/header.py`.

/// The board a header names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Board {
    Rom,
    Mbc1,
    Mbc2,
    Mmm01,
    Mbc3,
    Mbc5,
    Unknown,
}

impl Board {
    pub fn name(self) -> &'static str {
        match self {
            Board::Rom => "rom",
            Board::Mbc1 => "mbc1",
            Board::Mbc2 => "mbc2",
            Board::Mmm01 => "mmm01",
            Board::Mbc3 => "mbc3",
            Board::Mbc5 => "mbc5",
            Board::Unknown => "unknown",
        }
    }
}

/// header byte $0147 -> (board, has RAM, has battery, has timer, has rumble); Pan Docs table.
pub fn cartridge_type(code: u8) -> Option<(Board, bool, bool, bool, bool)> {
    use Board::*;
    Some(match code {
        0x00 => (Rom, false, false, false, false),
        0x01 => (Mbc1, false, false, false, false),
        0x02 => (Mbc1, true, false, false, false),
        0x03 => (Mbc1, true, true, false, false),
        0x05 => (Mbc2, false, false, false, false),
        0x06 => (Mbc2, false, true, false, false),
        0x08 => (Rom, true, false, false, false),
        0x09 => (Rom, true, true, false, false),
        0x0B => (Mmm01, false, false, false, false),
        0x0C => (Mmm01, true, false, false, false),
        0x0D => (Mmm01, true, true, false, false),
        0x0F => (Mbc3, false, true, true, false),
        0x10 => (Mbc3, true, true, true, false),
        0x11 => (Mbc3, false, false, false, false),
        0x12 => (Mbc3, true, false, false, false),
        0x13 => (Mbc3, true, true, false, false),
        0x19 => (Mbc5, false, false, false, false),
        0x1A => (Mbc5, true, false, false, false),
        0x1B => (Mbc5, true, true, false, false),
        0x1C => (Mbc5, false, false, false, true),
        0x1D => (Mbc5, true, false, false, true),
        0x1E => (Mbc5, true, true, false, true),
        _ => return None,
    })
}

/// header byte $0149 -> cartridge RAM bytes (Pan Docs)
pub fn ram_size(code: u8) -> usize {
    match code {
        1 => 0x800,
        2 => 0x2000,
        3 => 0x8000,
        4 => 0x20000,
        5 => 0x10000,
        _ => 0,
    }
}

pub const NINTENDO_LOGO_START: [u8; 16] = [
    0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83, 0x00, 0x0C, 0x00, 0x0D,
];

/// The parts of the header a board needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub title: String,
    pub cartridge_type: u8,
    pub board: Board,
    /// 16 KiB banks
    pub rom_banks: u32,
    pub ram_bytes: usize,
    pub has_battery: bool,
    pub has_timer: bool,
    pub has_rumble: bool,
    pub cgb_flag: u8,
    pub sgb_flag: u8,
    /// the byte stored at $014D
    pub header_checksum: u8,
    /// the boot ROM locks up if this is wrong
    pub checksum_ok: bool,
    pub logo_ok: bool,
}

impl Header {
    pub fn cgb_only(&self) -> bool {
        self.cgb_flag == 0xC0
    }

    pub fn cgb_enhanced(&self) -> bool {
        self.cgb_flag == 0x80 || self.cgb_flag == 0xC0
    }
}

/// Decode the header of `rom`; a ROM shorter than $0150 gets a zero header.
pub fn parse_header(rom: &[u8]) -> Header {
    let mut data = vec![0u8; 0x150];
    let n = rom.len().min(0x150);
    data[..n].copy_from_slice(&rom[..n]);
    let code = data[0x147];
    let (board, _ram, battery, timer, rumble) =
        cartridge_type(code).unwrap_or((Board::Unknown, false, false, false, false));
    let size_code = data[0x148];
    let banks = if size_code <= 8 { 2u32 << size_code } else { 2 };
    let mut checksum: u8 = 0;
    for &byte in &data[0x134..0x14D] {
        checksum = checksum.wrapping_sub(byte).wrapping_sub(1);
    }
    let raw = &data[0x134..0x144];
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    let title = raw[..end]
        .iter()
        .map(|&b| if b < 0x80 { b as char } else { '\u{FFFD}' })
        .collect();
    Header {
        title,
        cartridge_type: code,
        board,
        rom_banks: banks,
        ram_bytes: ram_size(data[0x149]),
        has_battery: battery,
        has_timer: timer,
        has_rumble: rumble,
        cgb_flag: data[0x143],
        sgb_flag: data[0x146],
        header_checksum: data[0x14D],
        checksum_ok: checksum == data[0x14D],
        logo_ok: data[0x104..0x114] == NINTENDO_LOGO_START,
    }
}
