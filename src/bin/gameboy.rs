//! Run a Game Boy ROM (gameboy-python `__main__.py`, without the window and the trace):
//!
//!     gameboy ROM [--machine KEY] [--boot skip|real|quick|PATH] [--frames N]
//!                 [--protocol mooneye|blargg-serial|blargg-ram|microtest] [--max-frames N]
//!
//! Prints the serial text (or the protocol's text) and a summary line on stderr; exits 0 on a
//! protocol pass.

use std::path::{Path, PathBuf};

use gameboy_rust::boot::{find_boot_rom, quickboot, read_image};
use gameboy_rust::testrom::{run_blargg_ram, run_blargg_serial, run_microtest, run_mooneye};
use gameboy_rust::{load_cartridge, machine, Clock, GameBoy};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut rom: Option<PathBuf> = None;
    let mut machine_key = "dmg".to_string();
    let mut boot = "skip".to_string();
    let mut frames = 60u64;
    let mut max_frames: Option<u64> = None;
    let mut protocol: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        let mut value = || {
            i += 1;
            args.get(i).cloned().expect("missing value")
        };
        match a.as_str() {
            "--machine" => machine_key = value(),
            "--boot" => boot = value(),
            "--frames" => frames = value().parse().expect("frames"),
            "--max-frames" => max_frames = Some(value().parse().expect("max frames")),
            "--protocol" => protocol = Some(value()),
            _ => rom = Some(PathBuf::from(a)),
        }
        i += 1;
    }
    let rom = rom.expect("usage: gameboy ROM [options]");
    let m = machine(&machine_key).expect("unknown machine");
    let (image, label) = match boot.as_str() {
        "skip" => (None, "skip-boot state".to_string()),
        "real" => find_boot_rom(m.boot_rom, None),
        "quick" => (
            Some(std::fs::read(quickboot()).expect("quickboot")),
            "quickboot (dmg-sim)".to_string(),
        ),
        path => (
            Some(read_image(Path::new(path)).expect("boot image")),
            format!("boot ROM {path}"),
        ),
    };
    let data = read_image(&rom).expect("ROM");
    let cart = load_cartridge(&data, Clock::Wall).expect("cartridge");
    eprintln!(
        "[{:?}, {}; {}]",
        cart.header.title,
        cart.header.board.name(),
        label
    );
    let mut gb = GameBoy::new(cart, m, image).expect("board");
    if let Some(p) = protocol {
        let result = match p.as_str() {
            "mooneye" => run_mooneye(&mut gb, max_frames.unwrap_or(600)),
            "blargg-serial" => run_blargg_serial(&mut gb, max_frames.unwrap_or(6000)),
            "blargg-ram" => run_blargg_ram(&mut gb, max_frames.unwrap_or(6000)),
            "microtest" => run_microtest(&mut gb, max_frames.unwrap_or(4), false),
            _ => panic!("unknown protocol {p}"),
        };
        if !result.text.is_empty() {
            println!("{}", result.text.trim_end_matches('\n'));
        }
        eprintln!(
            "[{}: {}; regs {:?}; {} frames, {} T]",
            result.protocol,
            if result.passed { "PASS" } else { "FAIL" },
            result.registers,
            result.frames,
            result.t_cycles
        );
        std::process::exit(if result.passed { 0 } else { 1 });
    }
    for _ in 0..frames {
        gb.run_frames(1, 50_000_000);
    }
    let text: String = gb.hw.serial.output.iter().map(|&b| b as char).collect();
    if !text.is_empty() {
        println!("{}", text.trim_end_matches('\n'));
    }
    eprintln!("[{} frames, {} T]", gb.hw.ppu.frames, gb.hw.t);
}
