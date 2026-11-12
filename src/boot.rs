//! Boot ROMs are bring-your-own, and the real one is the default (gameboy-python `boot.py`;
//! its docs/start-here.md "Boot ROMs").
//!
//! Order: a configured path (`override` or `GAMEBOY_BOOT_ROM`, a raw image or a No-Intro
//! `.zip`), then a local No-Intro set's `[BIOS]` zips read in place and checked by SHA-256
//! (`GAMEBOY_NOINTRO_DIR` moves the directory), then dmg-sim's `quickboot.bin` and SameBoy's
//! open ROM from the reference's `reference/` tree (`GAMEBOY_REFERENCE_ROOT`, default the
//! pinned checkout under `external/`). Nothing is ever copied into this repository.

use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// boot ROM name (machines::Machine::boot_rom) -> (zip under the No-Intro directory, SHA-256)
pub const LOCAL: [(&str, &str, &str); 4] = [
    (
        "DMG",
        "Nintendo - Game Boy/[BIOS] Nintendo Game Boy Boot ROM (World) (Rev 1).zip",
        "cf053eccb4ccafff9e67339d4e78e98dce7d1ed59be819d2a1ba2232c6fce1c7",
    ),
    (
        "DMG0",
        "Nintendo - Game Boy/[BIOS] Nintendo Game Boy Boot ROM (Japan) (En).zip",
        "26e71cf01e301e5dc40e987cd2ecbf6d0276245890ac829db2a25323da86818e",
    ),
    (
        "CGB",
        "Nintendo - Game Boy Color/[BIOS] Nintendo Game Boy Color Boot ROM (World) (Rev 1).zip",
        "b4f2e416a35eef52cba161b159c7c8523a92594facb924b3ede0d722867c50c7",
    ),
    (
        "MGB",
        "Nintendo - Game Boy/[BIOS] Nintendo Game Boy Pocket Boot ROM (World).zip",
        "a8cb5f4f1f16f2573ed2ecd8daedb9c5d1dd2c30a481f9b179b5d725d95eafe2",
    ),
];

pub fn nointro() -> PathBuf {
    PathBuf::from(
        std::env::var("GAMEBOY_NOINTRO_DIR")
            .unwrap_or_else(|_| "/data/emu/source/myrient.erista.me/files/No-Intro".into()),
    )
}

fn reference_root() -> PathBuf {
    std::env::var("GAMEBOY_REFERENCE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("external/gameboy-python"))
}

pub fn quickboot() -> PathBuf {
    reference_root().join("reference/src/dmg-sim/boot/quickboot.bin")
}

fn sameboy_open() -> PathBuf {
    reference_root().join("reference/src/SameBoy/build/bin/BootROMs/dmg_boot.bin")
}

/// The first member of a zip, or the file itself.
pub fn read_image(path: &Path) -> std::io::Result<Vec<u8>> {
    let is_zip = path
        .extension()
        .map(|e| e.eq_ignore_ascii_case("zip"))
        .unwrap_or(false);
    if is_zip {
        let file = std::fs::File::open(path)?;
        let mut archive = zip::ZipArchive::new(file).map_err(std::io::Error::other)?;
        let mut member = archive.by_index(0).map_err(std::io::Error::other)?;
        let mut out = Vec::new();
        member.read_to_end(&mut out)?;
        return Ok(out);
    }
    std::fs::read(path)
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The boot image for `name` and a label saying where it came from (None: skip boot).
pub fn find_boot_rom(name: &str, override_path: Option<&Path>) -> (Option<Vec<u8>>, String) {
    let env = std::env::var("GAMEBOY_BOOT_ROM").ok().map(PathBuf::from);
    let chosen = override_path.map(Path::to_path_buf).or(env);
    if let Some(chosen) = chosen {
        if let Ok(image) = read_image(&chosen) {
            let label = format!(
                "boot ROM {}",
                chosen.file_name().unwrap_or_default().to_string_lossy()
            );
            return (Some(image), label);
        }
    }
    if let Some((_, relative, digest)) = LOCAL.iter().find(|(n, _, _)| *n == name) {
        let path = nointro().join(relative);
        if path.exists() {
            if let Ok(image) = read_image(&path) {
                if sha256_hex(&image) == *digest {
                    return (Some(image), "real boot ROM".into());
                }
            }
        }
    }
    if name == "DMG" && quickboot().exists() {
        if let Ok(image) = std::fs::read(quickboot()) {
            return (Some(image), "quickboot (dmg-sim)".into());
        }
    }
    if name == "DMG" && sameboy_open().exists() {
        if let Ok(image) = std::fs::read(sameboy_open()) {
            return (
                Some(image),
                "SameBoy open boot ROM: boot-state tests unreliable".into(),
            );
        }
    }
    (None, "none found: skip-boot state".into())
}
