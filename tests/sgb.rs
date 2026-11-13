//! The HLE Super Game Boy (sgb.rs): synthetic packets through P1 writes and synthetic LCD lines
//! through the PPU taps, no ROMs (gameboy-python `tests/test_sgb.py`, transcribed; its PyPy
//! layout test and its Donkey Kong picture are not: the second is in the lockstep gate,
//! `scripts/cases-sgb.tsv`).

use gameboy_rust::sgb::{
    SuperGameBoy, BORDER_ROWS, FIRST_LINE, GREY_RAMP, SNES_W, WINDOW_X, WINDOW_Y,
};
use gameboy_rust::{load_cartridge, machine, Clock, GameBoy};

fn cartridge(sgb_flag: u8, licensee: u8) -> Vec<u8> {
    let mut rom = vec![0u8; 0x8000];
    rom[0x100..0x104].copy_from_slice(&[0x00, 0xC3, 0x00, 0x01]); // nop; jp $0100
    rom[0x146] = sgb_flag;
    rom[0x14B] = licensee;
    rom
}

fn machine_with(model: &str, boot: Option<Vec<u8>>, sgb_flag: u8, licensee: u8) -> GameBoy {
    let cart = load_cartridge(&cartridge(sgb_flag, licensee), Clock::Board).unwrap();
    let mut gb = GameBoy::new(cart, machine(model).unwrap(), boot).unwrap();
    gb.attach_sgb(GREY_RAMP).unwrap();
    gb
}

fn sgb_machine() -> GameBoy {
    machine_with("sgb", None, 0x03, 0x33)
}

fn sgb(gb: &GameBoy) -> &SuperGameBoy {
    gb.hw.sgb.as_deref().unwrap()
}

fn sgb_mut(gb: &mut GameBoy) -> &mut SuperGameBoy {
    gb.hw.sgb.as_deref_mut().unwrap()
}

/// One 16-byte packet as a game sends it: start pulse, 128 bits LSB first, stop bit.
fn send(gb: &mut GameBoy, data: &[u8], stop: u8) {
    assert_eq!(data.len(), 16);
    gb.joypad_write(0x30);
    gb.joypad_write(0x00);
    gb.joypad_write(0x30);
    let mut bits: Vec<u8> = data
        .iter()
        .flat_map(|byte| (0..8).map(move |n| byte >> n & 1))
        .collect();
    bits.push(stop);
    for bit in bits {
        gb.joypad_write(if bit != 0 { 0x10 } else { 0x20 });
        gb.joypad_write(0x30);
    }
}

/// A command of as many packets as its data needs (header byte = code * 8 + packets).
fn command(gb: &mut GameBoy, code: u8, data: &[u8]) {
    let packets = 1.max((data.len() + 1).div_ceil(16));
    let mut body = vec![code << 3 | packets as u8];
    body.extend_from_slice(data);
    body.resize(packets * 16, 0);
    for n in 0..packets {
        send(gb, &body[n * 16..n * 16 + 16], 0);
    }
}

/// Draw `shades` (160 x 144) as whole LCD frames, as the PPU taps deliver them.
fn show(gb: &mut GameBoy, shades: &[u8], frames: usize) {
    let unit = sgb_mut(gb);
    for _ in 0..frames {
        for y in 0..144 {
            unit.lcd_line(y, &shades[y * 160..y * 160 + 160]);
        }
        unit.lcd_frame();
    }
}

/// The SNES picture at layer coordinates (the screen shows the layers from line 1).
fn pixel(words: &[u16], x: usize, y: usize) -> u16 {
    words[(y - FIRST_LINE) * SNES_W + x]
}

fn gb_pixel(words: &[u16], x: usize, y: usize) -> u16 {
    pixel(words, WINDOW_X as usize + x, WINDOW_Y as usize + y)
}

fn le(words: &[u16]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

fn repeat(pattern: &[u8], times: usize) -> Vec<u8> {
    pattern
        .iter()
        .copied()
        .cycle()
        .take(pattern.len() * times)
        .collect()
}

/// The 160 x 144 shades that carry `data` (up to 4 KiB) by VRAM transfer: tile n of the data at
/// cell (n mod 20, n div 20), as a game shows $8000-$8FFF with BGP $E4.
fn tiles_screen(data: &[u8]) -> Vec<u8> {
    let mut data = data.to_vec();
    data.resize(4096, 0);
    let mut shades = vec![0u8; 160 * 144];
    for tile in 0..256 {
        for row in 0..8 {
            let (low, high) = (data[tile * 16 + row * 2], data[tile * 16 + row * 2 + 1]);
            for x in 0..8 {
                let shade = (low >> (7 - x) & 1) | (high >> (7 - x) & 1) << 1;
                shades[((tile / 20) * 8 + row) * 160 + (tile % 20) * 8 + x] = shade;
            }
        }
    }
    shades
}

/// A *_TRN command and the frames that carry its data.
fn transfer(gb: &mut GameBoy, code: u8, data: &[u8], arg: u8) {
    command(gb, code, &[arg]);
    show(gb, &tiles_screen(data), 2);
}

// ----- attaching ------------------------------------------------------------------------

#[test]
fn only_an_sgb_machine_takes_one() {
    let cart = load_cartridge(&cartridge(0x03, 0x33), Clock::Board).unwrap();
    let mut gb = GameBoy::new(cart, machine("dmg").unwrap(), None).unwrap();
    assert!(gb.attach_sgb(GREY_RAMP).is_err());
    assert!(sgb(&machine_with("sgb2", None, 0x03, 0x33)).enabled);
}

#[test]
fn power_on_picture_is_greys_in_a_black_border() {
    let mut gb = sgb_machine();
    show(&mut gb, &repeat(&[0, 1, 2, 3], 40 * 144), 1);
    let words = sgb(&gb).render();
    assert_eq!(words.len(), 256 * 224);
    let row: Vec<u16> = (0..4).map(|x| gb_pixel(&words, x, 0)).collect();
    assert_eq!(row, GREY_RAMP);
    assert!(pixel(&words, 0, 1) == 0 && pixel(&words, 255, 224) == 0);
}

// ----- the ICD2 -------------------------------------------------------------------------

#[test]
fn a_bad_stop_bit_drops_the_packet() {
    let mut gb = sgb_machine();
    command(&mut gb, 0x17, b"\x02"); // MASK_EN black
    assert_eq!(sgb(&gb).mask, 2);
    let mut packet = vec![0x17 << 3 | 1, 0];
    packet.resize(16, 0);
    send(&mut gb, &packet, 1);
    assert_eq!(sgb(&gb).mask, 2);
}

#[test]
fn mlt_req_and_the_player_id() {
    let mut gb = sgb_machine();
    gb.joypad_write(0x30);
    assert_eq!(gb.hw.joypad.read() & 0x0F, 0x0F); // one player: player 1's ID
    gb.joypad_write(0x10);
    gb.joypad_write(0x30);
    assert_eq!(gb.hw.joypad.read() & 0x0F, 0x0F); // one player: P15's edge changes nothing
    command(&mut gb, 0x11, b"\x01"); // two players
    gb.joypad_write(0x30);
    gb.joypad_write(0x10);
    gb.joypad_write(0x30);
    assert_eq!(gb.hw.joypad.read() & 0x0F, 0x0E); // player 2
    gb.joypad_write(0x10);
    gb.joypad_write(0x30);
    assert_eq!(gb.hw.joypad.read() & 0x0F, 0x0F); // back to player 1
    command(&mut gb, 0x11, b"\x03"); // four players
    for expected in [0x0E, 0x0D, 0x0C, 0x0F] {
        gb.joypad_write(0x10);
        gb.joypad_write(0x30);
        assert_eq!(gb.hw.joypad.read() & 0x0F, expected);
    }
}

#[test]
fn pads_drive_the_input_lines() {
    let mut gb = sgb_machine();
    gb.sgb_set_pressed("a", true, 0);
    gb.sgb_set_pressed("left", true, 0);
    gb.joypad_write(0x10); // buttons
    assert_eq!(gb.hw.joypad.read() & 0x0F, 0b1110);
    gb.joypad_write(0x20); // directions
    assert_eq!(gb.hw.joypad.read() & 0x0F, 0b1101);
    gb.sgb_set_pressed("a", false, 0);
    gb.joypad_write(0x10);
    assert_eq!(gb.hw.joypad.read() & 0x0F, 0x0F);
}

// ----- palettes and attributes ----------------------------------------------------------

#[test]
fn pal01_and_the_shared_colour_0() {
    let mut gb = sgb_machine();
    command(
        &mut gb,
        0x00,
        &le(&[0x1111, 0x0001, 0x0002, 0x0003, 0x0011, 0x0012, 0x0013]),
    );
    command(
        &mut gb,
        0x01,
        &le(&[0x2222, 0x0021, 0x0022, 0x0023, 0x0031, 0x0032, 0x0033]),
    ); // PAL23
    command(&mut gb, 0x06, &[0b0100_0100 | 3 << 4, 9]); // ATTR_DIV: rows; 9 is the line
                                                        // cells above row 9: palette 1; row 9: 3; below: 0
    show(&mut gb, &repeat(&[0, 1, 2, 3], 40 * 144), 1);
    let words = sgb(&gb).render();
    assert_eq!(gb_pixel(&words, 0, 0), 0x2222); // colour 0 is the last one sent, for all palettes
    let at = |y| [1, 2, 3].map(|x| gb_pixel(&words, x, y));
    assert_eq!(at(0), [0x0011, 0x0012, 0x0013]);
    assert_eq!(at(72), [0x0031, 0x0032, 0x0033]);
    assert_eq!(at(80), [0x0001, 0x0002, 0x0003]);
    assert_eq!(pixel(&words, 0, 1), 0); // no border loaded yet: black
}

#[test]
fn attr_blk_inside_edge_outside_and_the_exception() {
    let mut gb = sgb_machine();
    // set 1: outside only, palette 3, rectangle (31,31)-(31,31): every cell is outside it
    // set 2: inside only, palette 1, rectangle (2,2)-(5,5): the edge follows the inside
    // set 3: edge only, palette 2, rectangle (10,10)-(12,12)
    let sets = [
        4,
        3 << 4,
        31,
        31,
        31,
        31,
        1,
        1,
        2,
        2,
        5,
        5,
        2,
        2 << 2,
        10,
        10,
        12,
        12,
    ];
    let mut data = vec![3];
    data.extend_from_slice(&sets);
    command(&mut gb, 0x04, &data); // 19 data bytes: two packets
    let cells = &sgb(&gb).attr_map;
    assert!(cells[0] == 3 && cells[19 + 17 * 20] == 3);
    assert!(cells[2 * 20 + 2] == 1 && cells[3 * 20 + 3] == 1 && cells[5 * 20 + 5] == 1);
    assert_eq!(cells[6 * 20 + 6], 3);
    assert!(cells[10 * 20 + 10] == 2 && cells[12 * 20 + 11] == 2);
    assert_eq!(cells[11 * 20 + 11], 3); // the inside of an edge-only block is unchanged
}

#[test]
fn attr_lin_and_attr_chr() {
    let mut gb = sgb_machine();
    command(&mut gb, 0x05, &[2, 0x80 | 1 << 5 | 4, 2 << 5 | 7]); // row 4 -> 1, column 7 -> 2
    let map = &sgb(&gb).attr_map;
    assert!(map[4 * 20] == 1 && map[4 * 20 + 7] == 2);
    assert!(map[7] == 2 && map[0] == 0);
    // ATTR_CHR from (18, 0), left to right, 4 cells 3,2,1,0: wraps to row 1
    command(&mut gb, 0x07, &[18, 0, 4, 0, 0, 0b11_10_01_00]);
    assert_eq!(sgb(&gb).attr_map[18..22], [3, 2, 1, 0]);
    command(&mut gb, 0x07, &[0, 16, 3, 0, 1, 0b11_11_11_00]); // top to bottom from (0, 16)
    let map = &sgb(&gb).attr_map;
    assert_eq!([map[16 * 20], map[17 * 20], map[1]], [3, 3, 3]);
}

#[test]
fn pal_trn_attr_trn_and_pal_set() {
    let mut gb = sgb_machine();
    let mut colours = vec![0u8; 4096];
    colours[7 * 8..7 * 8 + 8].copy_from_slice(&le(&[0x0100, 0x0101, 0x0102, 0x0103])); // system palette 7
    colours[300 * 8..300 * 8 + 8].copy_from_slice(&le(&[0x0200, 0x0201, 0x0202, 0x0203])); // 300: a 9-bit ID
    transfer(&mut gb, 0x0B, &colours, 0); // PAL_TRN
    assert_eq!(sgb(&gb).system_palettes[7 * 4 + 2], 0x0102);
    let mut files = vec![0u8; 4050];
    files[3 * 90..4 * 90].fill(0b01_10_11_00); // ATF 3: cells 1,2,3,0 repeating
    transfer(&mut gb, 0x15, &files, 0); // ATTR_TRN
    command(&mut gb, 0x17, b"\x01"); // MASK_EN freeze
                                     // PAL_SET: palettes 0-3 = 7, 300, 7, 300; apply ATF 3, cancel the mask
    let mut data = le(&[7, 300, 7, 300]);
    data.push(0x80 | 0x40 | 3);
    command(&mut gb, 0x0A, &data);
    assert_eq!(sgb(&gb).mask, 0);
    assert_eq!(sgb(&gb).attr_map[..4], [1, 2, 3, 0]);
    show(&mut gb, &[1u8; 160 * 144], 1);
    let words = sgb(&gb).render();
    assert_eq!(gb_pixel(&words, 0, 0), 0x0201); // cell 0: palette 1 = system 300
    assert_eq!(gb_pixel(&words, 8, 0), 0x0101); // cell 1: palette 2 = system 7
    let cgram = &sgb(&gb).cgram;
    assert!(cgram[0] == cgram[48] && cgram[48] == 0x0100); // colour 0 from palette 0's system palette
}

// ----- the border, the mask ---------------------------------------------------------------

#[test]
fn chr_trn_pct_trn_border() {
    let mut gb = sgb_machine();
    let mut tile = [0u8; 32];
    tile[0..2].copy_from_slice(b"\x80\x00"); // row 0: the leftmost pixel colour 1 (plane 0)
    tile[16..18].copy_from_slice(b"\x00\x80"); // ... and colour 9 in its plane 3: colour 1 | 8 = 9
    let mut chr_data = vec![0u8; 32];
    chr_data.extend_from_slice(&tile); // tile 1
    transfer(&mut gb, 0x13, &chr_data, 0); // CHR_TRN, tiles $00-$7F
    assert!(!sgb(&gb).border_loaded); // tiles alone show nothing until a PCT_TRN
    let mut pct = vec![0u8; 4096];
    let mut entries = vec![0u16; 32 * BORDER_ROWS];
    entries[0] = 1 | 4 << 10; // row 0, column 0: tile 1, palette 4
    entries[1] = 1 | 5 << 10 | 0x4000 | 0x8000; // column 1: flipped both ways, palette 5
    entries[5 * 32 + 6] = 1 | 6 << 10; // over the Game Boy window's first cell
    pct[0..2 * entries.len()].copy_from_slice(&le(&entries));
    pct[0x800 + 9 * 2..0x800 + 9 * 2 + 2].copy_from_slice(&le(&[0x4444])); // palette 4, colour 9
    pct[0x820 + 9 * 2..0x820 + 9 * 2 + 2].copy_from_slice(&le(&[0x5555])); // palette 5
    pct[0x840 + 9 * 2..0x840 + 9 * 2 + 2].copy_from_slice(&le(&[0x6666])); // palette 6
    command(&mut gb, 0x00, &le(&[0x0123])); // colour 0: the backdrop
    transfer(&mut gb, 0x14, &pct, 0); // PCT_TRN
    assert!(sgb(&gb).border_loaded);
    let words = sgb(&gb).render();
    assert_eq!(pixel(&words, 0, 1), 0x0123); // row 1 of tile 1 is transparent: the backdrop
    assert_eq!(pixel(&words, 1, 1), 0x0123);
    assert_eq!(pixel(&words, 15, 7), 0x5555); // flipped: row 7, rightmost pixel
    assert_eq!(pixel(&words, 8, 7), 0x0123);
    // the border covers the Game Boy picture
    assert_eq!(pixel(&words, WINDOW_X as usize, WINDOW_Y as usize), 0x6666);
    // the layers' line 0 is not on the screen: the screen starts at line 1
    assert_eq!(words[0], 0x0123);
}

#[test]
fn mask_en() {
    let mut gb = sgb_machine();
    show(&mut gb, &[3u8; 160 * 144], 1);
    command(&mut gb, 0x17, b"\x01"); // freeze
    show(&mut gb, &[1u8; 160 * 144], 1);
    assert_eq!(gb_pixel(&sgb(&gb).render(), 0, 0), GREY_RAMP[3]);
    command(&mut gb, 0x17, b"\x02"); // black
    assert_eq!(gb_pixel(&sgb(&gb).render(), 0, 0), 0);
    command(&mut gb, 0x17, b"\x03"); // colour 0
    assert_eq!(gb_pixel(&sgb(&gb).render(), 0, 0), GREY_RAMP[0]);
    command(&mut gb, 0x17, b"\x00");
    show(&mut gb, &[1u8; 160 * 144], 1);
    assert_eq!(gb_pixel(&sgb(&gb).render(), 0, 0), GREY_RAMP[1]);
}

#[test]
fn a_transfer_reads_the_first_whole_frame_after_the_command() {
    let mut gb = sgb_machine();
    command(&mut gb, 0x0B, b""); // PAL_TRN, mid-frame
    show(&mut gb, &[0u8; 160 * 144], 1); // the frame in progress ends: not the data
    assert_ne!(sgb(&gb).transfer, 0);
    let mut colours = vec![0u8; 4096];
    colours[0..2].copy_from_slice(&le(&[0x7C00]));
    show(&mut gb, &tiles_screen(&colours), 1);
    assert!(sgb(&gb).transfer == 0 && sgb(&gb).system_palettes[0] == 0x7C00);
}

// ----- the header, save states --------------------------------------------------------------

#[test]
fn a_game_without_the_sgb_flag_is_not_obeyed() {
    let mut gb = machine_with("sgb", None, 0x00, 0x33);
    command(&mut gb, 0x17, b"\x02");
    assert_eq!(sgb(&gb).mask, 0);
    let mut gb = machine_with("sgb", None, 0x03, 0x01);
    command(&mut gb, 0x17, b"\x02");
    assert_eq!(sgb(&gb).mask, 0);
}

#[test]
fn header_packets_decide_while_the_boot_rom_runs() {
    // a boot ROM is mapped: it sends the header
    let mut gb = machine_with("sgb", Some(vec![0u8; 256]), 0x00, 0x33);
    assert!(!sgb(&gb).enabled);
    let mut header = [0u8; 84];
    header[0x146 - 0x104] = 0x03;
    header[0x14B - 0x104] = 0x33;
    for index in 0..6 {
        let mut packet = vec![0xF1 + 2 * index as u8, 0];
        packet.extend_from_slice(&header[index * 14..index * 14 + 14]);
        send(&mut gb, &packet, 0);
    }
    assert!(sgb(&gb).enabled);
}

#[test]
fn save_state_round_trip() {
    let mut gb = sgb_machine();
    command(&mut gb, 0x00, &le(&[0x1111, 2, 3, 4, 5, 6, 7]));
    command(&mut gb, 0x11, b"\x01");
    show(&mut gb, &[2u8; 160 * 144], 1);
    let blob = sgb(&gb).save_state();
    let mut other = sgb_machine();
    {
        let hw = &mut other.hw;
        let unit = hw.sgb.as_deref_mut().unwrap();
        let mut icd = gameboy_rust::sgb::Icd {
            joypad: &mut hw.joypad,
            boot_mapped: hw.boot_mapped,
            irq: false,
        };
        unit.load_state(&blob, &mut icd).unwrap();
    }
    assert_eq!(sgb(&other).save_state(), blob);
    assert_eq!(sgb(&other).render(), sgb(&gb).render());
    assert_eq!(sgb(&other).players, 1);
}

/// This board's own: the SGB rides in the board's save state (`GameBoy::save_state`).
#[test]
fn board_save_state_carries_the_sgb() {
    let mut gb = sgb_machine();
    command(&mut gb, 0x00, &le(&[0x1111, 2, 3, 4, 5, 6, 7]));
    command(&mut gb, 0x11, b"\x03");
    gb.run_frames(3, 1_000_000);
    let state = gb.save_state();
    let cart = load_cartridge(&cartridge(0x03, 0x33), Clock::Board).unwrap();
    let mut other = GameBoy::new(cart, machine("sgb").unwrap(), None).unwrap();
    other.load_state(&state).unwrap();
    assert!(other.hw.ppu.taps);
    assert_eq!(sgb(&other).save_state(), sgb(&gb).save_state());
    gb.run_frames(2, 1_000_000);
    other.run_frames(2, 1_000_000);
    assert_eq!(sgb(&other).save_state(), sgb(&gb).save_state());
}
