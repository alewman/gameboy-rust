//! The sound unit: gb-apu-rust (crate `gb_apu`, transcribed from gb-apu-python `b8cab61`, which
//! equals the reference's `f0a7f48` on the DMG and CGB rows for a board ticking even T: its
//! docs/status.md). board.py drives gb-apu-python's `APU` through `read`, `write`,
//! `frame_tick`, `secondary_tick`, `tick`, `pcm`, `skip_boot` and `frame_clock_high`; this board
//! makes the same calls on the same object.

pub use gb_apu::{APU, CGB, DMG};

/// The APU for a machine (`APU(model=CGB if cgb else DMG)`, 44,100 Hz).
pub fn new(cgb: bool) -> APU {
    APU::new(44100, if cgb { CGB } else { DMG })
}

/// A default APU for a state restored without one (serde).
pub fn placeholder() -> APU {
    APU::default()
}
