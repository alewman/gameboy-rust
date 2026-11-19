//! Serial port with nothing attached (gameboy-python `serial.py`).
//!
//! SB shifts out MSB first with the internal clock at 8192 Hz. The clock is a toggle flip-flop
//! that each falling edge of system-counter bit 7 flips; a shift happens when it falls (SameBoy
//! GB_serial_master_edge). Any SC write drops a high toggle, so a transfer's first shift comes
//! two bit-7 falls later, and a DIV write while bit 7 is high is a fall too. Both writes act
//! before a fall in their own M-cycle (FITTED: Gambatte serial/nopx*_start*,
//! *_late_div_write_*). The serial divider is shared with the main clock but has its own reset,
//! so its edges run a fixed number of T-cycles ahead of the counter DIV shows: `SERIAL_PHASE_T`.
//! That constant is **fitted**: with it (and the counter's 4 T power-on phase in board.rs)
//! Mooneye serial/boot_sclk_align (verified DMG ABC and MGB) passes together with boot_div, from
//! skip-boot and from dmg-sim's quickboot. It has not been read off the die (dmg-schematics):
//! UNVERIFIED there. With nothing connected every incoming bit is 1, so a transfer ends with
//! SB = $FF. The eighth shift clears SC bit 7 and raises IF bit 3.
//!
//! Where Rust forced a difference: the reference raises IF bit 3 through a callback; here
//! `irq` is set and the board takes it (`take_irq`) right after each call, which is where the
//! reference's callback ran (it was always the last thing the shift did).

use serde::{Deserialize, Serialize};

/// its falls clock a toggle: 4194304 / 512 = 8192 Hz
pub const SERIAL_CLOCK_BIT: u32 = 7;
/// CGB mode, SC bit 1: 262144 Hz (Pan Docs Serial_Data_Transfer_(Link_Cable).md)
pub const FAST_CLOCK_BIT: u32 = 2;
/// the divider's edges come this many T-cycles before the counter's
pub const SERIAL_PHASE_T: i64 = 4;
/// an SC write comes before its M-cycle's fall
pub const SC_BEFORE_EDGE: bool = true;
/// ... and a DIV write sees the clock bit as it was before the M-cycle
pub const SERIAL_DIV_PHASE: i64 = -4;
/// M-cycles from the eighth edge to the last shift showing
pub const SERIAL_FINISH_DELAY: u32 = 1;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Serial {
    pub sb: u8,
    /// bit 7 transfer in progress, bit 0 internal clock
    pub sc: u8,
    pub shifted: u32,
    /// M-cycles until the eighth shift and SC's clearing show
    pub finish_in: u32,
    /// the toggle flip-flop the clock bit drives
    pub master: bool,
    /// the clock bit fell in this M-cycle
    pub fell: bool,
    /// SC bit 1 (the fast clock) exists only in CGB mode
    pub cgb_mode: bool,
    /// every byte that finished shifting out, for test harnesses
    pub output: Vec<u8>,
    /// IF bit 3 was raised by the last call
    pub irq: bool,
}

impl Serial {
    pub fn new() -> Serial {
        Serial::default()
    }

    /// The interrupt raised by the last call, if any (cleared).
    pub fn take_irq(&mut self) -> bool {
        std::mem::take(&mut self.irq)
    }

    /// Called every M-cycle with the system counter before and after.
    pub fn tick(&mut self, previous: u32, counter: u32) {
        if self.finish_in != 0 {
            self.finish_in -= 1;
            if self.finish_in == 0 {
                self.sb = (self.sb << 1) | 1;
                self.sc &= 0x7F;
            }
        }
        let bit = if self.sc & 2 != 0 {
            FAST_CLOCK_BIT
        } else {
            SERIAL_CLOCK_BIT
        };
        let phase = SERIAL_PHASE_T;
        let p = previous as i64 + phase;
        let c = counter as i64 + phase;
        self.fell = (p >> bit) & 1 != 0 && (c >> bit) & 1 == 0;
        if self.fell {
            self.master_edge();
        }
    }

    /// A DIV write clears the counter: a high clock bit falls, which clocks the toggle.
    pub fn div_reset(&mut self, counter: u32) {
        let bit = if self.sc & 2 != 0 {
            FAST_CLOCK_BIT
        } else {
            SERIAL_CLOCK_BIT
        };
        if ((counter as i64 + SERIAL_PHASE_T + SERIAL_DIV_PHASE) >> bit) & 1 != 0 {
            self.master_edge();
        }
    }

    /// A falling edge of the clock bit flips the toggle; the toggle's fall shifts.
    fn master_edge(&mut self) {
        self.master = !self.master;
        if self.master || self.sc & 0x81 != 0x81 || self.finish_in != 0 {
            return;
        }
        self.shifted += 1;
        if self.shifted == 8 {
            // the interrupt goes out now; the last shift and the clearing of SC bit 7 show
            // one M-cycle later (FITTED: Gambatte serial/start_wait_read_s[bc]_*)
            self.finish_in = SERIAL_FINISH_DELAY;
            self.irq = true;
        } else {
            self.sb = (self.sb << 1) | 1;
        }
    }

    /// The eighth shift has not shown yet and now never will.
    pub fn cancel_finish(&mut self) {
        self.finish_in = 0;
    }

    pub fn write_sb(&mut self, value: u8) {
        self.sb = value;
    }

    pub fn write_sc(&mut self, value: u8) {
        // any write restarts the count and drops a high toggle, which is a fall (SameBoy)
        let fell = SC_BEFORE_EDGE && self.fell;
        if fell {
            self.master = !self.master; // the write comes before this M-cycle's fall
        }
        self.shifted = 0;
        if self.master {
            self.master_edge();
        }
        self.sc = value & if self.cgb_mode { 0x83 } else { 0x81 };
        if fell {
            self.master_edge();
        }
        if value & 0x80 != 0 {
            self.output.push(self.sb); // the byte as it leaves (MSB first on the wire)
        }
    }

    pub fn read_sc(&self) -> u8 {
        self.sc | if self.cgb_mode { 0x7C } else { 0x7E }
    }
}
