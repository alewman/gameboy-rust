//! OAM DMA (gameboy-python `dma.py`; its docs/timing.md "OAM DMA").
//!
//! Writing $FF46: one M-cycle of nothing, then at M+2 the transfer starts and OAM reads return
//! $FF; one byte per M-cycle for 160 M-cycles (Mooneye oam_dma_start header, all models; GBCTR
//! dma.typ). A write during a transfer restarts it, the old one running until the new one
//! starts. During the transfer the DMA owns the *source* bus: a CPU read on that bus gets the
//! DMA's byte (Pan Docs OAM_DMA_Transfer.md "OAM DMA bus conflicts"). Every source in
//! $A000-$FFFF is an external-bus read (GBCTR dma.typ "OAM DMA address decoding"; Mooneye
//! oam_dma/sources-GS).
//!
//! Where Rust forced a difference: the reference's `tick` reads the source and writes OAM
//! through two board callbacks; here it is split in two, `begin_tick` (returns the source
//! address to read, if a byte moves) and `end_tick` (the byte read), with the board's read in
//! between and the OAM write done by the board.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Bus {
    External,
    Video,
    Internal,
}

/// Which bus an address is on: cartridge and WRAM share the external bus.
pub fn bus_of(address: u16) -> Bus {
    if address < 0x8000 || (0xA000..0xFE00).contains(&address) {
        return Bus::External;
    }
    if address < 0xA000 {
        return Bus::Video;
    }
    Bus::Internal
}

/// The bus a DMA source page is read from ($E0 and above: the external bus, echoing WRAM).
pub fn source_bus(base: u8) -> Bus {
    bus_of(if base < 0xE0 {
        (base as u16) << 8
    } else {
        0xC000
    })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OamDma {
    /// the last value written (the register reads it back)
    pub register: u8,
    pub pending: Option<u8>,
    pub countdown: i32,
    pub active: bool,
    pub base: u8,
    pub index: usize,
    /// the byte on the source bus this M-cycle
    pub byte: u8,
    pub bus: Bus,
    /// the last byte moved this M-cycle: still locked until the next
    pub finishing: bool,
}

impl Default for OamDma {
    fn default() -> Self {
        OamDma {
            register: 0xFF,
            pending: None,
            countdown: 0,
            active: false,
            base: 0,
            index: 0,
            byte: 0xFF,
            bus: Bus::External,
            finishing: false,
        }
    }
}

impl OamDma {
    pub fn start(&mut self, value: u8) {
        self.register = value;
        self.pending = Some(value);
        self.countdown = 2; // the next M-cycle does nothing, the one after starts the transfer
    }

    /// The first half of one M-cycle: the source address to read, when a byte moves.
    pub fn begin_tick(&mut self) -> Option<u16> {
        if self.finishing {
            self.finishing = false;
            self.active = false;
        }
        if let Some(pending) = self.pending {
            self.countdown -= 1;
            if self.countdown == 0 {
                self.base = pending;
                self.pending = None;
                self.active = true;
                self.index = 0;
                self.bus = source_bus(self.base);
            }
        }
        if self.active {
            let mut source = ((self.base as u32) << 8) | self.index as u32;
            if self.base >= 0xE0 {
                source -= 0x2000; // $E000-$FFFF reads WRAM through the echo
            }
            return Some(source as u16);
        }
        None
    }

    /// The second half: `byte` was read from the source; returns the OAM index it goes to.
    pub fn end_tick(&mut self, byte: u8) -> usize {
        self.byte = byte;
        let index = self.index;
        self.index += 1;
        if self.index == 160 {
            self.finishing = true;
        }
        index
    }
}
