//! Save states: the whole board, sound unit included (gameboy-python's `state.py` offers only a
//! comparison snapshot, `capture_board_state`, and no restore; this is the board's own).
//!
//! A state is the CPU's `CpuState` (sm83's, saved between steps, where the core's in-step fields
//! are idle) and every board register, memory, unit and in-flight countdown (`Hw`'s fields,
//! serde), bincode-encoded behind a header. Left out, and kept from the console the state is
//! loaded into: the cartridge ROM and header, the boot ROM and the machine row. The APU is
//! gb-apu-rust's, serialised whole through its `serde` feature (since `GBRUST02`; `GBRUST01`
//! states had no APU and are refused). `capture_board_state` is here too.

use serde::{Deserialize, Serialize};
use sm83::CpuState;

use crate::board::{GameBoy, Hw};

const MAGIC: &[u8; 8] = b"GBRUST02";

#[derive(Serialize, Deserialize)]
struct Cpu {
    regs: [u8; 8],
    sp: u16,
    pc: u16,
    ie: u8,
    if_: u8,
    flags: [bool; 7],
    stop_if: u8,
}

impl Cpu {
    fn from(s: &CpuState) -> Cpu {
        Cpu {
            regs: [s.a, s.f, s.b, s.c, s.d, s.e, s.h, s.l],
            sp: s.sp,
            pc: s.pc,
            ie: s.ie,
            if_: s.if_,
            flags: [
                s.ime,
                s.ei_pending,
                s.halted,
                s.halt_bug,
                s.halt_dispatch,
                s.stopped,
                s.locked,
            ],
            stop_if: s.stop_if,
        }
    }

    fn to(&self) -> CpuState {
        let r = self.regs;
        let f = self.flags;
        CpuState {
            a: r[0],
            f: r[1],
            b: r[2],
            c: r[3],
            d: r[4],
            e: r[5],
            h: r[6],
            l: r[7],
            sp: self.sp,
            pc: self.pc,
            ie: self.ie,
            if_: self.if_,
            ime: f[0],
            ei_pending: f[1],
            halted: f[2],
            halt_bug: f[3],
            halt_dispatch: f[4],
            stopped: f[5],
            locked: f[6],
            stop_if: self.stop_if,
        }
    }
}

impl GameBoy {
    /// The state between two steps, as bytes.
    pub fn save_state(&self) -> Vec<u8> {
        let cpu = Cpu::from(&self.cpu.capture_state());
        let mut out = MAGIC.to_vec();
        out.extend(bincode::serialize(&(cpu, &self.hw)).expect("state"));
        out
    }

    /// Load a state saved by `save_state` from a console with the same cartridge and machine.
    pub fn load_state(&mut self, data: &[u8]) -> Result<(), String> {
        if data.len() < 8 || &data[..8] != MAGIC {
            return Err("not a gameboy-rust state".into());
        }
        let (cpu, mut hw): (Cpu, Hw) =
            bincode::deserialize(&data[8..]).map_err(|e| e.to_string())?;
        if hw.cgb != self.hw.cgb || hw.cartridge.ram.len() != self.hw.cartridge.ram.len() {
            return Err("the state is from another machine or cartridge".into());
        }
        let old = &mut self.hw;
        hw.machine = old.machine;
        hw.boot_rom = old.boot_rom.take();
        hw.cartridge.header = old.cartridge.header.clone();
        hw.cartridge.rom = std::mem::take(&mut old.cartridge.rom);
        hw.bus_log = old.bus_log.take();
        self.hw = hw;
        self.cpu.restore_state(&cpu.to());
        Ok(())
    }
}

/// gameboy-python's `BoardState`: the CPU and the board registers it cannot see (comparison and
/// reporting only).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardState {
    pub cpu: CpuState,
    pub t: u64,
    pub div_counter: u32,
    pub tima: u32,
    pub tma: u8,
    pub tac: u8,
    pub lcdc: u8,
    pub stat: u8,
    pub ly: i32,
    pub lyc: u8,
    pub dot: i32,
    pub dma_active: bool,
    pub boot_mapped: bool,
    pub serial_sb: u8,
    pub serial_sc: u8,
}

pub fn capture_board_state(gb: &GameBoy) -> BoardState {
    let ppu = &gb.hw.ppu;
    BoardState {
        cpu: gb.cpu.capture_state(),
        t: gb.hw.t,
        div_counter: gb.hw.timer.counter,
        tima: gb.hw.timer.tima,
        tma: gb.hw.timer.tma,
        tac: gb.hw.timer.tac,
        lcdc: ppu.lcdc,
        stat: ppu.read_stat(),
        ly: if ppu.lcd_on() { ppu.visible_ly() } else { 0 },
        lyc: ppu.lyc,
        dot: ppu.dot,
        dma_active: ppu.dma.active,
        boot_mapped: gb.hw.boot_mapped,
        serial_sb: gb.hw.serial.sb,
        serial_sc: gb.hw.serial.sc,
    }
}
