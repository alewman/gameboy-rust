//! Run a game on the Super Game Boy row with the HLE SGB (`src/sgb.rs`) and save the 256 x 224
//! SNES picture (border + colourised Game Boy picture) as PNG: gameboy-python's
//! `scripts/sgb_picture.py`.
//!
//!     gameboy-sgb GAME.zip --frames 900 --png out.png
//!     gameboy-sgb GAME.gb --boot sgb_boot.bin --frames 1500 --every 60 --out-dir pictures/
//!
//! `--boot` is `skip` (the default: the skip-boot state), `mame` (MAME's `sns_rom_sgb.zip` /
//! `sns_rom_sgb2.zip` in a directory of `$MAME_ROMPATH`, `;`-separated) or a raw or zipped boot
//! ROM (a zip's first member, as the reference reads it). Nothing is copied. Each picture is the
//! reference's `rgb_bytes(sgb.render())` in an 8-bit RGB PNG. A cartridge clock (MBC3) runs on
//! board time, as in the gate (the reference's script uses wall time).

use std::io::Write;
use std::path::{Path, PathBuf};

use gameboy_rust::boot::read_image;
use gameboy_rust::sgb::{rgb_bytes, GREY_RAMP, SNES_H, SNES_W};
use gameboy_rust::{load_cartridge, machine, Clock, GameBoy};

/// 8-bit RGB, 3 bytes per pixel, rows top to bottom, as a PNG (the reference's
/// `image.rgb24_png_bytes`).
fn rgb24_png_bytes(rgb: &[u8], width: usize, height: usize) -> Vec<u8> {
    let stride = width * 3;
    let mut rows = Vec::with_capacity((stride + 1) * height);
    for y in 0..height {
        rows.push(0);
        rows.extend_from_slice(&rgb[y * stride..(y + 1) * stride]);
    }
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
    encoder.write_all(&rows).expect("zlib");
    let idat = encoder.finish().expect("zlib");
    fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32fast::hash(&body).to_be_bytes());
    }
    let mut header = Vec::new();
    header.extend_from_slice(&(width as u32).to_be_bytes());
    header.extend_from_slice(&(height as u32).to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &header);
    chunk(&mut out, b"IDAT", &idat);
    chunk(&mut out, b"IEND", b"");
    out
}

fn find_boot(choice: &str, machine_key: &str) -> Option<Vec<u8>> {
    match choice {
        "skip" => None,
        "mame" => {
            let set = if machine_key == "sgb2" {
                "sns_rom_sgb2.zip"
            } else {
                "sns_rom_sgb.zip"
            };
            let rompath = std::env::var("MAME_ROMPATH").unwrap_or_default();
            for folder in rompath.split(';').filter(|f| !f.is_empty()) {
                let path = Path::new(folder).join(set);
                if path.exists() {
                    return Some(read_image(&path).expect("boot image"));
                }
            }
            eprintln!("no {set} in $MAME_ROMPATH");
            std::process::exit(2);
        }
        path => Some(read_image(Path::new(path)).expect("boot image")),
    }
}

fn save(gb: &GameBoy, path: &Path) {
    let sgb = gb.hw.sgb.as_deref().expect("Super Game Boy");
    let png = rgb24_png_bytes(&rgb_bytes(&sgb.render()), SNES_W, SNES_H);
    std::fs::write(path, png).expect("write PNG");
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut rom: Option<PathBuf> = None;
    let mut machine_key = "sgb".to_string();
    let mut boot = "skip".to_string();
    let mut frames = 900u64;
    let mut png: Option<PathBuf> = None;
    let mut every: Option<u64> = None;
    let mut out_dir = PathBuf::from(".");
    let mut i = 0;
    while i < args.len() {
        let a = args[i].clone();
        let mut value = || {
            i += 1;
            args.get(i).cloned().expect("missing value")
        };
        match a.as_str() {
            "--machine" => machine_key = value(),
            "--boot" => boot = value(),
            "--frames" => frames = value().parse().expect("frames"),
            "--png" => png = Some(PathBuf::from(value())),
            "--every" => every = Some(value().parse().expect("every")),
            "--out-dir" => out_dir = PathBuf::from(value()),
            _ => rom = Some(PathBuf::from(a)),
        }
        i += 1;
    }
    let m = machine(&machine_key).expect("unknown machine");
    let data = read_image(&rom.expect("ROM")).expect("ROM");
    let cart = load_cartridge(&data, Clock::Board).expect("cartridge");
    let mut gb = GameBoy::new(cart, m, find_boot(&boot, &machine_key)).expect("board");
    gb.attach_sgb(GREY_RAMP).expect("Super Game Boy");
    for frame in 1..=frames {
        gb.run_frames(1, 50_000_000);
        if let Some(k) = every {
            if frame % k == 0 {
                std::fs::create_dir_all(&out_dir).expect("out dir");
                save(&gb, &out_dir.join(format!("sgb_{frame:05}.png")));
            }
        }
    }
    if let Some(path) = png {
        save(&gb, &path);
    }
    let sgb = gb.hw.sgb.as_deref().expect("Super Game Boy");
    eprintln!(
        "{} frames; SGB commands {}; border {}",
        frames,
        if sgb.enabled { "on" } else { "off" },
        if sgb.border_loaded { "loaded" } else { "none" }
    );
}
