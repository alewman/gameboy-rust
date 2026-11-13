//! The Game Boy: the bus that decodes the CPU's M-cycles and advances every unit inside them
//! (gameboy-python `board.py`).
//!
//! sm83 makes one bus call per M-cycle, internal cycles included. In each call this board first
//! advances the system counter, timer, serial port, PPU and OAM DMA by one M-cycle (four
//! T-cycles), then completes the access; it never adds `step()`'s return value on top
//! (gameboy-python docs/timing.md "The design").
//!
//! IE ($FFFF) and IF ($FF0F) live in the core: the board raises an IF bit with
//! `cpu.request_interrupt(bit, ..)` from inside a call, and answers CPU accesses to $FF0F from
//! `cpu.if_` (the core already answers $FFFF).
//!
//! Where Rust forced a difference: the CPU is passed to the bus (sm83's `Bus`), so the console
//! is [`GameBoy`] = the CPU + [`Hw`] (everything else, the `Bus`). The units' interrupt
//! callbacks (`_timer_irq`, `_serial_irq`, `_ppu_irq`) are called by the board right where the
//! reference's units called them; the PPU's goes through [`IrqCtx`], the CPU and the board's
//! interrupt state ([`IrqBoard`]), which every PPU method that can raise one takes. The
//! reference's `cpu.step = self.step` (CGB: HDMA runs at instruction boundaries) is
//! [`GameBoy::step`] on every machine (a DMG never arms HDMA, so it is the same). An optional
//! bus log (`Hw::bus_log`) records every M-cycle for the lockstep gate.

use serde::{Deserialize, Serialize};
use sm83::{CpuState, Sm83, StopAction, StopResult};

use crate::apu::{self, APU};
use crate::cartridge::Cartridge;
use crate::dma::{bus_of, Bus as DmaBus};
use crate::joypad::Joypad;
use crate::machines::{Fea0Read, Machine, DMG_B};
use crate::ppu::{self, OamBugKind, Ppu};
use crate::serial::Serial;
use crate::timer::Timer;

/// post-boot register values for DMG-CPU A/B/C (Pan Docs Power_Up_Sequence.md "CPU registers",
/// confirmed by Mooneye boot_regs-dmgABC); F's Z is always set, H and C follow the header
/// checksum: a, b, c, d, e, h, l
pub const POST_BOOT_DMG: [u8; 7] = [0x01, 0x00, 0x13, 0x00, 0xD8, 0x01, 0x4D];
/// ... and for a CGB (Pan Docs Power_Up_Sequence.md): a CGB game, and a DMG game in
/// compatibility mode
pub const POST_BOOT_CGB: [u8; 7] = [0x11, 0x00, 0x00, 0xFF, 0x56, 0x00, 0x0D];
pub const POST_BOOT_CGB_COMPAT: [u8; 7] = [0x11, 0x00, 0x00, 0x00, 0x08, 0x99, 0x1A];

/// the CGB boot ROM is $0000-$00FF and $0200-$08FF; $0100-$01FF is the cartridge header
pub const CGB_BOOT_SIZE: usize = 0x900;
/// STOP's speed switch halts the CPU this many units (4 per M-cycle) unless an interrupt ends
/// it earlier (SameBoy `speed_switch_halt_countdown`; Gambatte speedchange)
pub const SPEED_SWITCH_HALT: i32 = 0x20008; // units, switching from double to single speed
pub const SPEED_SWITCH_HALT_TO_DOUBLE: i32 = 0x20008; // ... and from single to double; FITTED
/// dots the LCD runs on its own when the speed switches to double, and to single (negative: it
/// stands still). FITTED on Gambatte speedchange/lcd_offset/lcdoffset1 (230 -> 264 of 357;
/// -4/3, -2/2, 1/2, -3/3 all lower). SameBoy's TODO has 2 PPU ticks single->double.
pub const SWITCH_LCD_DS: i32 = 2;
/// dots LY shows the next line early after a switch to double speed. daid's
/// speed_switch_timing_ly.gbc wants 2 (as after an LCD-on in double speed: ppu.LY_LEAD_DS), but
/// 2 loses Gambatte ly0/lycint152_ly153_ds_1/_4 and lycint_ly/lycint_ly_ds_1 (CGB-04c hardware
/// verdicts) for one gain, and 1 satisfies neither: the hardware verdicts stand.
pub const SWITCH_LY_LEAD: i32 = 0;
pub const SWITCH_LCD_SS: i32 = -3;

/// FITTED: IF bits raised in a write's own M-cycle survive the write
pub const IF_WRITE_PHASE: i32 = 0;

/// dots; fitted, see serial_irq
pub const SERIAL_IF_HIDE: i32 = 4;
/// a DIV write's fall clocks the serial toggle (Gambatte serial/*_div_write_*)
pub const SERIAL_DIV_EDGE: bool = true;
/// an SC write while the interrupt hides cancels it (Gambatte serial/*_sc80_*, *_stop_*)
pub const SERIAL_SC_CANCEL: bool = true;
/// units (4 per M-cycle) a transfer takes after the opcode fetch before its first byte: SameBoy
/// GB_hdma_run's 2, 4 in double speed
pub const HDMA_START: i32 = 2;
pub const HDMA_START_DS: i32 = 4;
/// FITTED (Gambatte dma/hdma_start_*, hdma_cycles_*, gdma_cycles_* +30, nothing lost; SameBoy
/// sets hdma_on 3 T after mode 0): dots into HBlank before an HBlank block falls due
pub const HDMA_HBLANK_LAG: i32 = 4;
pub const HDMA_HBLANK_LAG_DS: i32 = 2;
/// ... and after its last, at single speed; FITTED
pub const HDMA_END: i32 = 2;
/// FITTED (Mooneye misc/ppu/vblank_stat_intr-C)
pub const VBLANK_CGB_DELAY: i32 = 4;
/// FITTED (GBMicrotest int_timer_halt, int_timer_halt_div_b)
pub const TIMER_HALT_DELAY: i32 = 4;
/// HALT decides on IF at the end of its fetch for a STAT interrupt raised in that fetch, and
/// samples IF at the start of its second cycle, so one raised there waits for the next cycle
/// (SameBoy halt() and GB_cpu_run's just_halted; Gambatte halt/late_m0i*_scx2/3, both models)
pub const HALT_SEES_STAT: bool = true;
pub const HALT_CYCLE_HALTED: bool = true;
/// a dispatch's low-byte push samples IF in its cycle's first dot, so an HBlank interrupt raised
/// after that is not chosen and a pending one raised again stays set (Gambatte irq_precedence
/// late_m0irq_vs_tima_*_2 and late_m0irq_retrigger_scx1_1; SameBoy reads IF at the push)
pub const DISPATCH_SAMPLE_DOT: i32 = 1;

/// the OAM-bug pattern an IDU step without an access causes
pub const OAM_IDLE_KIND: OamBugKind = OamBugKind::Write;

/// An interrupt held back (the reference's `_late` items: `[dots, bit]` or, deferred past the
/// PPU's lead, `[sub, bit, early, halt_delay]`).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Late {
    Simple {
        left: i32,
        bit: u8,
    },
    Deferred {
        sub: i32,
        bit: u8,
        early: i32,
        halt_delay: i32,
    },
}

/// The board's interrupt bookkeeping, which the units' interrupt handlers touch.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IrqBoard {
    pub cgb: bool,
    /// IF bits raised in the last dots of the current M-cycle
    pub late_bits: u8,
    /// IF bits raised early for the CPU's dispatch, not yet readable
    pub hide: u8,
    pub hide_left: i32,
    /// this M-cycle is HALT's own second one
    pub halt_cycle: bool,
    /// OAM interrupts held back while the CPU is halted
    pub late: Vec<Late>,
}

/// What a PPU interrupt handler reaches: the CPU and the board's interrupt state.
pub struct IrqCtx<'a> {
    pub cpu: &'a mut Sm83,
    pub b: &'a mut IrqBoard,
}

/// An HBlank interrupt raised after the dot where a dispatch's low-byte push samples IF.
fn dispatch_late(ppu: &Ppu, bit: u8, early: i32) -> bool {
    let hblank = bit == 1 && early == ppu::HBLANK_IRQ_EARLY;
    hblank && ppu.sub >= DISPATCH_SAMPLE_DOT
}

/// An HBlank interrupt reaching IF before its source rises: a STAT write clearing bit 3 later
/// in this M-cycle cancels it (`ppu::M0_CANCEL_DMG`, `ppu::M0_CANCEL_CGB`).
fn m0_cancellable(ppu: &mut Ppu, cgb: bool, bit: u8, early: i32, deferred: bool) {
    if bit != 1 || early != ppu::HBLANK_IRQ_EARLY {
        return;
    }
    if cgb {
        let sub = ppu.sub >= ppu::M0_CANCEL_CGB && !deferred;
        ppu.m0_early = ppu::M0_CANCEL_CGB != 0 && sub;
    } else {
        ppu.m0_early = ppu::M0_CANCEL_DMG != 0;
    }
}

/// A PPU interrupt (the reference's `GameBoy._ppu_irq`). A halted CPU sees the STAT ones a
/// fitted number of dots after the PPU raises them (`ppu.irq_halt_delay`); the line-start OAM
/// one reaches the CPU's dispatch a cycle before its IF bit shows.
pub fn ppu_irq(ppu: &mut Ppu, ctx: &mut IrqCtx, bit: u8) {
    let b = &mut *ctx.b;
    if bit == 0 && b.cgb && VBLANK_CGB_DELAY != 0 {
        // CGB: the VBlank interrupt comes this many dots after the line-144 OAM one (Mooneye
        // misc/ppu/vblank_stat_intr-C; the DMG's are simultaneous)
        b.late.push(Late::Simple {
            left: VBLANK_CGB_DELAY,
            bit: 0,
        });
        return;
    }
    let delay = if bit == 1 && ctx.cpu.halted {
        ppu.irq_halt_delay
    } else {
        0
    };
    // the dot within the M-cycle as it would be without the PPU's lead (IRQ_LEAD)
    let sub = ppu.sub + if ppu.double_speed { 0 } else { ppu::IRQ_LEAD };
    if bit == 1 && ppu.irq_defer {
        // raised in the dots after the access: the dispatch sees it a cycle later, as if the
        // PPU had no lead (IRQ_LEAD)
        ppu.irq_defer = false;
        b.late.push(Late::Deferred {
            sub,
            bit,
            early: ppu.irq_early_by,
            halt_delay: ppu.irq_halt_delay,
        });
        return;
    }
    if delay != 0 && sub + delay >= 4 {
        b.late.push(Late::Simple {
            left: sub + delay,
            bit,
        });
        return;
    }
    if bit == 1 && b.halt_cycle {
        b.late.push(Late::Simple { left: 4, bit }); // HALT's second cycle sampled IF at its start
        return;
    }
    let late = dispatch_late(ppu, bit, ppu.irq_early_by);
    m0_cancellable(ppu, b.cgb, bit, ppu.irq_early_by, false);
    ctx.cpu
        .request_interrupt(bit, bit == 1 && HALT_SEES_STAT, late);
    if ppu.sub >= IF_WRITE_PHASE {
        b.late_bits |= 1 << bit;
    }
    if (bit == 0 || bit == 1) && ppu.irq_early_by != 0 {
        // the dispatch decision sees this interrupt a cycle early; reads of IF do not
        b.hide |= 1 << bit;
        b.hide_left = ppu.irq_early_by;
    }
}

/// A PPU memory kept from it during STOP.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Held {
    Vram,
    BgPal,
    ObjPal,
}

/// Everything on the board but the CPU: sm83's `Bus`. (Not `Clone`/`Debug`: gb_apu's `APU` is
/// neither.)
#[derive(Serialize, Deserialize)]
pub struct Hw {
    #[serde(skip, default = "default_machine")]
    pub machine: Machine,
    pub cartridge: Cartridge,
    /// PPU dots (4.19 MHz T-cycles of real time) since power-on
    pub t: u64,
    pub cgb: bool,
    pub wram: Vec<u8>,
    /// the WRAM bank at $D000 (CGB mode: SVBK, 0 reads as 1)
    pub svbk: usize,
    pub hram: Vec<u8>,
    #[serde(skip)]
    pub boot_rom: Option<Vec<u8>>,
    pub boot_mapped: bool,
    pub double_speed: bool,
    /// bit 0: a speed switch is armed
    pub key1: u8,
    /// units left of the speed switch's halt
    pub switch_wait: i32,
    /// dots the PPU skips (a speed switch's negative skew)
    pub lcd_freeze: i32,
    /// a speed switch's STOP clears DIV in its operand read
    pub div_reset_due: bool,
    /// M-cycles the counter then stands still for
    pub div_hold: i32,
    pub switch_halted: bool,
    pub key0: u8,
    pub rp: u8,
    /// $FF72-$FF75
    pub undocumented: [u8; 4],
    pub svbk_register: u8,
    pub opri_register: u8,
    pub hdma_src: u32,
    pub hdma_dst: u32,
    /// 16-byte blocks left
    pub hdma_blocks: u32,
    /// an HBlank transfer is armed
    pub hdma_hblank: bool,
    /// a block is to run before the next instruction
    pub hdma_due: bool,
    /// ... at the end of this step's opcode fetch
    pub hdma_at_fetch: bool,
    pub idle_units: i32,
    /// STOP mode: 1 from the STOP hook, 2 once a stopped cycle has run
    pub stopped: u8,
    /// memories kept from the PPU in STOP
    pub stop_held: Vec<(Held, Vec<u8>)>,
    pub last_mode: u8,
    pub timer: Timer,
    pub serial: Serial,
    pub joypad: Joypad,
    pub irq: IrqBoard,
    pub ppu: Ppu,
    #[serde(skip, default = "apu::placeholder")]
    pub apu: APU,
    /// every M-cycle (`t << 26 | kind << 24 | address << 8 | value`; kind 0 read, 1 write, 2
    /// internal), when the lockstep gate asks for it
    #[serde(skip)]
    pub bus_log: Option<Vec<u64>>,
}

fn default_machine() -> Machine {
    DMG_B
}

/// One console: a machine row, a cartridge, an optional boot ROM, and all the units.
pub struct GameBoy {
    pub cpu: Sm83,
    pub hw: Hw,
}

impl GameBoy {
    pub fn new(
        cartridge: Cartridge,
        machine: Machine,
        boot_rom: Option<Vec<u8>>,
    ) -> Result<GameBoy, String> {
        let cgb = machine.cgb_hardware;
        let boot_mapped = boot_rom.is_some();
        let size = if cgb { CGB_BOOT_SIZE } else { 0x100 };
        if let Some(rom) = &boot_rom {
            if rom.len() != size {
                return Err(format!(
                    "a {} boot ROM is {} bytes",
                    machine.model.to_uppercase(),
                    size
                ));
            }
        }
        let mut timer = Timer::new(machine.tac_disable_glitch);
        // The counter's power-on phase: DIV reads in Mooneye boot_div-dmgABCmgb need the counter
        // 4 T ahead of the M-cycle count since reset (fitted; the only phase that passes it)
        timer.counter = 4;
        let mut ppu = Ppu::new(machine.stat_write_glitch, machine.stat_blocking, cgb);
        let pixels = ppu.pixels.as_mut().expect("pixels");
        pixels.scy_at_data_fetch = machine.scy_at_data_fetch;
        pixels.palette_late = machine.palette_write_late;
        ppu.lyc153_late = machine.lyc153_4_dots_later;
        ppu.cgb_mode = cgb; // the boot ROM runs in CGB mode and picks compatibility by KEY0
        let hw = Hw {
            machine,
            cartridge,
            t: 0,
            cgb,
            wram: vec![0; if cgb { 0x8000 } else { 0x2000 }],
            svbk: 1,
            hram: vec![0; 0x7F],
            boot_rom,
            boot_mapped,
            double_speed: false,
            key1: 0,
            switch_wait: 0,
            lcd_freeze: 0,
            div_reset_due: false,
            div_hold: 0,
            switch_halted: false,
            key0: 0,
            rp: 0,
            undocumented: [0; 4],
            svbk_register: 0,
            opri_register: 0,
            hdma_src: 0,
            hdma_dst: 0,
            hdma_blocks: 0,
            hdma_hblank: false,
            hdma_due: false,
            hdma_at_fetch: false,
            idle_units: 0,
            stopped: 0,
            stop_held: Vec::new(),
            last_mode: 0,
            timer,
            serial: Serial::new(),
            joypad: Joypad::new(),
            irq: IrqBoard {
                cgb,
                ..IrqBoard::default()
            },
            ppu,
            apu: apu::new(cgb),
            bus_log: None,
        };
        let mut gb = GameBoy {
            cpu: Sm83::new(),
            hw,
        };
        if gb.hw.boot_rom.is_none() {
            gb.skip_boot();
        }
        Ok(gb)
    }

    // ----- power-on ---------------------------------------------------------------------

    fn post_boot(regs: [u8; 7], f: u8) -> CpuState {
        CpuState {
            a: regs[0],
            b: regs[1],
            c: regs[2],
            d: regs[3],
            e: regs[4],
            h: regs[5],
            l: regs[6],
            f,
            sp: 0xFFFE,
            pc: 0x0100,
            if_: 0x01,
            ..CpuState::default()
        }
    }

    /// The state the DMG boot ROM leaves at $0100 (Pan Docs Power_Up_Sequence.md).
    fn skip_boot(&mut self) {
        let f = 0x80
            | if self.hw.cartridge.header.header_checksum != 0 {
                0x30
            } else {
                0
            };
        if self.hw.cgb {
            self.skip_boot_cgb();
            return;
        }
        self.cpu.restore_state(&Self::post_boot(POST_BOOT_DMG, f));
        let hw = &mut self.hw;
        // DIV $AB; the low byte is the counter phase at handoff: what dmg-sim's quickboot leaves
        // on this board and the only phase that passes Mooneye boot_div-dmgABCmgb
        hw.timer.counter = 0xABC8;
        hw.serial.sc = 0;
        hw.ppu.dma.register = 0xFF;
        hw.ppu.bgp = 0xFC;
        let mut ctx = IrqCtx {
            cpu: &mut self.cpu,
            b: &mut hw.irq,
        };
        hw.ppu.write_lcdc(0x91, &mut ctx);
        hw.ppu.stat_select = 0;
        // the PPU is late in line 153 (LY reads 0, mode 1, STAT $85) at handoff, as quickboot
        // leaves it
        hw.ppu.place(153, 396, 1, &mut ctx);
        hw.apu.skip_boot();
        hw.boot_mapped = false;
    }

    /// The state the CGB boot ROM leaves at $0100 (Pan Docs Power_Up_Sequence.md). A DMG game is
    /// left in compatibility mode with the boot ROM's default palettes (white to black).
    fn skip_boot_cgb(&mut self) {
        let header = &self.hw.cartridge.header;
        let cgb_game = header.cgb_enhanced();
        let cgb_flag = header.cgb_flag;
        let regs = if cgb_game {
            POST_BOOT_CGB
        } else {
            POST_BOOT_CGB_COMPAT
        };
        self.cpu.restore_state(&Self::post_boot(regs, 0x80));
        let hw = &mut self.hw;
        hw.ppu.cgb_mode = cgb_game;
        hw.opri_register = if cgb_game { 0 } else { 1 };
        hw.ppu.opri = hw.opri_register;
        hw.key0 = if cgb_game { cgb_flag } else { 0x04 };
        // measured by running the real CGB boot ROM on this board (2026-10-10, cgb-acid2): DIV
        // counter $1E9C, the PPU at line 144 dot 164, white background palettes. FITTED to that
        // run for a CGB game; the compatibility-mode values are the same, unmeasured.
        hw.timer.counter = 0x1E9C;
        hw.serial.sc = 0;
        hw.ppu.dma.register = 0xFF;
        hw.ppu.bgp = 0xFC;
        for entry in 0..8 {
            for k in 0..4 {
                hw.ppu.bg_pal[entry * 8 + k * 2] = 0xFF;
                hw.ppu.bg_pal[entry * 8 + k * 2 + 1] = 0x7F;
            }
        }
        hw.ppu.bcps = 0x80;
        hw.ppu.ocps = 0x81;
        let mut ctx = IrqCtx {
            cpu: &mut self.cpu,
            b: &mut hw.irq,
        };
        hw.ppu.write_lcdc(0x91, &mut ctx);
        hw.ppu.stat_select = 0;
        hw.ppu.place(144, 164, 1, &mut ctx);
        hw.apu.skip_boot();
        hw.boot_mapped = false;
    }

    // ----- running ----------------------------------------------------------------------

    /// Execute one CPU boundary; returns the M-cycles it took (the units have already advanced
    /// inside the calls).
    pub fn step(&mut self) -> u32 {
        let cpu = &self.cpu;
        if self.hw.hdma_due || self.hw.hdma_hblank {
            let pending = cpu.interrupt_pending();
            // SameBoy GB_cpu_run: the transfer runs after the next opcode fetch, not on a
            // dispatch's discarded one; an HBlank block that falls due during the fetch included
            self.hw.hdma_at_fetch = (!cpu.halted || pending) && !(cpu.ime && pending);
        }
        self.cpu.step(&mut self.hw)
    }

    /// Run until the PPU has completed `frames` more VBlanks.
    pub fn run_frames(&mut self, frames: u64, max_steps: u64) {
        let target = self.hw.ppu.frames + frames;
        let mut steps = 0;
        while self.hw.ppu.frames < target && steps < max_steps {
            self.step();
            steps += 1;
        }
    }

    /// The byte at `address` as the CPU would read it now, with no DMA conflict (debuggers,
    /// harnesses).
    pub fn peek(&mut self, address: u16) -> u8 {
        self.hw.peek(address, &self.cpu)
    }

    /// A host-side button change; raises the joypad interrupt on a selected press.
    pub fn set_pressed(&mut self, name: &str, down: bool) {
        if self.hw.joypad.set_pressed(name, down) {
            self.cpu.request_interrupt(4, false, false);
        }
    }
}

impl Hw {
    // ----- time -------------------------------------------------------------------------

    /// The timer's interrupt wakes a halted CPU TIMER_HALT_DELAY dots late (GBMicrotest
    /// int_timer_halt*).
    fn timer_irq(&mut self, cpu: &mut Sm83, bit: u8) {
        if TIMER_HALT_DELAY != 0 && cpu.halted {
            self.irq.late.push(Late::Simple {
                left: TIMER_HALT_DELAY,
                bit,
            });
        } else {
            cpu.request_interrupt(bit, false, false);
        }
    }

    /// The serial interrupt reaches the CPU's dispatch on time while IF shows it
    /// SERIAL_IF_HIDE dots later (FITTED: Gambatte serial/*_read_if_*).
    fn serial_irq(&mut self, cpu: &mut Sm83, bit: u8) {
        cpu.request_interrupt(bit, false, false);
        if SERIAL_IF_HIDE != 0 {
            self.irq.hide |= 1 << bit;
            self.irq.hide_left = SERIAL_IF_HIDE;
        }
    }

    fn take_serial_irq(&mut self, cpu: &mut Sm83) {
        if self.serial.take_irq() {
            self.serial_irq(cpu, 3);
        }
    }

    fn finish_timer(&mut self, cpu: &mut Sm83) {
        if self.timer.finish() {
            self.timer_irq(cpu, 2);
        }
    }

    fn after_access(&mut self, cpu: &mut Sm83) {
        let mut ctx = IrqCtx {
            cpu,
            b: &mut self.irq,
        };
        self.ppu.after_access(&mut ctx);
    }

    /// One M-cycle of the whole board.
    fn tick(&mut self, cpu: &mut Sm83) {
        let dots = if self.double_speed { 2 } else { 4 }; // the PPU and APU run at the single-speed rate
        self.t += dots as u64;
        if self.stopped > 1 {
            self.leave_stop();
        }
        self.irq.late_bits = 0;
        self.ppu.m0_early = false; // only a write in the M-cycle of the raise cancels it
        self.ppu.m0_hidden = if self.ppu.m0_hidden == 2 { 3 } else { 0 };
        self.ppu.stat_fell = 0;
        self.ppu.oam_hidden = false;
        self.ppu.lyc_rose = false;
        if self.switch_wait != 0 {
            self.switch_wait -= 4;
            if cpu.halted {
                self.switch_halted = true;
            }
            if self.switch_wait <= 0 {
                cpu.halted = false;
                self.switch_wait = 0;
            } else if self.switch_halted && !cpu.halted {
                self.switch_wait = 0; // an interrupt woke it first
            }
        } else {
            self.switch_halted = false;
        }
        if self.irq.hide != 0 {
            self.irq.hide_left -= 4;
            if self.irq.hide_left <= 0 {
                self.irq.hide = 0;
            }
        }
        if !self.irq.late.is_empty() {
            let mut held = Vec::new();
            let items = std::mem::take(&mut self.irq.late);
            for item in items {
                match item {
                    Late::Deferred {
                        sub,
                        bit,
                        early,
                        halt_delay,
                    } => {
                        // deferred past the lead: raised now, at dot sub
                        let sub = sub - 4;
                        let mut delay = if cpu.halted { halt_delay } else { 0 };
                        if self.irq.halt_cycle {
                            delay = 4 - sub; // HALT's second cycle sampled IF at its start
                        }
                        if sub + delay >= 4 {
                            // a halted CPU sees it later still
                            held.push(Late::Simple {
                                left: sub + delay,
                                bit,
                            });
                            continue;
                        }
                        let late = dispatch_late(&self.ppu, bit, early);
                        m0_cancellable(&mut self.ppu, self.cgb, bit, early, true);
                        let halt_sees = bit == 1 && HALT_SEES_STAT;
                        cpu.request_interrupt(bit, halt_sees, late);
                        self.irq.late_bits |= 1 << bit;
                        if early != 0 {
                            self.irq.hide |= 1 << bit;
                            self.irq.hide_left = early;
                        }
                    }
                    Late::Simple { left, bit } => {
                        let left = left - 4;
                        if left < 4 {
                            cpu.request_interrupt(bit, false, false);
                        } else {
                            held.push(Late::Simple { left, bit });
                        }
                    }
                }
            }
            // (an interrupt raised by the loop itself is not queued: request_interrupt only)
            self.irq.late = held;
        }
        if self.div_reset_due {
            self.div_reset_due = false;
            self.timer.tick(4);
            self.write_div();
            self.timer.previous = self.timer.counter; // the reset's falling edges were counted by write_div
        } else if self.div_hold != 0 {
            self.div_hold -= 1;
            self.timer.tick(0);
        } else {
            self.timer.tick(4);
        }
        let apu_bit = if self.double_speed { 0x2000 } else { 0x1000 };
        if self.timer.previous & apu_bit != 0 && self.timer.counter & apu_bit == 0 {
            self.apu.frame_tick(); // DIV-APU: a falling edge of the system counter's bit 12 (13)
        } else if self.timer.previous & apu_bit == 0 && self.timer.counter & apu_bit != 0 {
            self.apu.secondary_tick(); // and the rising edge half a period later
        }
        self.apu.tick(dots as u32);
        self.serial.tick(self.timer.previous, self.timer.counter);
        self.take_serial_irq(cpu);
        {
            let mut ctx = IrqCtx {
                cpu: &mut *cpu,
                b: &mut self.irq,
            };
            if self.lcd_freeze != 0 {
                let held = dots.min(self.lcd_freeze);
                self.lcd_freeze -= held;
                self.ppu.tick(dots - held, &mut ctx);
            } else {
                self.ppu.tick(dots, &mut ctx);
            }
        }
        self.dma_tick();
        if self.hdma_hblank {
            let mut mode = self.ppu.mode;
            let lag = if self.double_speed {
                HDMA_HBLANK_LAG_DS
            } else {
                HDMA_HBLANK_LAG
            };
            if mode == 0 && self.ppu.mode0_dots < lag {
                mode = 3;
            }
            if mode == 0 && self.last_mode != 0 && self.ppu.lcd_on() {
                self.hdma_due = true; // an HBlank transfer's next block
            }
            self.last_mode = mode;
        }
    }

    /// The OAM DMA's M-cycle (the reference's `OamDma.tick` with its two board callbacks).
    fn dma_tick(&mut self) {
        if let Some(source) = self.ppu.dma.begin_tick() {
            let byte = self.dma_source(source);
            let index = self.ppu.dma.end_tick(byte);
            self.ppu.oam[index] = byte; // _dma_write
        }
    }

    fn write_div(&mut self) {
        // Writing DIV clears the counter: a high bit 12 falls, which ticks DIV-APU.
        if self.timer.counter & if self.double_speed { 0x2000 } else { 0x1000 } != 0 {
            self.apu.frame_tick();
        }
        if SERIAL_DIV_EDGE {
            self.serial.div_reset(self.timer.counter);
        }
        self.timer.write_div();
    }

    /// STOP (Pan Docs "Using the STOP instruction", SonoSooS "NOP and STOP"; SameBoy stop()).
    ///
    /// With a selected button held there is no stop: a pending interrupt makes it a one-byte
    /// no-op, else it is a two-byte HALT. On a CGB in CGB mode with KEY1 bit 0 set it switches
    /// speed instead: the CPU halts for SPEED_SWITCH_HALT units unless an interrupt wakes it
    /// first. Otherwise it clears DIV and enters STOP mode (one byte long when an interrupt is
    /// pending), which only the joypad ends: `stop_cycle`.
    fn stop_hook(&mut self, pending: bool, cpu: &mut Sm83) -> StopResult {
        let held = self.joypad.read() & 0x0F != 0x0F;
        if held {
            if pending {
                return StopResult::with_operand(StopAction::Nop, false);
            }
            return StopResult::new(StopAction::Halt);
        }
        if self.cgb && self.ppu.cgb_mode && self.key1 & 1 != 0 {
            // the switch clears DIV like any STOP (Gambatte stop(), SameBoy), at the end of STOP's
            // operand read: a counter bit that is still high there makes its falling edge
            // (Gambatte speedchange*_tima _1/_2 pairs move STOP by one M-cycle). With IME off the
            // CPU then holds the reset one M-cycle more (SameBoy enter_stop_mode;
            // Gambatte speedchange*_div pairs).
            if pending {
                self.write_div();
                self.take_serial_irq(cpu);
            } else {
                self.div_reset_due = true;
            }
            self.div_hold = i32::from(!cpu.ime);
            self.double_speed = !self.double_speed;
            self.ppu.double_speed = self.double_speed;
            self.ppu.ly_lead = if self.double_speed { SWITCH_LY_LEAD } else { 0 };
            self.key1 = 0;
            // the PPU runs IRQ_LEAD dots ahead of the access in single speed, none in double
            let lead = ppu::IRQ_LEAD;
            let skew = if self.double_speed {
                SWITCH_LCD_DS - lead
            } else {
                SWITCH_LCD_SS + lead
            };
            if skew > 0 {
                let mut ctx = IrqCtx {
                    cpu: &mut *cpu,
                    b: &mut self.irq,
                };
                self.ppu.run_dots(skew, &mut ctx);
            } else if skew < 0 {
                self.lcd_freeze = -skew; // the LCD stands still this many dots
            }
            if pending {
                return StopResult::with_operand(StopAction::Nop, false);
            }
            self.switch_wait = if !self.double_speed {
                SPEED_SWITCH_HALT
            } else {
                SPEED_SWITCH_HALT_TO_DOUBLE
            };
            return StopResult::new(StopAction::Halt);
        }
        self.write_div();
        self.take_serial_irq(cpu);
        self.stopped = 1;
        if self.cgb {
            // the PPU runs on, but a memory the CPU could reach when it stopped stays with the
            // CPU: the PPU reads $FF from the VRAM and black from the palettes (SameBoy
            // enter_stop_mode; daid's stop_instr.gb is black all over). A STOP in mode 3 leaves
            // the PPU both (stop_instr_gbc_mode3.gb: the picture stays).
            let mut kept: Vec<(Held, u8)> = Vec::new();
            if !self.ppu.vram_locked() {
                kept.push((Held::Vram, 0xFF));
            }
            if !self.ppu.palettes_blocked() {
                kept.push((Held::BgPal, 0));
                kept.push((Held::ObjPal, 0));
            }
            for (name, fill) in kept {
                let memory = match name {
                    Held::Vram => &mut self.ppu.vram,
                    Held::BgPal => &mut self.ppu.bg_pal,
                    Held::ObjPal => &mut self.ppu.obj_pal,
                };
                let blank = vec![fill; memory.len()];
                let old = std::mem::replace(memory, blank);
                self.stop_held.push((name, old));
            }
        } else if self.ppu.lcd_on() {
            // the oscillator stops and the LCD goes blank (stop_instr.gb)
            self.ppu.frame = self.ppu.blank();
        }
        StopResult::with_operand(StopAction::Stop, !pending)
    }

    /// One M-cycle of STOP mode. A DMG's oscillator is off: nothing runs. On a CGB the PPU and
    /// the APU run on and the counter stands still (SameBoy GB_timers_run). No interrupt ends
    /// it; a selected button line going low does (SameBoy GB_cpu_run). The sound is clocked on
    /// a DMG too, so that a host still gets its samples: a labelled approximation.
    fn stop_cycle(&mut self, cpu: &mut Sm83) {
        self.stopped = 2;
        let dots = if self.double_speed { 2 } else { 4 };
        self.t += dots as u64;
        self.apu.tick(dots as u32);
        if self.cgb {
            let mut ctx = IrqCtx {
                cpu: &mut *cpu,
                b: &mut self.irq,
            };
            self.ppu.tick(dots, &mut ctx);
            self.ppu.after_access(&mut ctx);
        }
        cpu.stop_if = cpu.if_ & 0x1F; // an interrupt raised meanwhile waits; it does not wake
        if self.joypad.read() & 0x0F != 0x0F {
            cpu.stopped = false;
        }
    }

    fn leave_stop(&mut self) {
        self.stopped = 0;
        for (name, memory) in std::mem::take(&mut self.stop_held) {
            match name {
                Held::Vram => self.ppu.vram = memory,
                Held::BgPal => self.ppu.bg_pal = memory,
                Held::ObjPal => self.ppu.obj_pal = memory,
            }
        }
    }

    // ----- the bus ----------------------------------------------------------------------

    fn bug(&mut self, address: u16, kind: Option<OamBugKind>) {
        if let Some(kind) = kind {
            if self.machine.oam_bug && (0xFE00..=0xFEFF).contains(&address) {
                self.ppu.corrupt_oam(kind);
            }
        }
    }

    fn log(&mut self, kind: u64, address: u16, value: u8) {
        if let Some(log) = self.bus_log.as_mut() {
            log.push((self.t << 26) | (kind << 24) | ((address as u64) << 8) | value as u64);
        }
    }

    fn bus_read(&mut self, address: u16, cpu: &mut Sm83) -> u8 {
        self.irq.halt_cycle = HALT_CYCLE_HALTED
            && cpu.idu == 0
            && address == cpu.pc
            && self.peek(address.wrapping_sub(1), cpu) == 0x76;
        self.tick(cpu);
        self.irq.halt_cycle = false;
        self.bug(
            address,
            Some(if cpu.idu != 0 {
                OamBugKind::ReadInc
            } else {
                OamBugKind::Read
            }),
        );
        let value = self.load(address, cpu);
        self.after_access(cpu);
        self.finish_timer(cpu);
        if self.hdma_at_fetch {
            self.hdma_at_fetch = false;
            if self.hdma_due {
                self.hdma_run(cpu);
            }
        }
        value
    }

    fn bus_write(&mut self, address: u16, value: u8, cpu: &mut Sm83) {
        self.tick(cpu);
        self.bug(address, Some(OamBugKind::Write));
        let dma_busy = self.ppu.dma.active && self.ppu.dma.index != 0;
        if dma_busy && self.machine.cgb_hardware {
            if self.cgb_dma_conflict(address) {
                self.cgb_dma_write(address, value);
                self.after_access(cpu);
                self.finish_timer(cpu);
                return;
            }
        } else if dma_busy
            && address < 0xFE00
            && bus_of(address) == self.ppu.dma.bus
            && self.ppu.dma.bus != DmaBus::Internal
        {
            // a CPU write on the bus the DMA is reading never reaches memory; the byte lands in
            // OAM instead: as written from a cartridge source, ANDed with the DMA's byte from
            // WRAM (DMG; Gambatte oamdma_src*_busypush*)
            let i = self.ppu.dma.index - 1;
            if self.ppu.dma.base < 0xC0 {
                self.ppu.oam[i] = value;
            } else {
                self.ppu.oam[i] &= value;
            }
            self.after_access(cpu);
            self.finish_timer(cpu);
            return;
        }
        self.store(address, value, cpu);
        self.after_access(cpu);
        self.finish_timer(cpu);
    }

    fn bus_idle(&mut self, address: u16, cpu: &mut Sm83) {
        if self.stopped != 0 && cpu.stopped {
            self.stop_cycle(cpu);
            return;
        }
        self.tick(cpu);
        self.bug(
            address,
            if cpu.idu != 0 {
                Some(OAM_IDLE_KIND)
            } else {
                None
            },
        );
        self.after_access(cpu);
        self.finish_timer(cpu);
    }

    /// What the CPU reads at `address` this M-cycle.
    fn load(&mut self, address: u16, cpu: &Sm83) -> u8 {
        let active = self.ppu.dma.active;
        if active && self.machine.cgb_hardware && self.cgb_dma_conflict(address) {
            return self.cgb_dma_read(address);
        }
        if active {
            if (0xFE00..0xFF00).contains(&address) {
                // reads of the whole $FExx page return $FF while the DMA owns OAM (Gambatte
                // oamdma_src*_busypopFE9F/FEFF read $FEA0 and $FEFF as $FF, DMG-08)
                return 0xFF;
            }
            if address < 0xFE00
                && bus_of(address) == self.ppu.dma.bus
                && self.ppu.dma.bus != DmaBus::Internal
                && !self.machine.cgb_hardware
            {
                return self.ppu.dma.byte; // bus conflict: the CPU gets the DMA's byte
            }
        }
        self.peek(address, cpu)
    }

    /// The boot ROM covers $0000-$00FF, and on a CGB $0200-$08FF too.
    fn boot_at(&self, address: u16) -> bool {
        self.boot_mapped && (address < 0x100 || (self.cgb && (0x200..0x900).contains(&address)))
    }

    /// Offset into WRAM; $D000-$DFFF (and its echo) is the SVBK bank in CGB mode.
    fn wram_index(&self, address: u16) -> usize {
        let index = (address & 0x0FFF) as usize;
        if address & 0x1000 != 0 {
            return (self.svbk << 12) | index;
        }
        index
    }

    /// The byte at `address` as the CPU would read it now, with no side effects and no DMA
    /// conflict (debuggers, harnesses).
    pub fn peek(&mut self, address: u16, cpu: &Sm83) -> u8 {
        if address < 0x8000 {
            if self.boot_at(address) {
                return self.boot_rom.as_ref().expect("boot ROM")[address as usize];
            }
            return self.cartridge.read(address, self.t).unwrap_or(0xFF);
        }
        if address < 0xA000 {
            if self.ppu.vram_locked() {
                return 0xFF;
            }
            return self.ppu.vram
                [address as usize - 0x8000 + ((self.ppu.vram_bank as usize) << 13)];
        }
        if address < 0xC000 {
            return self.cartridge.read(address, self.t).unwrap_or(0xFF);
        }
        if address < 0xFE00 {
            return self.wram[self.wram_index(address)];
        }
        if address < 0xFEA0 {
            return if self.ppu.oam_locked() {
                0xFF
            } else {
                self.ppu.oam[address as usize - 0xFE00]
            };
        }
        if address < 0xFF00 {
            if self.ppu.oam_locked() {
                return 0xFF;
            }
            return self.fea0(address);
        }
        if address < 0xFF80 {
            return self.io_read(address, cpu);
        }
        if address < 0xFFFF {
            return self.hram[address as usize - 0xFF80];
        }
        cpu.ie
    }

    /// $FEA0-$FEFF with OAM free: $00 on a DMG; on CGB E the high nibble of the low address byte
    /// twice; before E a RAM area masked per revision (not modelled: reads as that).
    fn fea0(&self, address: u16) -> u8 {
        if self.machine.fea0_read == Fea0Read::HighNibbleTwice {
            let nibble = ((address >> 4) & 0xF) as u8;
            return nibble | (nibble << 4);
        }
        0x00
    }

    fn store(&mut self, address: u16, value: u8, cpu: &mut Sm83) {
        if address < 0x8000 {
            self.cartridge.write(address, value, self.t);
        } else if address < 0xA000 {
            if !self.ppu.vram_write_locked() {
                let i = address as usize - 0x8000 + ((self.ppu.vram_bank as usize) << 13);
                self.ppu.vram[i] = value;
            }
        } else if address < 0xC000 {
            self.cartridge.write(address, value, self.t);
        } else if address < 0xFE00 {
            let i = self.wram_index(address);
            self.wram[i] = value;
        } else if address < 0xFEA0 {
            if !self.ppu.oam_write_locked() && !self.ppu.dma.active {
                self.ppu.oam[address as usize - 0xFE00] = value;
            }
        } else if address < 0xFF00 {
            // unusable
        } else if address < 0xFF80 {
            self.io_write(address, value, cpu);
        } else if address < 0xFFFF {
            self.hram[address as usize - 0xFF80] = value;
        }
        // $FFFF: the core owns IE and has already stored it
    }

    // ----- DMA hooks --------------------------------------------------------------------

    // CGB OAM DMA against the CPU, as SameBoy models it (Core/memory.c is_addr_in_dma_use,
    // GB_read_memory, GB_write_memory; Expat licence). A CGB has three buses: the cartridge
    // ($0000-$7FFF, $A000-$BFFF), VRAM, and WRAM ($C000 up). While a DMA is moving bytes (not in
    // its first cycle), a CPU access below $FE00 collides with it when it is on the bus the DMA
    // reads, and also: anything from $C000 up collides unless the DMA reads VRAM; with a source
    // from $E000 up (invalid on a CGB) everything but VRAM collides.

    /// 0 the cartridge bus, 1 VRAM, 2 WRAM (a CGB's three buses).
    fn cgb_bus(address: u16) -> u8 {
        if address < 0x8000 || (0xA000..0xC000).contains(&address) {
            return 0;
        }
        if address < 0xA000 {
            1
        } else {
            2
        }
    }

    fn cgb_dma_conflict(&self, address: u16) -> bool {
        let dma = &self.ppu.dma;
        if address >= 0xFE00 || dma.index == 0 {
            return false;
        }
        // the address the DMA read this M-cycle
        let last = ((dma.base as u16) << 8) | (dma.index - 1) as u16;
        if address == last {
            return false;
        }
        if address >= 0xC000 {
            return Self::cgb_bus(last) != 1;
        }
        if dma.base >= 0xE0 {
            return Self::cgb_bus(address) != 1;
        }
        Self::cgb_bus(address) == Self::cgb_bus(last)
    }

    fn dma_wram_index(&self, address: u16) -> usize {
        // the WRAM half is the one the DMA's address names (its bit 12), not the CPU's
        if (self.ppu.dma.base >> 4) & 1 != 0 {
            return (self.svbk << 12) | (address & 0xFFF) as usize;
        }
        (address & 0xFFF) as usize
    }

    // What a colliding access does is fitted to Gambatte's oamdma_src*_busypush*/busypop* hardware
    // verdicts (cgb04c, a CPU CGB C), where they differ from SameBoy (which has OAM get 0 for any
    // ROM-source write on CGB C-E and never clears OAM on a read). The revisions after C are not
    // judged: the same rules are used.

    fn cgb_dma_read(&mut self, address: u16) -> u8 {
        let base = self.ppu.dma.base;
        if base >= 0xE0 && Self::cgb_bus(address) == 0 {
            return 0xFF; // an invalid source leaves the cartridge bus undriven
        }
        if address >= 0xC000 && (Self::cgb_bus((base as u16) << 8) != 2 || base >= 0xE0) {
            return self.wram[self.dma_wram_index(address)];
        }
        let i = self.ppu.dma.index - 1;
        let value = self.ppu.oam[i]; // the byte the DMA just moved
        if (0x80..0xA0).contains(&base) {
            // a VRAM source: the byte is taken from OAM (busypop7FFF/9FFF from $8000/$9F00: 00)
            self.ppu.oam[i] = 0;
        }
        value
    }

    fn cgb_dma_write(&mut self, address: u16, value: u8) {
        let base = self.ppu.dma.base;
        if address >= 0xC000 {
            // from a WRAM source the write is lost (srcC000 busypushC001); from any other it
            // reaches WRAM, in the half the DMA's address names (src0000/A000/E000 busypushC001)
            if !(0xC0..0xE0).contains(&base) {
                let i = self.dma_wram_index(address);
                self.wram[i] = value;
            }
            return;
        }
        // below $C000 the write is lost and OAM takes its place: the CPU's byte, or 0 from a VRAM
        // source (busypushA001 from $0000/$A000/$E000: 55; busypush8001 from $8000: 00)
        let i = self.ppu.dma.index - 1;
        self.ppu.oam[i] = if (0x80..0xA0).contains(&base) {
            0
        } else {
            value
        };
    }

    /// A DMA source read, on the bus the source page is on (no locks apply).
    fn dma_source(&mut self, address: u16) -> u8 {
        if self.cgb && self.ppu.dma.base >= 0xE0 {
            return 0xFF; // CGB: sources from $E0 up are invalid (SameBoy GB_dma_run)
        }
        if self.boot_at(address) {
            self.boot_rom.as_ref().expect("boot ROM")[address as usize]
        } else if address < 0x8000 || (0xA000..0xC000).contains(&address) {
            self.cartridge.read(address, self.t).unwrap_or(0xFF)
        } else if address < 0xA000 {
            self.ppu.vram[address as usize - 0x8000 + ((self.ppu.vram_bank as usize) << 13)]
        } else {
            self.wram[self.wram_index(address)]
        }
    }

    // ----- I/O registers ----------------------------------------------------------------

    fn io_read(&mut self, address: u16, cpu: &Sm83) -> u8 {
        let low = address & 0xFF;
        if (0x10..=0x3F).contains(&low) {
            return self.apu.read(address);
        }
        let ppu = &self.ppu;
        match low {
            0x00 => self.joypad.read(),
            0x01 => self.serial.sb,
            0x02 => {
                self.serial.cgb_mode = self.ppu.cgb_mode;
                self.serial.read_sc()
            }
            0x04 => self.timer.read_div(),
            0x05 => self.timer.tima as u8,
            0x06 => self.timer.tma,
            0x07 => 0xF8 | self.timer.tac,
            0x0F => 0xE0 | (cpu.if_ & !self.irq.hide),
            0x40 => ppu.lcdc,
            0x41 => ppu.read_stat(),
            0x42 => ppu.scy,
            0x43 => ppu.scx,
            0x44 => ppu.read_ly(),
            0x45 => ppu.lyc,
            0x46 => ppu.dma.register,
            0x47 => ppu.bgp,
            0x48 => ppu.obp0,
            0x49 => ppu.obp1,
            0x4A => ppu.wy,
            0x4B => ppu.wx,
            _ => {
                if self.cgb {
                    return self.cgb_io_read(low as u8);
                }
                0xFF // unmapped registers read 1 (Mooneye bits/unused_hwio-GS)
            }
        }
    }

    /// The registers only a CGB has (SameBoy `GB_read_memory`, Pan Docs CGB_Registers.md);
    /// outside CGB mode most read $FF.
    fn cgb_io_read(&mut self, low: u8) -> u8 {
        let mode = self.ppu.cgb_mode;
        match low {
            0x4D => {
                if mode {
                    (self.key1 & 1) | 0x7E | if self.double_speed { 0x80 } else { 0 }
                } else {
                    0xFF
                }
            }
            0x4F => 0xFE | self.ppu.vram_bank,
            0x50 => 0xFE | u8::from(!self.boot_mapped),
            0x55 => {
                let active = self.hdma_blocks != 0 && (self.hdma_hblank || self.hdma_due);
                if mode {
                    (if active { 0 } else { 0x80 })
                        | ((self.hdma_blocks.wrapping_sub(1) & 0x7F) as u8)
                } else {
                    0xFF
                }
            }
            0x56 => {
                if mode {
                    (self.rp & 0xC1) | 0x3E
                } else {
                    0xFF
                }
            }
            0x68 | 0x6A => {
                (if low == 0x6A {
                    self.ppu.ocps
                } else {
                    self.ppu.bcps
                }) | 0x40
            }
            0x69 | 0x6B => {
                if !mode && !self.boot_mapped {
                    return 0xFF;
                }
                self.ppu.read_palette(low == 0x6B)
            }
            0x6C => 0xFE | self.opri_register,
            0x70 => {
                if mode {
                    0xF8 | self.svbk_register
                } else {
                    0xFF
                }
            }
            0x72 | 0x73 => self.undocumented[(low - 0x72) as usize],
            0x74 => {
                if mode {
                    self.undocumented[2]
                } else {
                    0xFF
                }
            }
            0x75 => self.undocumented[3] | 0x8F,
            0x76 | 0x77 => self.apu.pcm((low - 0x76) as usize),
            _ => 0xFF,
        }
    }

    fn io_write(&mut self, address: u16, value: u8, cpu: &mut Sm83) {
        let low = address & 0xFF;
        if (0x10..=0x3F).contains(&low) {
            self.apu.frame_clock_high = self.timer.counter & 0x1000 != 0;
            self.apu.write(address, value);
            return;
        }
        match low {
            0x00 => {
                if self.joypad.write(value) {
                    cpu.request_interrupt(4, false, false);
                }
            }
            0x01 => self.serial.write_sb(value),
            0x02 => {
                self.serial.cgb_mode = self.ppu.cgb_mode;
                if SERIAL_SC_CANCEL && self.irq.hide & 8 != 0 {
                    // the transfer has not ended yet: the write stops it before its interrupt
                    cpu.if_ &= !8;
                    self.irq.hide &= !8;
                    self.serial.cancel_finish();
                }
                self.serial.write_sc(value);
                self.take_serial_irq(cpu);
            }
            0x04 => {
                self.write_div();
                self.take_serial_irq(cpu);
            }
            0x05 => self.timer.write_tima(value),
            0x06 => self.timer.write_tma(value),
            0x07 => self.timer.write_tac(value),
            0x0F => {
                // an interrupt raised late in this very cycle is not cleared by the write nor is
                // a serial interrupt still hidden: it has not happened yet (Gambatte flags it at
                // the end of the hiding)
                cpu.if_ = (value & 0x1F) | self.irq.late_bits | (cpu.if_ & self.irq.hide & 8);
            }
            0x40 => {
                let mut ctx = IrqCtx {
                    cpu,
                    b: &mut self.irq,
                };
                self.ppu.write_lcdc(value, &mut ctx);
            }
            0x41 => {
                let mut ctx = IrqCtx {
                    cpu: &mut *cpu,
                    b: &mut self.irq,
                };
                if self.ppu.write_stat(value, &mut ctx) {
                    // cancelled an interrupt raised early
                    cpu.if_ &= !2;
                    self.irq.hide &= !2;
                }
            }
            0x42 => self.ppu.scy = value,
            0x43 => self.ppu.scx = value,
            0x45 => {
                let mut ctx = IrqCtx {
                    cpu: &mut *cpu,
                    b: &mut self.irq,
                };
                if self.ppu.write_lyc(value, &mut ctx) {
                    // cancelled an LY=LYC interrupt raised early
                    cpu.if_ &= !2;
                    self.irq.hide &= !2;
                }
            }
            0x46 => self.ppu.dma.start(value),
            0x47 => self.ppu.write_bgp(value),
            0x48 => self.ppu.obp0 = value,
            0x49 => self.ppu.obp1 = value,
            0x4A => {
                self.ppu.wy = value;
                self.ppu.compare_wy();
            }
            0x4B => self.ppu.wx = value,
            0x50 if value & 1 != 0 && self.boot_mapped => {
                self.boot_mapped = false; // the boot ROM switches itself off; it cannot come back
            }
            _ => {
                if self.cgb {
                    self.cgb_io_write(low as u8, value);
                }
            }
        }
    }

    fn cgb_io_write(&mut self, low: u8, value: u8) {
        let mode = self.ppu.cgb_mode;
        match low {
            0x4C => {
                if self.boot_mapped {
                    self.ppu.cgb_mode = value & 0xC == 0; // the boot ROM tells the PPU which mode to be in
                    self.key0 = value;
                }
            }
            0x4D => {
                if mode {
                    self.key1 = value & 1;
                }
            }
            0x4F => {
                if mode {
                    self.ppu.vram_bank = value & 1;
                }
            }
            0x51..=0x54 => {
                if mode {
                    self.hdma_address(low, value);
                }
            }
            0x55 => {
                if mode {
                    self.hdma_control(value);
                }
            }
            0x56 => self.rp = value,
            0x68 => self.ppu.bcps = value,
            0x6A => self.ppu.ocps = value,
            0x69 | 0x6B => {
                if mode || self.boot_mapped {
                    self.ppu.write_palette(low == 0x6B, value);
                }
            }
            0x6C => {
                if self.boot_mapped || self.key0 & 8 != 0 {
                    self.opri_register = value & 1;
                    self.ppu.opri = value & 1;
                } else if mode {
                    self.opri_register = value & 1;
                }
            }
            0x70 => {
                if mode || self.boot_mapped {
                    self.svbk_register = value & 7;
                    self.svbk = if value & 7 != 0 {
                        (value & 7) as usize
                    } else {
                        1
                    };
                }
            }
            0x72 | 0x73 => self.undocumented[(low - 0x72) as usize] = value,
            0x74 => {
                if mode {
                    self.undocumented[2] = value;
                }
            }
            0x75 => self.undocumented[3] = value & 0x70,
            _ => {}
        }
    }

    fn hdma_address(&mut self, low: u8, value: u8) {
        let value = value as u32;
        if low == 0x51 {
            self.hdma_src = (self.hdma_src & 0x00F0) | (value << 8);
            if self.hdma_src >= 0xE000 {
                self.hdma_src |= 0xF000;
            }
        } else if low == 0x52 {
            self.hdma_src = (self.hdma_src & 0xFF00) | (value & 0xF0);
        } else if low == 0x53 {
            self.hdma_dst = (self.hdma_dst & 0x00F0) | ((value & 0x1F) << 8);
        } else {
            self.hdma_dst = (self.hdma_dst & 0xFF00) | (value & 0xF0);
        }
    }

    /// $FF55: bit 7 chooses HBlank (1) or general (0); bits 6-0 are the length in 16-byte
    /// blocks minus one. Clearing bit 7 while an HBlank transfer runs stops it. An HBlank
    /// transfer armed in mode 0 runs its first block at once, also with the LCD off, where no
    /// further HBlank comes (SameSuite `hdma_lcd_off`, `gdma_addr_mask`).
    fn hdma_control(&mut self, value: u8) {
        self.hdma_blocks = (value & 0x7F) as u32 + 1;
        if value & 0x80 == 0 && self.hdma_hblank {
            self.hdma_hblank = false;
            return;
        }
        self.hdma_hblank = value & 0x80 != 0;
        if !self.hdma_hblank || self.ppu.read_mode == 0 {
            self.hdma_due = true;
        }
    }

    // ----- CGB DMA (HDMA/GDMA) ----------------------------------------------------------

    /// Run the armed transfer's current block with the CPU stopped: 16 bytes at one byte per
    /// 2 dots (SameBoy `GB_hdma_run`: one M-cycle of start-up, 8 M-cycles a block at single
    /// speed, 16 at double). A general-purpose transfer runs every block back to back; an
    /// HBlank one a block per HBlank.
    fn hdma_run(&mut self, cpu: &mut Sm83) {
        let dma_dots = if self.double_speed { 4 } else { 2 }; // units a byte takes: 4 make an M-cycle
        self.hdma_due = false;
        self.hdma_wait(
            if self.double_speed {
                HDMA_START_DS
            } else {
                HDMA_START
            },
            cpu,
        );
        while self.hdma_blocks != 0 {
            for _ in 0..16 {
                let byte = self.hdma_source(self.hdma_src as u16);
                self.hdma_wait(dma_dots, cpu);
                let i = ((self.ppu.vram_bank as usize) << 13) + (self.hdma_dst & 0x1FFF) as usize;
                self.ppu.vram[i] = byte;
                self.hdma_src = (self.hdma_src + 1) & 0xFFFF;
                self.hdma_dst = (self.hdma_dst + 1) & 0xFFFF;
            }
            self.hdma_blocks -= 1;
            if self.hdma_blocks == 0 {
                self.hdma_hblank = false;
            }
            if self.hdma_hblank {
                break;
            }
        }
        if !self.double_speed {
            self.hdma_wait(HDMA_END, cpu);
        }
    }

    /// Advance the board `units` CPU units (4 per M-cycle) with the CPU stopped.
    fn hdma_wait(&mut self, units: i32, cpu: &mut Sm83) {
        self.idle_units += units;
        while self.idle_units >= 4 {
            self.idle_units -= 4;
            self.tick(cpu);
            self.after_access(cpu);
            self.finish_timer(cpu);
        }
    }

    /// HDMA reads cartridge, VRAM-less sources only: ROM, cartridge RAM and WRAM (else $FF).
    fn hdma_source(&mut self, address: u16) -> u8 {
        if address < 0x8000 || (0xA000..0xE000).contains(&address) {
            return self.dma_source(address);
        }
        0xFF
    }
}

impl sm83::Bus for Hw {
    fn read(&mut self, address: u16, cpu: &mut Sm83) -> u8 {
        let value = self.bus_read(address, cpu);
        self.log(0, address, value);
        value
    }

    fn write(&mut self, address: u16, value: u8, cpu: &mut Sm83) {
        self.bus_write(address, value, cpu);
        self.log(1, address, value);
    }

    fn idle(&mut self, address: u16, cpu: &mut Sm83) {
        self.bus_idle(address, cpu);
        self.log(2, address, 0);
    }

    fn stop(&mut self, pending: bool, cpu: &mut Sm83) -> StopResult {
        self.stop_hook(pending, cpu)
    }
}
