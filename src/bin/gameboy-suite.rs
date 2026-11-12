//! The picture- and screen-judged suites, with the reference's rules (gameboy-python
//! `scripts/gambatte_dmg08.py`, `scripts/mealybug_compare.py`, `scripts/cgb_pictures.py`,
//! `scripts/cgb_picture.py`, `tests/test_pictures.py`):
//!
//!     gameboy-suite gambatte [--cgb] [--skip-boot] [--jobs N] [--write FILE]
//!     gameboy-suite mealybug-dmg [--jobs N]        # real DMG boot ROM, run_mooneye 420 frames
//!     gameboy-suite cgb-pictures [--jobs N] [--revision cgb_d]   # real CGB boot ROM, 260 frames
//!     gameboy-suite pictures                       # dmg-acid2, cgb-acid2, halt_bug, strikethrough
//!
//! `$GBTR` moves the test-ROM directory (default gameboy-python's tests/test_roms/gbtr).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use gameboy_rust::boot::find_boot_rom;
use gameboy_rust::pngread::{read_rgb555, read_rgb888, read_shades};
use gameboy_rust::testrom::run_mooneye;
use gameboy_rust::{load_cartridge, Clock, GameBoy, Machine, CGB, DMG_B};

const FRAME_T: u64 = 70224;
const FRAMES: u64 = 334 + 15;
const CGB_FRAMES: u64 = 186 + 15; // testrunner.cpp: the CGB boot ROM's frames and 15 more

fn gbtr() -> PathBuf {
    PathBuf::from(
        std::env::var("GBTR")
            .unwrap_or_else(|_| "/data/emu/gameboy-python/tests/test_roms/gbtr".into()),
    )
}

fn board(rom: &Path, machine: Machine, boot: Option<Vec<u8>>) -> GameBoy {
    let data = std::fs::read(rom).expect("ROM");
    GameBoy::new(
        load_cartridge(&data, Clock::Wall).expect("cartridge"),
        machine,
        boot,
    )
    .expect("board")
}

fn run_to(gb: &mut GameBoy, end: u64) {
    while gb.hw.t < end {
        gb.step();
    }
}

fn real_boot(name: &str) -> Vec<u8> {
    let (image, label) = find_boot_rom(name, None);
    if label != "real boot ROM" {
        eprintln!("no real {name} boot ROM ({label})");
        std::process::exit(2);
    }
    image.unwrap()
}

/// Run `work` items on `jobs` threads; results in input order.
fn parallel<T: Sync, R: Send>(items: &[T], jobs: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<Option<R>>> = Mutex::new((0..items.len()).map(|_| None).collect());
    std::thread::scope(|s| {
        for _ in 0..jobs.max(1) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i >= items.len() {
                    break;
                }
                let r = f(&items[i]);
                out.lock().unwrap()[i] = Some(r);
            });
        }
    });
    out.into_inner()
        .unwrap()
        .into_iter()
        .map(|r| r.unwrap())
        .collect()
}

// ----- Gambatte ---------------------------------------------------------------------------

fn glyph(ch: char) -> [u8; 64] {
    let rows = match ch {
        '0' => "........|.#######|.#.....#|.#.....#|.#.....#|.#.....#|.#.....#|.#######",
        '1' => "........|....#...|....#...|....#...|....#...|....#...|....#...|....#...",
        '2' => "........|.#######|.......#|.......#|.#######|.#......|.#......|.#######",
        '3' => "........|.#######|.......#|.......#|..######|.......#|.......#|.#######",
        '4' => "........|.#.....#|.#.....#|.#.....#|.#######|.......#|.......#|.......#",
        '5' => "........|.#######|.#......|.#......|.######.|.......#|.......#|.######.",
        '6' => "........|.#######|.#......|.#......|.#######|.#.....#|.#.....#|.#######",
        '7' => "........|.#######|.......#|......#.|.....#..|....#...|...#....|...#....",
        '8' => "........|..#####.|.#.....#|.#.....#|..#####.|.#.....#|.#.....#|..#####.",
        '9' => "........|.#######|.#.....#|.#.....#|.#######|.......#|.......#|.#######",
        'a' => "........|....#...|..#...#.|.#.....#|.#######|.#.....#|.#.....#|.#.....#",
        'b' => "........|.######.|.#.....#|.#.....#|.######.|.#.....#|.#.....#|.######.",
        'c' => "........|..#####.|.#.....#|.#......|.#......|.#......|.#.....#|..#####.",
        'd' => "........|.######.|.#.....#|.#.....#|.#.....#|.#.....#|.#.....#|.######.",
        'e' => "........|.#######|.#......|.#......|.#######|.#......|.#......|.#######",
        _ => "........|.#######|.#......|.#......|.#######|.#......|.#......|.#......",
    };
    let mut out = [0u8; 64];
    for (i, c) in rows.chars().filter(|&c| c != '|').enumerate() {
        out[i] = if c == '#' { 3 } else { 0 };
    }
    out
}

#[derive(Clone, Debug)]
enum Test {
    Out(String),
    Png(PathBuf),
}

impl Test {
    fn kind(&self) -> &'static str {
        match self {
            Test::Out(_) => "out",
            Test::Png(_) => "png",
        }
    }
}

/// The tests this ROM carries for the DMG-08 (or, with `cgb`, the CGB-04c).
fn plan(rom: &Path, cgb: bool) -> Vec<Test> {
    let stem = rom.file_stem().unwrap().to_string_lossy().to_string();
    let mut tests = Vec::new();
    let markers: &[&str] = if cgb {
        &["dmg08_cgb04c_out", "cgb04c_out", "_out"]
    } else {
        &["dmg08_cgb04c_out", "dmg08_out"]
    };
    for marker in markers {
        if let Some(at) = stem.find(marker) {
            if !cgb || *marker != "_out" || !stem.contains("dmg08_out") {
                let tail = &stem[at + marker.len()..];
                if !tail.starts_with("audio") {
                    let digits: String = tail
                        .chars()
                        .take_while(|c| c.is_ascii_hexdigit())
                        .collect::<String>()
                        .to_lowercase();
                    tests.push(Test::Out(digits));
                }
                break;
            }
        }
    }
    let suffixes: &[&str] = if cgb {
        &["_dmg08_cgb04c.png", "_cgb04c.png"]
    } else {
        &["_dmg08_cgb04c.png", "_dmg08.png"]
    };
    for suffix in suffixes {
        let png = rom.with_file_name(format!("{stem}{suffix}"));
        if png.exists() {
            tests.push(Test::Png(png));
            break;
        }
    }
    tests
}

fn matches_out(frame: &[u8], digits: &str) -> bool {
    for (i, ch) in digits.chars().enumerate() {
        let tile = glyph(ch);
        for y in 0..8 {
            let start = y * 160 + i * 8;
            if frame.get(start..start + 8) != Some(&tile[y * 8..y * 8 + 8]) {
                return false;
            }
        }
    }
    true
}

/// Gambatte's testrunner colour for an RGB555 word, as 0xRRGGBB (its `lut` in runTestRom).
fn lut(word: u16) -> u32 {
    let (r, g, b) = (
        (word & 31) as u32,
        ((word >> 5) & 31) as u32,
        ((word >> 10) & 31) as u32,
    );
    (((13 * r + 2 * g + b) >> 1) << 16)
        | (((3 * g + b) << 1) << 8)
        | ((3 * r + 2 * g + 11 * b) >> 1)
}

fn judge(frame: &[u16], tests: &[Test], cgb: bool) -> Vec<bool> {
    let shades: Vec<u8> = if cgb {
        frame
            .iter()
            .map(|&w| match w & 0x7FFF {
                0x7FFF => 0,
                0 => 3,
                _ => 9,
            })
            .collect()
    } else {
        frame.iter().map(|&v| v as u8).collect()
    };
    tests
        .iter()
        .map(|test| match test {
            Test::Out(digits) => matches_out(&shades, digits),
            Test::Png(png) if cgb => match read_rgb888(png) {
                Ok((pixels, w, h)) => {
                    (w, h) == (160, 144)
                        && frame
                            .iter()
                            .zip(pixels.iter())
                            .all(|(&f, &p)| lut(f) & 0xF8F8F8 == p & 0xF8F8F8)
                }
                Err(_) => false,
            },
            Test::Png(png) => match read_shades(png) {
                Ok((want, w, h)) => (w, h) == (160, 144) && want == shades,
                Err(_) => false,
            },
        })
        .collect()
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn gambatte(args: &[String]) {
    let cgb = args.iter().any(|a| a == "--cgb");
    let skip = args.iter().any(|a| a == "--skip-boot");
    let jobs = opt(args, "--jobs")
        .map(|v| v.parse().unwrap())
        .unwrap_or(10);
    let suite = gbtr().join("gambatte");
    let mut roms = Vec::new();
    walk(&suite, &mut roms);
    roms.retain(|p| {
        matches!(
            p.extension().and_then(|e| e.to_str()),
            Some("gb") | Some("gbc")
        ) && !plan(p, cgb).is_empty()
    });
    // Python sorts the paths component by component
    roms.sort_by(|a, b| a.components().cmp(b.components()));
    if let Some(m) = opt(args, "--match") {
        roms.retain(|p| p.to_string_lossy().contains(&m));
    }
    let boot = if skip {
        None
    } else {
        Some(real_boot(if cgb { "CGB" } else { "DMG" }))
    };
    let results = parallel(&roms, jobs, |rom| {
        let tests = plan(rom, cgb);
        let mut gb = board(rom, if cgb { CGB } else { DMG_B }, boot.clone());
        let frames = if skip {
            15
        } else if cgb {
            CGB_FRAMES
        } else {
            FRAMES
        };
        run_to(&mut gb, frames * FRAME_T);
        let ok = judge(&gb.hw.ppu.frame, &tests, cgb);
        let name = rom
            .strip_prefix(&suite)
            .unwrap()
            .to_string_lossy()
            .to_string();
        tests
            .iter()
            .zip(ok)
            .map(|(t, ok)| (name.clone(), t.kind(), ok))
            .collect::<Vec<_>>()
    });
    let flat: Vec<_> = results.into_iter().flatten().collect();
    let passed: Vec<_> = flat.iter().filter(|r| r.2).collect();
    println!(
        "{} of {} agree with the {} ({} ROMs)",
        passed.len(),
        flat.len(),
        if cgb { "CGB-04c" } else { "DMG-08" },
        roms.len()
    );
    if let Some(path) = opt(args, "--write") {
        let text: String = passed
            .iter()
            .map(|r| format!("{} {}\n", r.0, r.1))
            .collect();
        std::fs::write(path, text).expect("write");
    }
}

// ----- pictures ---------------------------------------------------------------------------

fn diff_count<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    a.iter().zip(b.iter()).filter(|(x, y)| x != y).count()
}

fn mealybug_dmg(args: &[String]) {
    let jobs = opt(args, "--jobs")
        .map(|v| v.parse().unwrap())
        .unwrap_or(10);
    let dir = gbtr().join("mealybug-tearoom-tests/ppu");
    let mut roms = Vec::new();
    walk(&dir, &mut roms);
    roms.retain(|p| {
        p.extension().and_then(|e| e.to_str()) == Some("gb")
            && dir
                .join(format!(
                    "{}_dmg_blob.png",
                    p.file_stem().unwrap().to_string_lossy()
                ))
                .exists()
    });
    roms.sort();
    let boot = real_boot("DMG");
    let results = parallel(&roms, jobs, |rom| {
        let mut gb = board(rom, DMG_B, Some(boot.clone()));
        run_mooneye(&mut gb, 420);
        let stem = rom.file_stem().unwrap().to_string_lossy().to_string();
        let (want, _, _) = read_shades(&dir.join(format!("{stem}_dmg_blob.png"))).unwrap();
        let got: Vec<u8> = gb.hw.ppu.frame.iter().map(|&v| v as u8).collect();
        (stem, diff_count(&got, &want))
    });
    let mut exact = 0;
    let mut total = 0;
    for (name, diff) in &results {
        println!("{name:40} {diff:6}");
        exact += (*diff == 0) as usize;
        total += diff;
    }
    println!(
        "{exact} of {} pixel-exact, {total} pixels differ",
        results.len()
    );
}

fn cgb_pictures(args: &[String]) {
    let jobs = opt(args, "--jobs")
        .map(|v| v.parse().unwrap())
        .unwrap_or(10);
    let revision = opt(args, "--revision").unwrap_or_else(|| "cgb_d".into());
    let frames = opt(args, "--frames")
        .map(|v| v.parse().unwrap())
        .unwrap_or(260u64);
    let base = gbtr();
    let mut cases = vec![(
        base.join("cgb-acid-hell/cgb-acid-hell.gbc"),
        base.join("cgb-acid-hell/cgb-acid-hell.png"),
    )];
    let dir = base.join("mealybug-tearoom-tests/ppu");
    let mut pngs = Vec::new();
    walk(&dir, &mut pngs);
    let suffix = format!("_{revision}.png");
    pngs.retain(|p| p.to_string_lossy().ends_with(&suffix));
    pngs.sort();
    for png in pngs {
        let name = png.file_name().unwrap().to_string_lossy().to_string();
        let rom = png.with_file_name(format!("{}.gb", &name[..name.len() - suffix.len()]));
        if rom.exists() {
            cases.push((rom, png));
        }
    }
    let boot = real_boot("CGB");
    let results = parallel(&cases, jobs, |(rom, png)| {
        let mut gb = board(rom, CGB, Some(boot.clone()));
        run_to(&mut gb, frames * FRAME_T);
        let (want, _, _) = read_rgb555(png).unwrap();
        let name = rom.file_name().unwrap().to_string_lossy().to_string();
        (name, diff_count(&gb.hw.ppu.frame, &want))
    });
    let mut exact = 0;
    for (name, bad) in &results {
        println!(
            "{} {name}: {bad} pixels differ",
            if *bad == 0 { "ok  " } else { "DIFF" }
        );
        exact += (*bad == 0) as usize;
    }
    println!("{exact} of {} pixel-exact", results.len());
}

fn pictures() {
    let g = gbtr();
    let shades = |gb: &GameBoy| -> Vec<u8> { gb.hw.ppu.frame.iter().map(|&v| v as u8).collect() };
    let report = |name: &str, bad: usize| println!("{name}: {bad} pixels differ");
    // dmg-acid2 from skip-boot, 30 frames
    let mut gb = board(&g.join("dmg-acid2/dmg-acid2.gb"), DMG_B, None);
    gb.run_frames(30, 50_000_000);
    let (want, _, _) = read_shades(&g.join("dmg-acid2/dmg-acid2-dmg.png")).unwrap();
    report("dmg-acid2", diff_count(&shades(&gb), &want));
    // halt_bug, 600 frames
    let mut gb = board(&g.join("blargg/halt_bug.gb"), DMG_B, None);
    gb.run_frames(600, 50_000_000);
    let (want, _, _) = read_shades(&g.join("blargg/halt_bug-dmg-cgb.png")).unwrap();
    report("halt_bug", diff_count(&shades(&gb), &want));
    // strikethrough, 60 frames
    let path = g.join("strikethrough/strikethrough.gb");
    if path.exists() {
        let mut gb = board(&path, DMG_B, None);
        gb.run_frames(60, 50_000_000);
        let (want, _, _) = read_shades(&g.join("strikethrough/strikethrough-dmg.png")).unwrap();
        report("strikethrough", diff_count(&shades(&gb), &want));
    }
    // cgb-acid2: real CGB boot ROM, 300 frames (validation.md)
    let boot = real_boot("CGB");
    let mut gb = board(&g.join("cgb-acid2/cgb-acid2.gbc"), CGB, Some(boot));
    run_to(&mut gb, 300 * FRAME_T);
    let (want, _, _) = read_rgb555(&g.join("cgb-acid2/cgb-acid2.png")).unwrap();
    report("cgb-acid2", diff_count(&gb.hw.ppu.frame, &want));
}

fn opt(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("gambatte") => gambatte(&args[1..]),
        Some("mealybug-dmg") => mealybug_dmg(&args[1..]),
        Some("cgb-pictures") => cgb_pictures(&args[1..]),
        Some("pictures") => pictures(),
        _ => {
            eprintln!("usage: gameboy-suite gambatte|mealybug-dmg|cgb-pictures|pictures [options]");
            std::process::exit(2);
        }
    }
}
