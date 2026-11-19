//! Board tests that need no ROM files: a small program built in place (CI runs these; the
//! lockstep and suite rungs need the reference and the test ROMs and run by hand).

use gameboy_rust::cartridge::parse_header;
use gameboy_rust::{load_cartridge, Clock, GameBoy, CGB, DMG_B};

/// A 32 KiB ROM-only image: LCD on, a loop that counts in WRAM, writes the count to SB, starts a
/// transfer and halts until the serial interrupt.
fn program() -> Vec<u8> {
    let mut rom = vec![0u8; 0x8000];
    let code: &[u8] = &[
        0x3E, 0x08, // LD A,$08 (IE: serial)
        0xE0, 0xFF, // LDH ($FF),A
        0xFB, // EI
        0x21, 0x00, 0xC0, // LD HL,$C000
        0x34, // loop: INC (HL)
        0x7E, // LD A,(HL)
        0xE0, 0x01, // LDH ($01),A
        0x3E, 0x81, // LD A,$81
        0xE0, 0x02, // LDH ($02),A
        0x76, // HALT
        0x00, // NOP
        0x18, 0xF4, // JR loop
    ];
    rom[0x100..0x104].copy_from_slice(&[0x00, 0xC3, 0x50, 0x01]); // NOP; JP $0150
    rom[0x150..0x150 + code.len()].copy_from_slice(code);
    rom[0x58] = 0xD9; // RETI at the serial vector
    rom
}

#[test]
fn header_of_the_program() {
    let header = parse_header(&program());
    assert_eq!(header.board.name(), "rom");
    assert_eq!(header.rom_banks, 2);
}

#[test]
fn serial_output_counts() {
    let cart = load_cartridge(&program(), Clock::Board).unwrap();
    let mut gb = GameBoy::new(cart, DMG_B, None).unwrap();
    gb.run_frames(20, 10_000_000);
    let out = &gb.hw.serial.output;
    assert!(
        out.len() > 10,
        "{} bytes, pc {:04x}, frames {}",
        out.len(),
        gb.cpu.pc,
        gb.hw.ppu.frames
    );
    assert!(out.windows(2).all(|w| w[1] == w[0].wrapping_add(1)));
}

#[test]
fn save_state_round_trip() {
    for machine in [DMG_B, CGB] {
        let make = || {
            GameBoy::new(
                load_cartridge(&program(), Clock::Board).unwrap(),
                machine,
                None,
            )
        };
        let mut a = make().unwrap();
        a.run_frames(5, 10_000_000);
        let state = a.save_state();
        // a new console loaded from the state goes on exactly as the one that saved it, APU
        // included
        let mut c = make().unwrap();
        c.load_state(&state).unwrap();
        for gb in [&mut a, &mut c] {
            gb.run_frames(5, 10_000_000);
        }
        assert_eq!(a.save_state(), c.save_state());
        assert_eq!(a.hw.serial.output, c.hw.serial.output);
        assert_eq!(a.hw.ppu.frame, c.hw.ppu.frame);
        assert_eq!(a.hw.apu.capture_state(), c.hw.apu.capture_state());
    }
}
