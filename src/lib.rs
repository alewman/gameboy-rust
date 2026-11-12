//! A Rust Game Boy (DMG/CGB) board transcribed from gameboy-python at `gameboy-rust-pin-1`
//! (`efdde44`) and proven equivalent to it in lockstep: every M-cycle's bus access (address,
//! value, direction, board time) and every finished picture.
//!
//! Each module is the transcription of one `gameboy_python` module, with the same names, the
//! same structure and the hardware comments carried over. Where Rust forced a difference, the
//! module header says so. The CPU is the `sm83` crate (sm83-rust, transcribed from sm83-python
//! `95f2b8b`, the revision the reference pins).

pub mod apu;
pub mod board;
pub mod boot;
pub mod cartridge;
pub mod dma;
pub mod joypad;
pub mod machines;
pub mod pixels;
pub mod ppu;
pub mod serial;
pub mod testrom;
pub mod timer;

pub use board::GameBoy;
pub use cartridge::{load_cartridge, Cartridge, Clock};
pub use machines::{machine, Machine, CGB, DMG_B};
