//! The lockstep gate's board side: the same lines as `scripts/gameboy_trace.py` writes from
//! the reference (see its docstring for the format and the options).
//!
//!     gameboy-trace ROM --slices N [--machine KEY] [--boot skip|real|PATH]
//!                   [--input FILE] [--detail SLICE] [--mask-apu] [--audio] [--sgb]
//!                   [--save-at SLICE] --out FILE
//!
//! `--save-at SLICE` (this board only: the save-state round trip of rung 6): before that slice
//! the console saves its state, a new console is built from the same ROM, machine and boot ROM,
//! the state is loaded into it (APU included) and the run goes on there; the trace must equal the reference's uninterrupted one.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use gameboy_rust::boot::{find_boot_rom, read_image};
use gameboy_rust::joypad::{BUTTONS, DIRECTIONS};
use gameboy_rust::sgb::GREY_RAMP;
use gameboy_rust::{load_cartridge, machine, Clock, GameBoy};

const SLICE_T: u64 = 70224;

fn read_inputs(path: Option<&Path>) -> HashMap<u64, Vec<String>> {
    let mut out = HashMap::new();
    let Some(path) = path else {
        return out;
    };
    let text = std::fs::read_to_string(path).expect("input file");
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("");
        let words: Vec<&str> = line.split_whitespace().collect();
        if let Some(first) = words.first() {
            let names = words[1..]
                .iter()
                .filter(|w| **w != "-")
                .map(|w| w.to_string())
                .collect();
            out.insert(first.parse().expect("slice"), names);
        }
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut rom: Option<PathBuf> = None;
    let mut slices = 0u64;
    let mut machine_key = "dmg".to_string();
    let mut boot = "skip".to_string();
    let mut input: Option<PathBuf> = None;
    let mut detail: i64 = -1;
    let mut apu_values = true;
    let mut out_path: Option<PathBuf> = None;
    let mut save_at: i64 = -1;
    let mut audio = false;
    let mut sgb = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].clone();
        let mut value = || {
            i += 1;
            args.get(i).cloned().expect("missing value")
        };
        match a.as_str() {
            "--slices" => slices = value().parse().expect("slices"),
            "--machine" => machine_key = value(),
            "--boot" => boot = value(),
            "--input" => input = Some(PathBuf::from(value())),
            "--detail" => detail = value().parse().expect("detail"),
            "--mask-apu" => apu_values = false,
            "--out" => out_path = Some(PathBuf::from(value())),
            "--audio" => audio = true,
            "--sgb" => sgb = true,
            "--save-at" => save_at = value().parse().expect("save-at"),
            _ => rom = Some(PathBuf::from(a)),
        }
        i += 1;
    }
    let m = machine(&machine_key).expect("unknown machine");
    let image = match boot.as_str() {
        "skip" => None,
        "real" => Some(find_boot_rom(m.boot_rom, None).0.expect("no boot ROM")),
        path => Some(read_image(Path::new(path)).expect("boot image")),
    };
    let data = read_image(&rom.expect("ROM")).expect("ROM");
    let cart = load_cartridge(&data, Clock::Board).expect("cartridge");
    let mut gb = GameBoy::new(cart, m, image.clone()).expect("board");
    gb.hw.bus_log = Some(Vec::new());
    gb.hw.apu.capture = audio;
    if sgb {
        gb.attach_sgb(GREY_RAMP).expect("Super Game Boy");
    }
    let inputs = read_inputs(input.as_deref());
    let names: Vec<&str> = BUTTONS.iter().chain(DIRECTIONS.iter()).copied().collect();
    let mut pressed: Vec<String> = Vec::new();
    let file = std::fs::File::create(out_path.expect("--out")).expect("out");
    let mut out = std::io::BufWriter::new(file);
    for index in 0..slices {
        if let Some(wanted) = inputs.get(&index) {
            for name in &names {
                let n = name.to_string();
                if pressed.contains(&n) && !wanted.contains(&n) {
                    if sgb {
                        gb.sgb_set_pressed(name, false, 0);
                    } else {
                        gb.set_pressed(name, false);
                    }
                }
            }
            for name in &names {
                let n = name.to_string();
                if wanted.contains(&n) && !pressed.contains(&n) {
                    if sgb {
                        gb.sgb_set_pressed(name, true, 0);
                    } else {
                        gb.set_pressed(name, true);
                    }
                }
            }
            pressed = wanted.clone();
        }
        if index as i64 == save_at {
            let state = gb.save_state();
            let cart = load_cartridge(&data, Clock::Board).expect("cartridge");
            let mut fresh = GameBoy::new(cart, m, image.clone()).expect("board");
            fresh.load_state(&state).expect("load state");
            fresh.hw.bus_log = gb.hw.bus_log.take();
            gb = fresh;
        }
        let end = (index + 1) * SLICE_T;
        while gb.hw.t < end {
            gb.step();
        }
        let mut log = std::mem::take(gb.hw.bus_log.as_mut().unwrap());
        if !apu_values {
            for record in log.iter_mut() {
                let kind = (*record >> 24) & 3;
                let address = (*record >> 8) & 0xFFFF;
                if kind < 2
                    && ((0xFF10..=0xFF3F).contains(&address)
                        || (0xFF76..=0xFF77).contains(&address))
                {
                    *record &= !0xFF;
                }
            }
        }
        let bus = if log.is_empty() {
            0
        } else {
            let mut h = crc32fast::Hasher::new();
            for r in &log {
                h.update(&r.to_le_bytes());
            }
            h.finalize()
        };
        let ppu = &gb.hw.ppu;
        let mut h = crc32fast::Hasher::new();
        if gb.hw.cgb {
            for v in &ppu.frame {
                h.update(&v.to_le_bytes());
            }
        } else {
            let bytes: Vec<u8> = ppu.frame.iter().map(|&v| v as u8).collect();
            h.update(&bytes);
        }
        let pic = h.finalize();
        let c = &gb.cpu;
        let t = &gb.hw.timer;
        write!(
            out,
            "{} t={} pc={:04x} sp={:04x} af={:02x}{:02x} bc={:02x}{:02x} de={:02x}{:02x} \
             hl={:02x}{:02x} ie={:02x} if={:02x} ime={} halt={} stop={} div={:04x} tima={:02x} \
             ly={} dot={} mode={}/{} lcdc={:02x} dma={} ser={} frames={} bus={:08x}/{} pic={:08x}",
            index,
            gb.hw.t,
            c.pc,
            c.sp,
            c.a,
            c.f,
            c.b,
            c.c,
            c.d,
            c.e,
            c.h,
            c.l,
            c.ie,
            c.if_,
            c.ime as u8,
            c.halted as u8,
            c.stopped as u8,
            t.counter,
            t.tima,
            ppu.ly,
            ppu.dot,
            ppu.mode,
            ppu.read_mode,
            ppu.lcdc,
            ppu.dma.active as u8,
            gb.hw.serial.output.len(),
            ppu.frames,
            bus,
            log.len(),
            pic
        )
        .unwrap();
        if audio {
            let samples = gb.hw.apu.take_samples();
            let mut h = crc32fast::Hasher::new();
            for v in &samples {
                h.update(&v.to_le_bytes());
            }
            write!(out, " snd={:08x}/{}", h.finalize(), samples.len()).unwrap();
        }
        if sgb {
            let unit = gb.hw.sgb.as_deref().expect("Super Game Boy");
            let mut h = crc32fast::Hasher::new();
            for v in unit.render() {
                h.update(&v.to_le_bytes());
            }
            let picture = h.finalize();
            let state = crc32fast::hash(&unit.save_state());
            write!(out, " sgb={picture:08x}/{state:08x}").unwrap();
        }
        writeln!(out).unwrap();
        if index as i64 == detail {
            for r in &log {
                let kind = ['r', 'w', 'i'][((r >> 24) & 3) as usize];
                writeln!(
                    out,
                    "  {} {} {:04x} {:02x}",
                    r >> 26,
                    kind,
                    (r >> 8) & 0xFFFF,
                    r & 0xFF
                )
                .unwrap();
            }
        }
        log.clear();
        gb.hw.bus_log = Some(log);
    }
}
