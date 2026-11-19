//! Run the rung-4 suite jobs (`scripts/suite_jobs.py`) on this board, one JSON line per job,
//! as `scripts/suite_ref.py` does on the reference (see it for the fields):
//!
//!     gameboy-jobs JOBS --out FILE [--jobs N] [--only TEXT]
//!
//! The shootout jobs follow the reference's `scripts/shootout.py` (`run`, `startup_time`,
//! `_screen`, `_matches`).

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use serde_json::{json, Value};

use gameboy_rust::boot::{find_boot_rom, quickboot};
use gameboy_rust::pngread::read_rgb888;
use gameboy_rust::testrom::{run_blargg_ram, run_blargg_serial, run_microtest, run_mooneye};
use gameboy_rust::{load_cartridge, machine, Clock, GameBoy, Machine};

const FRAME_T: u64 = 70224;
const SECOND_T: u64 = 4194304;
const POLL_T: u64 = SECOND_T / 10;
const EXTRA_S: f64 = 5.0;
const TOLERANCE: i32 = 50;
const GREYS: [u32; 4] = [0xFF, 0xAA, 0x55, 0x00];

fn boot_image(m: &Machine, boot: &str) -> Result<Option<Vec<u8>>, String> {
    match boot {
        "skip" => Ok(None),
        "quick" => std::fs::read(quickboot())
            .map(Some)
            .map_err(|e| e.to_string()),
        _ => {
            let (image, label) = find_boot_rom(m.boot_rom, None);
            if label != "real boot ROM" {
                return Err(format!("no real boot ROM for {}", m.boot_rom));
            }
            Ok(image)
        }
    }
}

fn grey(r: u32, g: u32, b: u32) -> u8 {
    ((r * 19595 + g * 38470 + b * 7471 + 0x8000) >> 16) as u8
}

fn scale(v: u16) -> u32 {
    // round(v * 255 / 31), Python's round (half to even; no halves occur for 0..31)
    (v as f64 * 255.0 / 31.0).round() as u32
}

fn expected(path: &str) -> Option<Vec<u8>> {
    let p = Path::new(path);
    if !p.exists() {
        return None;
    }
    let (pixels, w, h) = read_rgb888(p).ok()?;
    if (w, h) != (160, 144) {
        return None;
    }
    Some(
        pixels
            .iter()
            .map(|&p| grey(p >> 16, (p >> 8) & 255, p & 255))
            .collect(),
    )
}

fn screen(gb: &GameBoy) -> Vec<u8> {
    let frame = &gb.hw.ppu.frame;
    if !gb.hw.cgb {
        return frame
            .iter()
            .map(|&s| {
                let v = GREYS[s as usize];
                grey(v, v, v)
            })
            .collect();
    }
    frame
        .iter()
        .map(|&w| grey(scale(w & 31), scale((w >> 5) & 31), scale((w >> 10) & 31)))
        .collect()
}

fn matches(screen: &[u8], want: &[u8]) -> bool {
    screen
        .iter()
        .zip(want.iter())
        .all(|(&a, &b)| (a as i32 - b as i32).abs() <= TOLERANCE)
}

fn startup_time(model: &str, rom: &str, png: &str) -> Result<f64, String> {
    let cgb = model == "CGB";
    let m = machine(if cgb { "cgb" } else { "dmg-b" }).unwrap();
    let boot = boot_image(&m, "real")?;
    let data = std::fs::read(rom).map_err(|e| e.to_string())?;
    let mut gb = GameBoy::new(load_cartridge(&data, Clock::Wall)?, m, boot)?;
    let want = expected(png).ok_or("no startup picture")?;
    while gb.hw.t < 60 * SECOND_T {
        let target = gb.hw.t + FRAME_T;
        while gb.hw.t < target {
            gb.step();
        }
        if matches(&screen(&gb), &want) {
            return Ok(gb.hw.t as f64 / SECOND_T as f64);
        }
    }
    Err("startup_time_test.gb never showed its picture".into())
}

fn shootout(job: &Value, startup: f64) -> String {
    let strs = |key: &str| -> Vec<String> {
        job[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect()
    };
    let passes: Vec<Vec<u8>> = strs("passes")
        .iter()
        .filter_map(|r| expected(r))
        .filter(|p| !p.is_empty())
        .collect();
    let fails: Vec<Vec<u8>> = strs("fails")
        .iter()
        .filter_map(|r| expected(r))
        .filter(|p| !p.is_empty())
        .collect();
    let m = machine(job["machine"].as_str().unwrap()).unwrap();
    let boot = match boot_image(&m, "real") {
        Ok(b) => b,
        Err(e) => return format!("error: {e}"),
    };
    let data = std::fs::read(job["rom"].as_str().unwrap()).unwrap();
    let cart = match load_cartridge(&data, Clock::Board) {
        Ok(c) => c,
        Err(e) => return format!("FAIL (load: {e})"),
    };
    let mut gb = GameBoy::new(cart, m, boot).unwrap();
    let runtime = job["runtime"].as_f64().unwrap();
    let end = ((startup + runtime + EXTRA_S) * SECOND_T as f64) as u64;
    let mut next_poll = POLL_T;
    while gb.hw.t < end {
        while gb.hw.t < next_poll {
            gb.step();
        }
        next_poll += POLL_T;
        let s = screen(&gb);
        if passes.iter().any(|p| matches(&s, p)) {
            return "PASS".into();
        }
        if fails.iter().any(|f| matches(&s, f)) {
            return "FAIL".into();
        }
    }
    if passes.is_empty() {
        "no result".into()
    } else {
        "FAIL".into()
    }
}

fn run(job: &Value, startup: &Mutex<HashMap<String, f64>>) -> Value {
    let id = job["id"].as_str().unwrap();
    let protocol = job["protocol"].as_str().unwrap();
    let mut out = json!({"id": id, "protocol": protocol});
    if protocol == "shootout" {
        let model = job["model"].as_str().unwrap().to_string();
        let known = startup.lock().unwrap().get(&model).copied();
        let s = match known {
            Some(s) => s,
            None => {
                let s = startup_time(
                    &model,
                    job["startup_rom"].as_str().unwrap(),
                    job["startup_png"].as_str().unwrap(),
                )
                .unwrap();
                startup.lock().unwrap().insert(model, s);
                s
            }
        };
        out["verdict"] = json!(shootout(job, s));
        out["startup"] = json!(s);
        return out;
    }
    let m = machine(job["machine"].as_str().unwrap()).unwrap();
    let result = (|| -> Result<Value, String> {
        let boot = boot_image(&m, job["boot"].as_str().unwrap())?;
        let data = std::fs::read(job["rom"].as_str().unwrap()).map_err(|e| e.to_string())?;
        let cart = load_cartridge(&data, Clock::Board)?;
        let mut gb = GameBoy::new(cart, m, boot)?;
        let frames = job["frames"].as_u64().unwrap();
        let r = match protocol {
            "mooneye" => run_mooneye(&mut gb, frames),
            "blargg-serial" => run_blargg_serial(&mut gb, frames),
            "blargg-ram" => run_blargg_ram(&mut gb, frames),
            _ => run_microtest(
                &mut gb,
                frames,
                job.get("vram_flag")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            ),
        };
        Ok(
            json!({"kind": r.protocol, "passed": r.passed, "text": r.text,
                   "registers": r.registers.to_vec(), "t": r.t_cycles, "frames": r.frames}),
        )
    })();
    match result {
        Ok(v) => {
            for (k, val) in v.as_object().unwrap() {
                out[k] = val.clone();
            }
        }
        Err(e) => out["error"] = json!(e),
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opt = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1).cloned())
    };
    let text = std::fs::read_to_string(&args[0]).expect("jobs file");
    let mut jobs: Vec<Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    if let Some(only) = opt("--only") {
        jobs.retain(|j| j["id"].as_str().unwrap().contains(&only));
    }
    let workers: usize = opt("--jobs").map(|v| v.parse().unwrap()).unwrap_or(10);
    let startup = Mutex::new(HashMap::new());
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<Value>>> = Mutex::new(vec![None; jobs.len()]);
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= jobs.len() {
                    break;
                }
                let r = run(&jobs[i], &startup);
                results.lock().unwrap()[i] = Some(r);
            });
        }
    });
    let mut out = String::new();
    for r in results.into_inner().unwrap().into_iter().flatten() {
        out.push_str(&serde_json::to_string(&r).unwrap());
        out.push('\n');
    }
    std::fs::write(opt("--out").expect("--out"), out).expect("write");
}
