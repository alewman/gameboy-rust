//! Run test ROMs and read their result protocols (gameboy-python `testrom.py`; its
//! docs/validation.md).
//!
//! * **Mooneye**: B, C, D, E, H, L = 3, 5, 8, 13, 21, 34 at `LD B,B` is a pass, all $42 a fail.
//! * **blargg serial** (`cpu_instrs`, `instr_timing`, `mem_timing`): text on the serial port
//!   ending `Passed` or `Failed`.
//! * **blargg $A000** (`dmg_sound`, `oam_bug`, `mem_timing-2`): cartridge RAM $A001-$A003 =
//!   $DE $B0 $61, $A000 = $80 while running then the result code, text from $A004.
//! * **GBMicrotest**: $FF82 = $01 pass, $FF fail ($FF80 got, $FF81 expected) after about 2
//!   frames.

use crate::board::GameBoy;

pub const MOONEYE_PASS: [u8; 6] = [3, 5, 8, 13, 21, 34];
pub const MOONEYE_FAIL: [u8; 6] = [0x42; 6];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RomResult {
    /// "mooneye", "blargg-serial", "blargg-ram", "microtest", "timeout"
    pub protocol: &'static str,
    pub passed: bool,
    pub text: String,
    pub registers: [u8; 6],
    pub t_cycles: u64,
    pub frames: u64,
}

pub fn registers(gb: &GameBoy) -> [u8; 6] {
    let c = &gb.cpu;
    [c.b, c.c, c.d, c.e, c.h, c.l]
}

fn decode(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| if b < 0x80 { b as char } else { '\u{FFFD}' })
        .collect()
}

fn serial_text(gb: &GameBoy) -> String {
    decode(&gb.hw.serial.output)
}

fn result(gb: &GameBoy, protocol: &'static str, passed: bool, text: String) -> RomResult {
    RomResult {
        protocol,
        passed,
        text,
        registers: registers(gb),
        t_cycles: gb.hw.t,
        frames: gb.hw.ppu.frames,
    }
}

/// Run until `LD B,B` with Mooneye's registers, or the frame budget runs out.
pub fn run_mooneye(gb: &mut GameBoy, max_frames: u64) -> RomResult {
    let limit = gb.hw.t + max_frames * 70224;
    while gb.hw.t < limit {
        let pc = gb.cpu.pc;
        if gb.peek(pc) == 0x40 && !(gb.cpu.halted || gb.cpu.stopped) {
            let regs = registers(gb);
            if regs == MOONEYE_PASS || regs == MOONEYE_FAIL {
                let text = serial_text(gb);
                return result(gb, "mooneye", regs == MOONEYE_PASS, text);
            }
        }
        gb.step();
    }
    let text = serial_text(gb);
    result(gb, "timeout", false, text)
}

/// Run until the serial text ends with `Passed` or `Failed`.
pub fn run_blargg_serial(gb: &mut GameBoy, max_frames: u64) -> RomResult {
    let limit = gb.hw.t + max_frames * 70224;
    let mut check = 0;
    while gb.hw.t < limit {
        gb.step();
        check += 1;
        if check >= 2000 {
            // the text only changes on serial writes; look every so often
            check = 0;
            let text = serial_text(gb);
            let trimmed = text.trim_end();
            if trimmed.ends_with("Passed")
                || trimmed.ends_with("Failed")
                || text.contains("Passed all tests")
            {
                let passed = text.contains("Passed");
                return result(gb, "blargg-serial", passed, text);
            }
        }
    }
    let text = serial_text(gb);
    result(gb, "timeout", false, text)
}

/// Run until cartridge RAM holds blargg's signature and a result code other than $80.
pub fn run_blargg_ram(gb: &mut GameBoy, max_frames: u64) -> RomResult {
    let limit = gb.hw.t + max_frames * 70224;
    let mut check = 0;
    while gb.hw.t < limit {
        gb.step();
        check += 1;
        if check >= 5000 {
            check = 0;
            let ram = &gb.hw.cartridge.ram;
            if ram.len() > 4 && ram[1..4] == [0xDE, 0xB0, 0x61] && ram[0] != 0x80 {
                let tail = &ram[4..];
                let end = tail.iter().position(|&b| b == 0).unwrap_or(tail.len());
                let text = decode(&tail[..end]);
                if text.is_empty() {
                    // the status byte is cleared for a moment while the text is rewritten
                    continue;
                }
                let passed = ram[0] == 0;
                return result(gb, "blargg-ram", passed, text);
            }
        }
    }
    result(gb, "timeout", false, String::new())
}

/// GBMicrotest ROMs from the suite's early style: no $FF82 report, a flag byte written to $8000
/// in a loop instead ($55 when the DMG value came out as expected; their sources, DMG build).
pub const VRAM_FLAG_ROMS: [&str; 2] = ["ppu_spritex_vs_scx.gb", "004-tima_boot_phase.gb"];

/// Run a GBMicrotest ROM for a few frames and read $FF82 (or, for a `VRAM_FLAG_ROMS` ROM, the
/// flag at $8000).
pub fn run_microtest(gb: &mut GameBoy, frames: u64, vram_flag: bool) -> RomResult {
    gb.run_frames(frames, 50_000_000);
    if vram_flag {
        let flag = gb.peek(0x8000);
        return result(
            gb,
            "microtest",
            flag == 0x55,
            format!("flag ${flag:02X}, expected $55"),
        );
    }
    let r = gb.hw.hram[0x02];
    let text = format!(
        "got ${:02X} expected ${:02X}",
        gb.hw.hram[0], gb.hw.hram[1]
    );
    result(gb, "microtest", r == 1, text)
}
