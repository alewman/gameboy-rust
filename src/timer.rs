//! The system counter, DIV and the timer (gameboy-python `timer.py`; its docs/timing.md "The
//! system counter").
//!
//! One 16-bit counter counts every T-cycle. DIV ($FF04) is its top byte; any write to DIV clears
//! the whole counter. TIMA increments on the **falling edge** of `selected counter bit AND TAC
//! enable`, so writing DIV, or changing TAC, can produce an extra edge (Pan Docs
//! Timer_and_Divider_Registers.md and Timer_Obscure_Behaviour.md; Mooneye acceptance/timer/*,
//! hardware-verified). On overflow TIMA reads $00 for one M-cycle, then TMA is loaded and IF bit 2
//! is raised; a write to TIMA in the first of those cycles cancels the reload, a write in the
//! second is ignored, a write to TMA in the second is loaded too.
//!
//! Other consumers of the counter (the serial clock, the frame sequencer) read `previous` and
//! `counter` after `tick` and look for falling edges themselves.
//!
//! Where Rust forced a difference: the reference raises IF bit 2 through a callback at the end
//! of `finish`; here `finish` returns true and the board makes the same call.

use serde::{Deserialize, Serialize};

/// counter bit selected by TAC bits 1-0 (Pan Docs): 4096, 262144, 65536, 16384 Hz
pub const TIMER_BITS: [u32; 4] = [9, 3, 5, 7];

/// The counter, TIMA, TMA and TAC.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Timer {
    pub tac_disable_glitch: bool,
    pub counter: u32,
    /// the counter before the last tick
    pub previous: u32,
    pub tima: u32,
    pub tma: u8,
    pub tac: u8,
    /// TIMA overflowed in the previous M-cycle
    pub reload_pending: bool,
    /// this M-cycle is the one that loads TMA
    pub reloading: bool,
}

impl Timer {
    pub fn new(tac_disable_glitch: bool) -> Timer {
        Timer {
            tac_disable_glitch,
            counter: 0,
            previous: 0,
            tima: 0,
            tma: 0,
            tac: 0,
            reload_pending: false,
            reloading: false,
        }
    }

    /// Advance one M-cycle: `t_cycles` counts of the system counter.
    pub fn tick(&mut self, t_cycles: u32) {
        let previous = self.counter;
        self.previous = previous;
        let counter = (previous + t_cycles) & 0xFFFF;
        self.counter = counter;
        if self.tac & 4 != 0 {
            let bit = TIMER_BITS[(self.tac & 3) as usize];
            if (previous >> bit) & 1 != 0 && (counter >> bit) & 1 == 0 {
                self.increment();
            }
        }
    }

    /// The end of the M-cycle, after the CPU's access. The reload and the interrupt happen
    /// here, at the end of the overflow cycle, so that the core's next decision sees IF: the
    /// hardware services the timer interrupt before the instruction after the one whose cycle
    /// overflowed TIMA (Mooneye timer/rapid_toggle). TIMA still read $00 during the overflow
    /// cycle; the reload is visible from the next one. True: raise IF bit 2.
    #[must_use]
    pub fn finish(&mut self) -> bool {
        if self.reload_pending {
            self.reload_pending = false;
            self.reloading = true;
            self.tima = self.tma as u32;
            true
        } else {
            self.reloading = false;
            false
        }
    }

    fn increment(&mut self) {
        self.tima += 1;
        if self.tima > 0xFF {
            self.tima = 0;
            self.reload_pending = true;
        }
    }

    /// The level that clocks TIMA: the selected counter bit AND the enable.
    fn signal(counter: u32, tac: u8) -> u32 {
        if tac & 4 != 0 {
            (counter >> TIMER_BITS[(tac & 3) as usize]) & 1
        } else {
            0
        }
    }

    pub fn read_div(&self) -> u8 {
        (self.counter >> 8) as u8
    }

    /// Any write clears the counter; if the clocking level was high, that is a falling edge.
    pub fn write_div(&mut self) {
        if Self::signal(self.counter, self.tac) != 0 {
            self.increment();
        }
        self.counter = 0;
    }

    pub fn write_tima(&mut self, value: u8) {
        if self.reloading {
            return; // TMA wins during the reload cycle
        }
        self.reload_pending = false; // a write in the overflow cycle cancels the reload
        self.tima = value as u32;
    }

    pub fn write_tma(&mut self, value: u8) {
        self.tma = value;
        if self.reloading {
            self.tima = value as u32; // loaded too if written in the reload cycle
        }
    }

    pub fn write_tac(&mut self, value: u8) {
        let value = value & 7;
        let old = Self::signal(self.counter, self.tac);
        let new = Self::signal(self.counter, value);
        let enabling_edge = old != 0 && new == 0;
        if enabling_edge && (value & 4 != 0 || self.tac_disable_glitch) {
            // a falling edge: a different bit selected while enabled, or (DMG only) disabling
            self.increment();
        }
        self.tac = value;
    }
}
