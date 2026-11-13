//! The PPU's timing, dot by dot: OAM scan, mode 3's length, LY, the STAT line, VBlank
//! (gameboy-python `ppu.py`).
//!
//! One call to `dot` per PPU dot (4 per M-cycle). Mode 3 is a **pixel stepper**: after the
//! start-up plus `SCX % 8` it moves one pixel per dot and stalls for the window and the objects
//! (Pan Docs Rendering.md "OBJ penalty algorithm"), so STAT, the interrupts and the VRAM/OAM
//! locks see mode 3 end where the hardware does. It does not draw: the picture is drawn by
//! `pixels::Pixels`, a literal fetcher and FIFO pair (after docboy) that runs `DRAW_LAG` dots
//! behind this PPU and matches Mealybug Tearoom's DMG and CGB-D pictures (`step_pixels`).
//!
//! Also modelled: 456 dots per line, 154 lines, LYC, the STAT interrupt **line** (rising edge
//! only), the VBlank interrupt at LY 144, VRAM locked in mode 3 and OAM in modes 2-3 (reads
//! return $FF, writes dropped), nothing locked with the LCD off. The mode edges are fitted to
//! Mooneye ppu/*.
//!
//! Where Rust forced a difference:
//! - the reference's `raise_irq` callback is the board's `_ppu_irq`, which reads and writes board
//!   state and the CPU; here every method that can raise an interrupt takes an
//!   [`IrqCtx`](crate::board::IrqCtx) (the CPU and the board's interrupt state) and calls
//!   [`ppu_irq`](crate::board::ppu_irq) at the same point;
//! - the board's OAM DMA lives here (`dma`), where the drawing machine looks at it;
//! - the timing stepper's background FIFO (`bg`) is a count: its entries (tile tickets) are never
//!   read, only its length, so the tickets are not kept;
//! - the drawing machine is held in an `Option<Box<_>>` so it can read the PPU while it runs;
//! - the Super Game Boy taps (`line_out`, `frame_start`) are not carried over (no SGB host).

use serde::{Deserialize, Serialize};

use crate::board::{ppu_irq, IrqCtx};
use crate::dma::OamDma;
use crate::machines::DOTS_PER_LINE;
use crate::pixels::Pixels;

/// FITTED (Mealybug: all 24 DMG and 20 of 21 CGB-D exact at 5 only): dots the drawing machine
/// runs behind
pub const DRAW_LAG: i32 = 5;

pub const MODE2_END: i32 = 80;
/// FITTED (blargg oam_bug 7): the last row that can be corrupted
pub const OAM_BUG_LAST_ROW: i32 = 19;
/// FITTED (blargg oam_bug): the dot at which the scan reads row 0
pub const OAM_BUG_START: i32 = 0;
/// FITTED: STAT and the CPU's VRAM/OAM locks see the mode 4 dots late (Mooneye
/// intr_2_mode0_timing, intr_2_mode3_timing, intr_2_oam_ok_timing); interrupts do not
pub const MODE_READ_LAG: usize = 4;
/// FITTED: the CPU's dispatch sees the line-start OAM interrupt this early (GBMicrotest)
pub const OAM_IRQ_EARLY: i32 = 4;
/// FITTED (GBMicrotest hblank_int_scx*): pixels before the end it fires
pub const HBLANK_IRQ_EARLY: i32 = 2;
/// FITTED (GBMicrotest ppu_sprite0_scx*): STAT shows mode 0 this many dots after
pub const MODE0_READ_LAG: i32 = 1;
/// FITTED: the same in double speed (Gambatte cgb04c *_ds m3stat, vramw_m3end, gdma/hdma_cycles)
pub const MODE0_READ_LAG_DS: i32 = 0;
/// FITTED (Gambatte cgbpal_m3end_ds): dots the CGB palettes stay locked after STAT leaves mode 3
/// in double speed (SameBoy frees them after STAT and VRAM too)
pub const PAL_HOLD_DS: i32 = 2;
/// in double speed (after an LCD-on in it, or a switch to it) LY shows the next line this many
/// dots before it starts: LCD_ON_DS moves the lines, not the LY reads (AGE ly-cgbE, Gambatte
/// enable_display frame0/1_ly_count_ds_2)
pub const LY_LEAD_DS: i32 = 2;
/// CGB: a STAT write clearing bit 4 in the M-cycle the mode 1 source rises cancels that
/// interrupt (Gambatte m1irq_disable_1, _ds_1; the write lands in the dot the source rises),
/// like the OAM one
pub const M1_CANCEL_CGB: i32 = 1;
/// The HBlank interrupt reaches IF HBLANK_IRQ_EARLY dots before its source rises; a STAT write
/// clearing bit 3 in that M-cycle lands first and cancels it (Gambatte m0enable/disable_*): on
/// DMG in every phase, on CGB only for one raised in the cycle from this dot on (scx 1, 2, 5)
pub const M0_CANCEL_DMG: i32 = 1;
pub const M0_CANCEL_CGB: i32 = 1;
/// A STAT write lands before the line's fall earlier in its M-cycle and sees the line still high
/// (Gambatte m1/lyc143_late_m*enable_lycdisable_2, m2enable/late_enable_after_lycint_*): on DMG
/// in every phase; on CGB in double speed from the cycle's second dot, in single speed from its
/// third and not for the LY=LYC source, which falls before the write there
pub const STAT_FALL_DMG: i32 = 1;
pub const STAT_FALL_CGB: i32 = 2;
pub const STAT_FALL_CGB_DS: i32 = 1;
/// DMG: an LYC write lands before an LY=LYC or mode 1 fall of the line earlier in its M-cycle
/// (not an HBlank one: Gambatte miscmstatirq/lycwirq_trigger_m0_late_*)
pub const LYC_FALL_DMG: i32 = 1;
/// IF shows line 0's OAM interrupt this many dots after its raise
pub const OAM_LY0_IF_DMG: i32 = 4;
pub const OAM_LY0_IF_CGB: i32 = 4;
/// an LYC write ending the match cancels an LY=LYC raise earlier in its M-cycle
pub const LYC_CANCEL_DMG: i32 = 1;
/// the hidden rise (m0_hidden) of the OAM pulse
pub const OAM_HIDDEN_DMG: i32 = 1;
/// ... and on CGB in double speed
pub const OAM_HIDDEN_DS: i32 = 1;
/// Line 144: the mode 1 STAT source rises this many dots after the LY=LYC flag falls (unless
/// mode 0 is selected and holds the line), so LYC=143 then mode 1 is a new edge. FITTED
/// (Gambatte m1 lycint143_*)
pub const M1_SRC_DOT: i32 = 1;
/// CGB, lines 0-142: the HBlank STAT source ends this many dots before the line does unless the
/// mode 2 source is selected (it takes over at once), so an LY=LYC rise at the next line's start
/// is a new edge. FITTED (Gambatte lycEnable ff41_disable_2/_3, miscmstatirq
/// lycwirq_trigger_m0_late_*)
pub const M0_SRC_END: i32 = 1;
/// FITTED (Mooneye hblank_ly_scx_timing with the early HBlank interrupt)
pub const LY_LEAD: i32 = 0;
/// DMG: WX=166 starts the window at the next line's start (SameBoy display.c)
pub const WX166_CARRY: i32 = 1;
/// WX=w below 7 is compared 7 - w dots before the first shown pixel's compare
pub const WX_EDGE_LEAD: i32 = 7;
/// FITTED (Gambatte cgb04c window 143 -> 148 of 193 at 0 or 1): the window comparator sees WX
/// this many dots late
pub const WX_LAG: i32 = 1;
/// ... and LCDC.5/.0 this many
pub const WIN_LAG: i32 = 0;
/// CGB: LCDC.5 cleared just after the window starts puts the background back (DMG keeps the
/// window). FITTED to the hardware verdicts (Gambatte cgb04c window/late_disable_*, +16 -2); the
/// idea was first read in Gambatte's source (gameboy-python docs/validation.md), the dots are
/// this board's fit: the clear undoes the window for this many dots after it starts
pub const WIN_UNDO_DOTS: i32 = 6;
/// ... and gives back this many of the dots the window's start took
pub const WIN_UNDO_REFUND: i32 = 3;
/// FITTED (GBMicrotest win*; Gambatte window scx2 CGB +4, DMG +5 over 4): fetcher dots the
/// window restart gets for free
pub const WINDOW_HEAD: i32 = 3;
/// ... in double speed (3 loses the m2int_wx*_m3stat_ds_2 halves)
pub const WINDOW_HEAD_DS: i32 = 4;
/// ... when the window starts at the left edge (WX < 7)
pub const WINDOW_EDGE_HEAD: i32 = 6;
/// FITTED (GBMicrotest stat_write_glitch_l143_*): IF shows VBlank this much later
pub const VBLANK_IF_LATE: i32 = 4;
/// FITTED (GBMicrotest vblank_int_if_*): the STAT VBlank source comes this late
pub const MODE1_IRQ_DOT: i32 = 4;
/// Single speed: the PPU runs this many dots of each M-cycle after the CPU's access, before the
/// interrupt dispatch looks, so an interrupt raised early in a 4-dot window can be dispatched a
/// cycle sooner. Aligned ROMs cannot tell; after a speed switch's odd LCD skew (board
/// SWITCH_LCD_*) the lcdoffset1 ROMs can (Gambatte cgb04c lcdoffset1 +13, speedchange +25). The
/// HBlank and mode 2 interrupts keep their old dispatch cycle (irq_defer): their dots are pinned
/// by the SCX ROMs.
pub const IRQ_LEAD: i32 = 1;
/// the LY=LYC interrupt comes this many dots before the flag shows (CGB)
pub const LYC_IRQ_EARLY: i32 = 4;
/// DMG: one dot less, so an HBlank source ending at the line's start leaves the line low for a
/// dot (Gambatte miscmstatirq/lycwirq_trigger_m0_late_*, lcdirq_precedence/*lycirq_ly44_*;
/// GBMicrotest int_lyc_* unchanged)
pub const LYC_IRQ_EARLY_DMG: i32 = 3;
/// and lasts this many dots
pub const OAM_IRQ_WIDTH: i32 = 4;
/// CGB double speed, line 0 excepted (Gambatte m2enable/late_enable_ds_*, m1)
pub const OAM_IRQ_WIDTH_DS: i32 = 2;
/// FITTED: after LCD on, line 0 is 'late by 2 T' (Mooneye lcdon_timing-GS)
pub const LINE0_START_DOT: i32 = 4;
/// Double speed: line 0 starts this many dots further on at LCD on. Gambatte enable_display
/// frame0/frame1 m0irq/m2irq/m2stat _ds_1, ly0_m0irq_*_ds_1, ly0_late_cgbp*_ds_1 and the
/// speedchange2 lcdoff ROMs pass at -2 (CGB +20 -2 with LY_LEAD_DS; ly0_oambusy_read_ds_2 and
/// ly0_late_vramr_ds_2, a lock read against the LCD-on moment, are lost); odd values break
/// scx_during_m3
pub const LCD_ON_DS: i32 = -2;
/// FITTED (Mealybug line_0_fix): on line 0 the pixel pipeline runs this early
pub const LINE0_LEAD: i32 = 0;
/// FITTED (Gambatte scx_during_m3, Mealybug m3_scx_low_3_bits): dots late the fine-scroll drop
/// sees SCX
pub const SKIP_LAG: usize = 7;
/// FITTED (Gambatte window/arg/late_wy_*_2, CGB 207 -> 217 of 273 in window): on CGB a written
/// WY is compared this many dots later (SameBoy runs the compare at the next 4-dot boundary, on
/// CGB one phase later than on DMG); 4-5 gain, 6 loses the _1 halves, DMG keeps the immediate
/// compare
pub const WY_WAIT_CGB: i32 = 5;
/// CGB single speed: line 0's WY==LY compare comes this many dots into the line, so a WY write
/// in the line's first M-cycle still counts (Gambatte window late_wy_1, late_wy_lcdoffset1_1:
/// +3; in double speed no value moves anything)
pub const WY0_WAIT_CGB: i32 = 5;
/// WY==LY only counts while LCDC.5 is set (SameBoy wy_check), and setting LCDC.5 runs a new
/// compare this many dots later on CGB (4; 2 in double speed; DMG at once): Gambatte window CGB
/// +6 -1, DMG +2
pub const WIN_EN_WAIT: i32 = 4;
pub const WIN_EN_WAIT_DS: i32 = 2;
/// FITTED (Gambatte sprites/space 9pos8_wx*_m3stat_ds_1, 1pos8_8pos9_wx08: 346 -> 355 of 380):
/// in double speed a window started after an object fetch stalls the pipeline this many dots (3
/// loses them, 5 loses a _2 half; SameBoy's fetch log puts its mode 3 about 4 dots past ours)
pub const WIN_OBJ_STALL_DS: i32 = 4;
/// SameBoy (sm83_cpu.c cgb_conflict_map, LYC = GB_CONFLICT_WRITE_CPU against DMG's READ_OLD): on
/// CGB at single speed a LYC write lands this many dots later (CGB answers a DMG write 4 dots
/// earlier: Gambatte lycEnable/m0enable/m2enable +8 -4, three of the four lcdoffset1 halves)
pub const LYC_WAIT_CGB: i32 = 4;
/// CGB: a halted CPU sees the HBlank and LY=LYC interrupts this many dots later than a DMG does
/// (SameBoy GB_cpu_run samples IF before a halted CGB's 4 T, a DMG's halfway; Gambatte halt +8,
/// dma/hdma_start +7). Not the line-start OAM one, already raised early (Mooneye intr_2_*).
pub const HALT_LATE_CGB: i32 = 2;
/// FITTED (Gambatte halt/m0i*_m0stat_scx2/3_ds_2 +4, ly0 lycint152_lyc0flag_ds_4, oam_access
/// prewrite_ds_lcdoffset1_2; lycint_lycflag/lycint_m0stat ds trade 2 for 2; 2 scores one fewer):
/// in double speed STAT shows a line's mode 2 this many dots into the line, not MODE_READ_LAG
pub const M2_READ_LAG_DS: i32 = 0;
/// Single speed: a line's mode 2 shows in STAT (and locks OAM) this many dots into the line, and
/// on CGB VRAM locks this many dots into mode 3. Aligned ROMs read only every 4th dot, so 1-4
/// look the same to them; after a speed switch's skew the lcdoffset1 ROMs see the difference
/// (oam_access prewrite_lcdoffset1_2, vram_m3 preread/prewrite_lcdoffset1_2 pass at 3, 2 or 1; 1
/// loses one)
pub const M2_READ_LAG_SS: i32 = 3;
pub const VRAM_LOCK_SS: i32 = 3;
/// SameBoy (mode_for_interrupt = 2 at LINE_LENGTH - 2; no DMG write glitch): a CGB STAT write
/// clearing bit 5 before this many dots from a line's end cancels the OAM interrupt raised early
/// for the dispatch (Gambatte m2enable/disable_*)
pub const OAM_RISE_CGB: i32 = 2;
/// ... and on line 0, one written before this dot of the line
pub const OAM_RISE_LY0_CGB: i32 = 2;
/// FITTED (Gambatte scx_during_m3_ds_* row 0): in double speed line 0's OAM interrupt also comes
/// early, by one M-cycle (SameBoy's line-0 writes land one M-cycle after line 1's, ours two)
pub const EARLY_LINE0_DS: i32 = 2;
/// dots LCDC.2 arrives late at an object fetch
pub const OBJ_SIZE_LAG: i32 = 0;
/// which dot (1 or 2) of its step each fetch reads
pub const MAP_READ_DOT: i32 = 2;
pub const LOW_READ_DOT: i32 = 2;
pub const HIGH_READ_DOT: i32 = 2;
pub const OBJ_FETCH_DOTS: i32 = 6;
/// FITTED to Mooneye ppu/intr_2_mode0_timing_sprites
pub const FIRST_OBJECT_DISCOUNT: i32 = 3;
/// FITTED (Gambatte cgb04c sprites/space in double speed, 261 -> 341 of 380); AGE
/// stat-mode-sprites (single speed) needs 3
pub const FIRST_OBJECT_DISCOUNT_DS: i32 = 2;
pub const VBLANK_LINE: i32 = 144;
pub const LAST_LINE: i32 = 153;
pub const SCREEN_W: usize = 160;
pub const SCREEN_H: usize = 144;

/// An object on the line: (x, OAM index, tile, flags, y).
pub type LineObj = (i32, i32, i32, i32, i32);

/// The background's state when the window started (CGB), for `undo_window`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WinUndo {
    pub bg: usize,
    pub f_step: i32,
    pub f_dot: i32,
    pub fetch_col: i32,
    pub ticket_col: i32,
    pub t1_x: i32,
    pub t1_obj: bool,
    pub dummy: bool,
    pub free_skip: i32,
    pub end: i32,
    pub x: i32,
}

/// A history of a register's value over the last 16 dots: `hist[-k]` of the reference is
/// `self.0[16 - k]`.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Hist(pub [u8; 16]);

impl Hist {
    fn push(&mut self, value: u8) {
        self.0.copy_within(1.., 0);
        self.0[15] = value;
    }

    /// The reference's `hist[-k]` (k >= 1).
    #[inline]
    pub fn ago(&self, k: usize) -> u8 {
        self.0[16 - k]
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Ppu {
    /// CGB hardware: two VRAM banks and palette RAM; `cgb_mode` is false when the boot ROM chose
    /// the DMG-compatible mode (KEY0), where BGP/OBPn still pick the shades
    pub cgb: bool,
    /// dots LY reads 153 for at the start of line 153: 4 on a DMG, 8 on a CGB E (AGE ly-cgbE)
    pub ly153_dots: i32,
    /// the CPU runs at twice the PPU's rate (set by the board)
    pub double_speed: bool,
    pub cgb_mode: bool,
    /// $FF6C bit 0: 1 = objects ordered by X (DMG), 0 = by OAM index
    pub opri: u8,
    /// the bank the CPU sees ($FF4F)
    pub vram_bank: u8,
    pub bg_pal: Vec<u8>,
    pub obj_pal: Vec<u8>,
    pub bcps: u8,
    pub ocps: u8,
    pub stat_write_glitch: bool,
    /// the STAT interrupt line is one OR: a source that rises while another holds it raises
    /// nothing (every model but DMG 0, Mooneye ppu/stat_irq_blocking)
    pub stat_blocking: bool,
    pub sources: [bool; 4],
    /// mode 3 is within HBLANK_IRQ_EARLY pixels of its end
    pub hblank_early: bool,
    pub mode0_dots: i32,
    pub glitching: bool,
    pub early_lyc153: bool,
    /// CGB D/E line 153 comparator (machines.rs; the board sets it)
    pub lyc153_late: bool,
    pub early_fired: bool,
    /// the HBlank interrupt reached IF early in this M-cycle (board)
    pub m0_early: bool,
    /// the HBlank source rose in this M-cycle under a line held high (2: in a lead dot; 3: in
    /// the last cycle's lead, which a STAT write still beats but an LYC write does not)
    pub m0_hidden: i32,
    /// line 0's OAM pulse rose (selected or not) in this M-cycle under a line held high (board)
    pub oam_hidden: bool,
    /// the OAM pulse, whether STAT selects it or not
    pub oam_raw: bool,
    /// DMG: the LY=LYC source raised the interrupt in this M-cycle (board)
    pub lyc_rose: bool,
    /// the STAT line fell in this M-cycle before the CPU's access (board)
    pub stat_fell: i32,
    /// CGB: the mode 1 STAT interrupt was just raised; a write cancels it
    pub m1_early: bool,
    /// ds: the LY=LYC interrupt just came early; a write cancels it
    pub lyc_early: bool,
    /// dots before its real time the interrupt just raised came
    pub irq_early_by: i32,
    /// the interrupt just raised reaches the dispatch a cycle later
    pub irq_defer: bool,
    /// dots later a halted CPU sees the STAT interrupt just raised
    pub irq_halt_delay: i32,
    /// the dot within the M-cycle being ticked (0-3)
    pub sub: i32,
    /// dots LY shows the next line early, in double speed (LY_LEAD_DS)
    pub ly_lead: i32,
    /// dots of this M-cycle left to run after the access (IRQ_LEAD)
    pub owed: i32,
    pub vram: Vec<u8>,
    pub oam: Vec<u8>,
    pub lcdc: u8,
    pub wx_hist: Hist,
    pub scy_hist: Hist,
    pub scx_hist: Hist,
    /// LCDC as of each of the last dots (for the lagged bits)
    pub lcdc_hist: Hist,
    /// STAT bits 6-3 as written
    pub stat_select: u8,
    pub scy: u8,
    pub scx: u8,
    pub ly: i32,
    pub lyc: u8,
    pub bgp: u8,
    pub obp0: u8,
    pub obp1: u8,
    pub wy: u8,
    pub wx: u8,
    pub dot: i32,
    pub mode: u8,
    pub lyc_flag: bool,
    pub stat_line: bool,
    /// VBlanks since power-on
    pub frames: u64,
    /// shades 0-3 (DMG); on CGB hardware RGB555 words: the last completed picture
    pub frame: Vec<u16>,
    /// (x, index, tile, flags, y), by x then index
    pub line_objs: Vec<LineObj>,
    pub skip_scan: bool,
    /// next pixel to emit
    pub x: i32,
    pub next_obj: usize,
    pub window_active: bool,
    /// CGB: the background's state when the window started
    pub win_undo: Option<WinUndo>,
    /// pixel steps owed back after an undone window start
    pub win_refund: i32,
    /// the window's own line counter
    pub window_line: i32,
    /// window pixels drawn on this line
    pub window_used: bool,
    /// the window started mid-line (STAT shows mode 0 later)
    pub win_mid: bool,
    /// DMG: the line starts inside the window (WX=166 at the last end)
    pub win_carry: bool,
    /// WY == LY seen this frame
    pub wy_hit: bool,
    /// dots until a written WY is compared (CGB)
    pub wy_wait: i32,
    /// dots until a written LYC lands, and its value (CGB)
    pub lyc_wait: i32,
    pub lyc_next: u8,
    /// dots the CGB palettes stay locked after read_mode leaves 3
    pub pal_hold: i32,
    /// the mode as the CPU sees it: `mode` from MODE_READ_LAG dots ago
    pub read_mode: u8,
    pub recent: [u8; MODE_READ_LAG],
    /// mode 3 is running
    pub drawing: bool,
    /// the first line after LCD on: starts at dot 4
    pub line0: bool,
    pub mode3_start: i32,
    pub mode0_dot: i32,
    pub m3: i32,
    pub skip: i32,
    pub fetch_col: i32,
    pub dropped: i32,
    pub free_skip: i32,
    pub win_stall: i32,
    /// the background FIFO's length (its entries are never read)
    pub bg: usize,
    /// (colour, flags, OAM index) for the next pixels
    pub obj_fifo: Vec<(i32, i32, i32)>,
    /// 0 tile, 1 data low, 2 data high, 3 waiting to push
    pub f_step: i32,
    pub f_dot: i32,
    pub ticket_col: i32,
    pub t1_x: i32,
    pub t1_obj: bool,
    pub obj_lo: u8,
    pub obj_hi: u8,
    /// the first fetch is thrown away (Pan Docs Rendering.md, "first12")
    pub dummy: bool,
    /// an object has been fetched on this line
    pub penalised: bool,
    pub obj_wait_left: i32,
    pub obj_tiles_waited: Vec<(bool, Option<i32>)>,
    pub obj_low_at: i32,
    pub obj_high_at: i32,
    /// dots left of the object fetch
    pub obj_dots: i32,
    pub obj_entry: Option<LineObj>,
    pub win_start_x: i32,
    /// the board's OAM DMA, which the drawing machine looks at
    pub dma: OamDma,
    /// the picture
    pub pixels: Option<Box<Pixels>>,
}

impl Ppu {
    /// This PPU times the mode, the STAT line and the interrupts; `pixels::Pixels` draws.
    pub fn new(stat_write_glitch: bool, stat_blocking: bool, cgb: bool) -> Ppu {
        let mut ppu = Ppu {
            cgb,
            ly153_dots: if cgb { 8 } else { 4 },
            double_speed: false,
            cgb_mode: false,
            opri: if cgb { 0 } else { 1 },
            vram_bank: 0,
            bg_pal: vec![0; 64],
            obj_pal: vec![0; 64],
            bcps: 0,
            ocps: 0,
            stat_write_glitch,
            stat_blocking,
            sources: [false; 4],
            hblank_early: false,
            mode0_dots: 0,
            glitching: false,
            early_lyc153: false,
            lyc153_late: false,
            early_fired: false,
            m0_early: false,
            m0_hidden: 0,
            oam_hidden: false,
            oam_raw: false,
            lyc_rose: false,
            stat_fell: 0,
            m1_early: false,
            lyc_early: false,
            irq_early_by: 0,
            irq_defer: false,
            irq_halt_delay: 0,
            sub: 0,
            ly_lead: 0,
            owed: 0,
            vram: vec![0; if cgb { 0x4000 } else { 0x2000 }],
            oam: vec![0; 0xA0],
            lcdc: 0,
            wx_hist: Hist::default(),
            scy_hist: Hist::default(),
            scx_hist: Hist::default(),
            lcdc_hist: Hist::default(),
            stat_select: 0,
            scy: 0,
            scx: 0,
            ly: 0,
            lyc: 0,
            bgp: 0,
            // power-on: GBMicrotest poweron_obp*_000 read $FF
            obp0: 0xFF,
            obp1: 0xFF,
            wy: 0,
            wx: 0,
            dot: 0,
            mode: 0,
            lyc_flag: false,
            stat_line: false,
            frames: 0,
            frame: Vec::new(),
            line_objs: Vec::new(),
            skip_scan: false,
            x: 0,
            next_obj: 0,
            window_active: false,
            win_undo: None,
            win_refund: 0,
            window_line: 0,
            window_used: false,
            win_mid: false,
            win_carry: false,
            wy_hit: false,
            wy_wait: 0,
            lyc_wait: 0,
            lyc_next: 0,
            pal_hold: 0,
            read_mode: 0,
            recent: [0; MODE_READ_LAG],
            drawing: false,
            line0: false,
            mode3_start: MODE2_END,
            mode0_dot: 0,
            m3: 0,
            skip: 0,
            fetch_col: 0,
            dropped: 0,
            free_skip: 0,
            win_stall: 0,
            bg: 0,
            obj_fifo: Vec::new(),
            f_step: 0,
            f_dot: 0,
            ticket_col: 0,
            t1_x: 0,
            t1_obj: false,
            obj_lo: 0,
            obj_hi: 0,
            dummy: false,
            penalised: false,
            obj_wait_left: 0,
            obj_tiles_waited: Vec::new(),
            obj_low_at: 0,
            obj_high_at: 0,
            obj_dots: 0,
            obj_entry: None,
            win_start_x: -1,
            dma: OamDma::default(),
            pixels: None,
        };
        ppu.frame = ppu.blank();
        ppu.pixels = Some(Box::new(Pixels::new(cgb)));
        ppu
    }

    pub fn px(&self) -> &Pixels {
        self.pixels.as_ref().expect("pixels")
    }

    fn px_mut(&mut self) -> &mut Pixels {
        self.pixels.as_mut().expect("pixels")
    }

    // ----- state ------------------------------------------------------------------------

    /// A white screen: shade 0, or RGB555 $7FFF on CGB hardware.
    pub fn blank(&self) -> Vec<u16> {
        vec![if self.cgb { 0x7FFF } else { 0 }; SCREEN_W * SCREEN_H]
    }

    #[inline]
    pub fn lcd_on(&self) -> bool {
        self.lcdc & 0x80 != 0
    }

    /// Writes lock 4 dots after reads do (Mooneye lcdon_write_timing-GS).
    pub fn vram_write_locked(&self) -> bool {
        self.lcd_on() && (self.read_mode == 3 || self.vram_lock_early())
    }

    /// CGB single speed: VRAM locks VRAM_LOCK_SS dots into mode 3, before STAT shows it.
    fn vram_lock_early(&self) -> bool {
        self.cgb
            && !self.double_speed
            && self.mode == 3
            && !self.line0
            && MODE2_END + VRAM_LOCK_SS <= self.dot
            && self.dot < MODE2_END + MODE_READ_LAG as i32
    }

    /// Like the read lock, but free at dot 0 and in the dot where mode 2 turns into 3.
    pub fn oam_write_locked(&self) -> bool {
        self.lcd_on()
            && (self.read_mode == 2 || self.read_mode == 3)
            && !(self.mode == 3 && self.read_mode == 2)
    }

    /// locks at once (CGB single speed: with STAT, 4 dots late), frees late
    pub fn vram_locked(&self) -> bool {
        self.lcd_on()
            && ((self.mode == 3 && !self.line0 && !(self.cgb && !self.double_speed))
                || self.read_mode == 3
                || self.vram_lock_early())
    }

    pub fn oam_locked(&self) -> bool {
        self.lcd_on()
            && (((self.mode == 2 || self.mode == 3) && !self.line0)
                || self.read_mode == 2
                || self.read_mode == 3)
    }

    // ----- time -------------------------------------------------------------------------

    /// Advance `dots` PPU dots (one M-cycle): all but the lead before the CPU's access, the
    /// lead after it (`after_access`).
    pub fn tick(&mut self, dots: i32, ctx: &mut IrqCtx) {
        self.owed = if self.double_speed {
            0
        } else {
            dots.min(IRQ_LEAD)
        };
        if self.lcdc & 0x80 == 0 {
            return;
        }
        for sub in 0..dots - self.owed {
            self.sub = sub;
            self.dot_once(ctx);
        }
    }

    /// Advance `dots` dots outside the M-cycle split (a speed switch's LCD skew).
    pub fn run_dots(&mut self, dots: i32, ctx: &mut IrqCtx) {
        if self.lcdc & 0x80 != 0 {
            for _ in 0..dots {
                self.dot_once(ctx);
            }
        }
    }

    fn dot_once(&mut self, ctx: &mut IrqCtx) {
        self.lcdc_hist.push(self.lcdc);
        self.wx_hist.push(self.wx);
        self.scy_hist.push(self.scy);
        self.scx_hist.push(self.scx);
        self.dot += 1;
        if self.dot >= DOTS_PER_LINE {
            self.next_line(ctx);
        }
        if self.wy_wait != 0 {
            self.wy_wait -= 1;
            if self.wy_wait == 0 {
                self.compare_wy_now();
            }
        }
        if self.lyc_wait != 0 {
            self.lyc_wait -= 1;
            if self.lyc_wait == 0 {
                let value = self.lyc_next;
                self.write_lyc_now(value, ctx);
            }
        }
        if self.ly < VBLANK_LINE {
            let dot = self.dot;
            if (0..MODE2_END).contains(&dot) && !self.skip_scan && dot & 1 == 0 {
                self.scan(dot >> 1);
            }
            if dot == self.mode3_start {
                self.start_mode3();
            } else if self.drawing {
                self.step_pixel();
                while self.win_refund != 0 && self.drawing {
                    self.win_refund -= 1;
                    self.step_pixel();
                }
            }
            if dot == MODE2_END && self.drawing {
                self.mode = 3;
            }
            if self.mode0_dot != 0 && dot >= self.mode0_dot {
                self.mode = 0;
                self.mode0_dot = 0;
            }
        }
        self.step_pixels();
        let mut lagged = self.recent[0];
        self.recent.copy_within(1.., 0);
        self.recent[MODE_READ_LAG - 1] = self.mode;
        self.mode0_dots = if self.mode == 0 {
            self.mode0_dots + 1
        } else {
            0
        };
        let lag = if !(self.penalised || self.win_mid) {
            if self.double_speed {
                MODE0_READ_LAG_DS
            } else {
                MODE0_READ_LAG
            }
        } else {
            MODE_READ_LAG as i32
        };
        if lagged == 3 && self.mode == 0 && self.mode0_dots > lag {
            lagged = 0; // the end of mode 3 shows up in STAT sooner than the start of it does
        }
        if self.double_speed
            && (lagged == 0 || lagged == 1)
            && self.mode == 2
            && self.dot >= M2_READ_LAG_DS
        {
            // in double speed a line's mode 2 shows in STAT without the lag, line 0's too: no
            // mode 0 dots after VBlank (Bust-A-Move Millennium's title polls STAT for mode 0 to
            // start its palette stream; catching line 0's first dots started it a line early
            // and tore the picture, which SameBoy never shows; Gambatte cgb04c unchanged)
            lagged = 2;
        }
        if !self.double_speed && lagged == 0 && self.mode == 2 && self.dot >= M2_READ_LAG_SS {
            lagged = 2;
        }
        if lagged == 1 && self.mode == 2 {
            lagged = 0; // the first dots of line 0 read as mode 0, not VBlank's 1
        }
        if self.pal_hold != 0 {
            self.pal_hold -= 1;
        }
        if self.read_mode == 3 && lagged != 3 {
            self.pal_hold = if self.double_speed { PAL_HOLD_DS } else { 0 };
        }
        self.read_mode = lagged;
        self.update_stat_line(ctx);
    }

    /// One dot of the drawing machine, DRAW_LAG dots behind this PPU.
    fn step_pixels(&mut self) {
        let mut ly = self.ly;
        let mut dot = self.dot - DRAW_LAG;
        if dot < 0 {
            if self.line0 {
                return; // the LCD was just turned on: the drawing machine has not started yet
            }
            ly = if ly == 0 { LAST_LINE } else { ly - 1 };
            dot += DOTS_PER_LINE;
        }
        let mut pixels = self.pixels.take().expect("pixels");
        if dot == 0 && !self.line0 {
            pixels.line_done();
        }
        pixels.tick(self, ly, dot);
        self.pixels = Some(pixels);
    }

    fn next_line(&mut self, ctx: &mut IrqCtx) {
        self.dot = 0;
        self.hblank_early = false;
        self.line0 = false;
        self.mode3_start = MODE2_END;
        self.skip_scan = false;
        if self.window_used {
            self.window_line = (self.window_line + 1) & 255;
        }
        self.window_used = false;
        self.ly = if self.ly == LAST_LINE { 0 } else { self.ly + 1 };
        if self.ly == 0 {
            self.window_line = 0;
            self.wy_hit = false;
            self.mode3_start = MODE2_END - LINE0_LEAD;
        }
        if self.ly == VBLANK_LINE {
            self.frames += 1;
            self.frame = self.px().framebuffer.clone();
            self.mode = 1;
            self.irq_early_by = VBLANK_IF_LATE;
            self.irq_halt_delay = 0;
            ppu_irq(self, ctx, 0);
            self.irq_early_by = 0;
        } else if self.ly < VBLANK_LINE {
            self.mode = 2;
            self.line_objs.clear();
            self.window_active = false;
            if self.ly == 0 && self.cgb && !self.double_speed && WY0_WAIT_CGB != 0 {
                self.wy_wait = WY0_WAIT_CGB;
            } else if self.ly == self.wy as i32 && self.lcdc & 0x20 != 0 {
                self.wy_hit = true;
            }
        }
    }

    // ----- the OAM corruption bug (DMG): Pan Docs OAM_Corruption_Bug.md -------------------

    /// The OAM row the PPU is reading this M-cycle, or None outside the scan.
    pub fn oam_row(&self) -> Option<i32> {
        if !self.lcd_on() || self.ly >= VBLANK_LINE || self.line0 {
            return None;
        }
        let row = (self.dot - OAM_BUG_START).div_euclid(4);
        if (0..=OAM_BUG_LAST_ROW).contains(&row) {
            Some(row)
        } else {
            None
        }
    }

    /// An address in $FE00-$FEFF on the bus during the scan: `kind` is read, write or readinc
    /// (a read in the cycle the IDU steps the same register).
    pub fn corrupt_oam(&mut self, kind: OamBugKind) {
        let Some(row) = self.oam_row() else {
            return;
        };
        let row = row as usize;
        let oam = &mut self.oam;
        let word = |oam: &Vec<u8>, r: usize, w: usize| -> u32 {
            oam[r * 8 + w * 2] as u32 | ((oam[r * 8 + w * 2 + 1] as u32) << 8)
        };
        let put = |oam: &mut Vec<u8>, r: usize, w: usize, value: u32| {
            oam[r * 8 + w * 2] = (value & 0xFF) as u8;
            oam[r * 8 + w * 2 + 1] = ((value >> 8) & 0xFF) as u8;
        };
        let copy_row = |oam: &mut Vec<u8>, src: usize, dst: usize| {
            oam.copy_within(src * 8..src * 8 + 8, dst * 8);
        };
        if kind == OamBugKind::ReadInc && (4..19).contains(&row) {
            let a = word(oam, row - 2, 0);
            let b = word(oam, row - 1, 0);
            let c = word(oam, row, 0);
            let d = word(oam, row - 1, 2);
            put(oam, row - 1, 0, (b & (a | c | d)) | (a & c & d));
            copy_row(oam, row - 1, row);
            copy_row(oam, row - 1, row - 2);
        }
        if row == 0 {
            return;
        }
        let a = word(oam, row, 0);
        let b = word(oam, row - 1, 0);
        let c = word(oam, row - 1, 2);
        if kind == OamBugKind::Write {
            put(oam, row, 0, ((a ^ c) & (b ^ c)) ^ c);
        } else {
            put(oam, row, 0, b | (a & c));
        }
        oam.copy_within((row - 1) * 8 + 2..(row - 1) * 8 + 8, row * 8 + 2);
    }

    // ----- OAM scan ---------------------------------------------------------------------

    fn scan(&mut self, entry: i32) {
        if entry >= 40 || self.line_objs.len() >= 10 {
            return;
        }
        let base = (entry * 4) as usize;
        let y = self.oam[base] as i32 - 16;
        let height = if self.lcdc & 4 != 0 { 16 } else { 8 };
        if y <= self.ly && self.ly < y + height {
            self.line_objs.push((
                self.oam[base + 1] as i32,
                entry,
                self.oam[base + 2] as i32,
                self.oam[base + 3] as i32,
                y,
            ));
            self.line_objs.sort_by_key(|o| (o.0, o.1));
        }
    }

    // ----- mode 3 -----------------------------------------------------------------------

    fn start_mode3(&mut self) {
        if self.dot >= MODE2_END {
            self.mode = 3;
        }
        self.drawing = true;
        self.x = 0;
        self.m3 = 0;
        self.win_stall = 0;
        // the fine scroll: pixels are dropped until their count matches SCX's low bits (SameBoy)
        self.skip = 1;
        self.dropped = 0;
        self.next_obj = 0;
        self.window_active = self.win_carry;
        self.win_mid = false;
        if self.win_carry {
            self.window_used = true;
        }
        self.win_undo = None;
        self.free_skip = 0;
        self.bg = 0; // the background FIFO
        self.obj_fifo.clear(); // (colour, flags, OAM index) for the next pixels
        self.f_step = 0; // 0 tile, 1 data low, 2 data high, 3 waiting to push
        self.f_dot = 0;
        self.t1_x = 0;
        self.t1_obj = false;
        self.fetch_col = 0;
        self.dummy = true; // the first fetch is thrown away (Pan Docs Rendering.md, "first12")
        self.penalised = false; // an object has been fetched on this line
        self.obj_wait_left = 0;
        self.obj_tiles_waited.clear();
        self.obj_dots = 0; // dots left of the object fetch
        self.obj_entry = None;
    }

    // ----- the background fetcher -------------------------------------------------------

    fn lcdc_ago(&self, lag: i32) -> u8 {
        if lag != 0 {
            self.lcdc_hist.ago(lag as usize)
        } else {
            self.lcdc
        }
    }

    fn fetcher_dot(&mut self) {
        let step = self.f_step;
        if step < 3 {
            self.f_dot += 1;
            if self.f_dot == 1 && step == 0 {
                if self.window_active && !self.cgb && self.lcdc & 0x20 == 0 {
                    // DMG: a tile fetch that sees the window disabled goes back to the
                    // background (SameBoy advance_fetcher_state_machine)
                    self.window_active = false;
                }
                self.t1_x = self.x; // the map column's position is taken on the step's first dot
                if self.skip != 0 && self.dropped > (self.scx & 7) as i32 {
                    // the drop has passed SCX's low bits and will wrap: SameBoy's position went
                    // back by 8 (Gambatte scx_during_m3 scx_0360c0)
                    self.t1_x = self.dropped - 8;
                }
                self.t1_obj = self.obj_entry.is_some(); // ... during an object's fetch
            }
            if self.f_dot == [MAP_READ_DOT, LOW_READ_DOT, HIGH_READ_DOT][step as usize] && step == 0
            {
                self.ticket_col = self.fetch_col;
                self.fetch_col += 1;
            }
            if self.f_dot < 2 {
                return;
            }
            self.f_dot = 0;
            self.f_step = step + 1;
            if self.f_step < 3 {
                return;
            }
        }
        // step 3: push, retried every dot until the FIFO is empty
        if self.bg == 0 {
            if self.dummy {
                self.dummy = false;
                self.fetch_col = if self.win_carry { 1 } else { 0 }; // a carried window goes on at tile 1
            } else {
                self.bg = 8;
                if self.free_skip != 0 {
                    self.bg = self.bg.saturating_sub(self.free_skip as usize);
                    self.free_skip = 0;
                }
            }
            self.f_step = 0;
            self.f_dot = 0;
        }
    }

    // ----- objects ----------------------------------------------------------------------

    /// One dot of an object fetch; the background fetcher is frozen.
    fn object_dot(&mut self) {
        self.obj_dots -= 1;
        let (ox, index, tile0, flags, oy) = self.obj_entry.expect("object entry");
        for (read_at, is_high) in [(self.obj_low_at, 0usize), (self.obj_high_at, 1)] {
            if self.obj_dots == read_at {
                // LCDC.2 is read at each of the two reads, and the row is cut to that height
                // (SameBoy get_object_line_address; Mealybug m3_lcdc_obj_size_change)
                let mask = if self.lcdc_ago(OBJ_SIZE_LAG) & 4 != 0 {
                    15
                } else {
                    7
                };
                let mut row = (self.ly - oy) & mask;
                if flags & 0x40 != 0 {
                    row ^= mask;
                }
                let mut tile = tile0;
                if mask == 15 {
                    tile &= 0xFE;
                }
                let bank = if self.cgb_mode && flags & 8 != 0 {
                    0x2000
                } else {
                    0
                };
                let data = self.vram[(bank + tile * 16 + row * 2) as usize + is_high];
                if is_high != 0 {
                    self.obj_hi = data;
                } else {
                    self.obj_lo = data;
                }
            }
        }
        if self.obj_dots == 0 {
            self.merge_object(ox, flags, index);
            self.obj_entry = None;
        }
    }

    /// CGB: the window was switched off just after it started; the background goes on.
    fn undo_window(&mut self) {
        let u = self.win_undo.take().expect("window undo");
        self.bg = u.bg;
        self.f_step = u.f_step;
        self.f_dot = u.f_dot;
        self.fetch_col = u.fetch_col;
        self.ticket_col = u.ticket_col;
        self.t1_x = u.t1_x;
        self.t1_obj = u.t1_obj;
        self.dummy = u.dummy;
        self.free_skip = u.free_skip;
        // the window's pixels already out stand for the background's
        self.bg = self.bg.saturating_sub((self.x - u.x).max(0) as usize);
        self.window_active = false;
        self.win_stall = 0;
        self.win_refund = WIN_UNDO_REFUND;
    }

    fn start_object_fetch(&mut self, entry: LineObj) {
        let mut dots = OBJ_FETCH_DOTS;
        if !self.penalised {
            // FITTED: the first object on a line costs less
            self.penalised = true;
            dots -= if self.cgb_mode && self.double_speed {
                FIRST_OBJECT_DISCOUNT_DS
            } else {
                FIRST_OBJECT_DISCOUNT
            };
        }
        self.obj_dots = dots;
        self.obj_low_at = (dots - 3).max(0);
        self.obj_high_at = (dots - 5).max(0);
        self.obj_entry = Some(entry);
        self.object_dot();
    }

    /// Pan Docs' "OBJ penalty algorithm" (Rendering.md): dots spent waiting for the background
    /// tile under the object's leftmost pixel, once per tile; an object at X=0 is a tile of its
    /// own that is waited for in full.
    fn object_wait(&mut self, ox: i32) -> i32 {
        let (key, wait) = if ox == 0 {
            ((false, None), 5)
        } else {
            let pixel = ox - 8;
            let position = if self.window_active {
                pixel - (self.wx as i32 - 7)
            } else {
                pixel + self.scx as i32
            };
            (
                (self.window_active, Some(position >> 3)),
                (7 - (position & 7) - 2).max(0),
            )
        };
        if self.obj_tiles_waited.contains(&key) {
            return 0;
        }
        self.obj_tiles_waited.push(key);
        wait
    }

    fn merge_object(&mut self, ox: i32, flags: i32, index: i32) {
        let skip = (8 - ox).max(0); // the part hanging off the left edge
        while (self.obj_fifo.len() as i32) < 8 - skip {
            self.obj_fifo.push((0, 0, 255));
        }
        let by_index = self.cgb_mode && self.opri & 1 == 0; // CGB: the lower OAM index wins
        for i in skip..8 {
            let bit = if flags & 0x20 != 0 { i } else { 7 - i };
            let colour =
                ((self.obj_lo as i32 >> bit) & 1) | (((self.obj_hi as i32 >> bit) & 1) << 1);
            let slot = (i - skip) as usize;
            let old = self.obj_fifo[slot];
            if colour != 0 && (old.0 == 0 || (by_index && index < old.2)) {
                self.obj_fifo[slot] = (colour, flags, index);
            }
        }
    }

    fn step_pixel(&mut self) {
        self.m3 += 1;
        if self.win_stall != 0 {
            self.win_stall -= 1;
            return;
        }
        if self.obj_dots != 0 {
            self.object_dot();
            return;
        }
        if let Some(entry) = self.obj_entry {
            // the wait for the object's tile is over: fetch it
            if self.obj_wait_left != 0 {
                self.obj_wait_left -= 1;
                self.fetcher_dot();
            } else {
                self.start_object_fetch(entry);
            }
            return;
        }
        let lcdc = self.lcdc;
        if self.skip != 0
            && self.bg != 0
            && self.dropped == (self.scx_hist.ago(1 + SKIP_LAG) & 7) as i32
        {
            self.skip = 0; // the next pixel is the first one shown
        }
        if let Some(undo) = &self.win_undo {
            if self.m3 > undo.end {
                self.win_undo = None;
            } else if self.lcdc & 0x20 == 0 {
                self.undo_window();
            }
        }
        if self.bg != 0 && self.skip == 0 {
            let x = self.x;
            let mut wx = if WX_LAG != 0 {
                self.wx_hist.ago(WX_LAG as usize) as i32
            } else {
                self.wx as i32
            };
            let win_lcdc = self.lcdc_ago(WIN_LAG);
            let mut edge = false;
            if x == 0 && !self.window_active {
                // WX below 7: the comparator matched on one of the hidden pixels left of the
                // screen, WX=6 one dot before the first shown pixel, WX=5 two. Those pixels are
                // not dots here, so each value is looked for where its dot was (Mealybug
                // m3_wx_4/5/6_change: a WX write lands between the WX=5 and the WX=6 dot).
                for low in 0..7 {
                    let depth = WX_LAG + WX_EDGE_LEAD - low;
                    let seen = if depth > 0 {
                        self.wx_hist.ago(depth as usize)
                    } else {
                        self.wx
                    };
                    if seen as i32 == low {
                        edge = true;
                        wx = low;
                        break;
                    }
                }
            }
            if self.window_active
                && x + 7 == wx
                && self.bg == 8
                && x != self.win_start_x
                && !self.cgb
                && self.f_step == 0
                && self.f_dot == 0
            {
                // DMG: WX matches again on the dot a window tile went into the shifter: a
                // colour-0 pixel goes out ahead of the tile (SameBoy insert_bg_pixel; Mealybug
                // m3_wx_4_change, m3_wx_5_change)
                self.bg += 1;
            }
            if !self.window_active
                && win_lcdc & 0x20 != 0
                && (win_lcdc & 1 != 0 || self.cgb_mode)
                && self.wy_hit
                && (x + 7 == wx || edge)
            {
                if wx == 166 && !self.cgb && WX166_CARRY != 0 {
                    // DMG: WX=166 does not start the window here, but it counts as a window line
                    // and the window starts the next line instead (SameBoy)
                    self.window_used = true;
                } else {
                    self.window_active = true;
                    self.win_mid = true;
                    self.win_start_x = x;
                    if self.window_used {
                        // every start on a line moves the window a row on (SameBoy window_y;
                        // Mealybug m3_lcdc_win_en_change_multiple)
                        self.window_line = (self.window_line + 1) & 255;
                    }
                    self.window_used = true;
                    if self.cgb {
                        self.win_undo = Some(WinUndo {
                            bg: self.bg,
                            f_step: self.f_step,
                            f_dot: self.f_dot,
                            fetch_col: self.fetch_col,
                            ticket_col: self.ticket_col,
                            t1_x: self.t1_x,
                            t1_obj: self.t1_obj,
                            dummy: self.dummy,
                            free_skip: self.free_skip,
                            end: self.m3 + WIN_UNDO_DOTS,
                            x,
                        });
                    }
                    if self.double_speed && self.penalised {
                        // an object was fetched before it
                        self.win_stall = WIN_OBJ_STALL_DS;
                    }
                    self.bg = 0;
                    self.f_step = 0;
                    self.f_dot = 0;
                    self.fetch_col = 0;
                    self.free_skip = (7 - wx).max(0); // hidden window pixels cost no dots
                    let head = if self.double_speed {
                        WINDOW_HEAD_DS
                    } else {
                        WINDOW_HEAD
                    };
                    for _ in 0..1 + if wx < 7 { WINDOW_EDGE_HEAD } else { head } {
                        self.fetcher_dot();
                    }
                    return;
                }
            }
            if self.next_obj < self.line_objs.len() && self.line_objs[self.next_obj].0 - 8 <= x {
                if lcdc & 2 == 0 {
                    self.next_obj += 1;
                } else {
                    let entry = self.line_objs[self.next_obj];
                    self.next_obj += 1;
                    self.obj_entry = Some(entry);
                    self.obj_wait_left = self.object_wait(entry.0);
                    if self.obj_wait_left != 0 {
                        // this dot is the first of the wait
                        self.obj_wait_left -= 1;
                        self.fetcher_dot();
                    } else {
                        self.start_object_fetch(entry);
                    }
                    return;
                }
            }
        }
        if self.bg != 0 {
            self.bg -= 1;
            if self.skip != 0 {
                self.dropped = (self.dropped + 1) & 7;
            } else {
                if !self.obj_fifo.is_empty() {
                    self.obj_fifo.remove(0);
                }
                self.x += 1;
                if self.x == SCREEN_W as i32 - HBLANK_IRQ_EARLY
                    && self.next_obj >= self.line_objs.len()
                    && !self.line0
                {
                    self.hblank_early = true; // the HBlank interrupt precedes mode 0 by a few dots
                }
                if self.x >= SCREEN_W as i32 {
                    self.win_carry =
                        (WX166_CARRY != 0 && !self.cgb && self.wy_hit && lcdc & 0x20 != 0)
                            && self.wx == 166;
                    if self.mode3_start < MODE2_END {
                        // line 0: the pipeline ran early, STAT did not
                        self.mode0_dot = self.dot + MODE2_END - self.mode3_start;
                    } else {
                        self.mode = 0;
                    }
                    self.drawing = false;
                    return;
                }
            }
        }
        self.fetcher_dot();
    }

    /// RGB555 colour `colour` of palette `number` in palette RAM.
    pub fn rgb(palette: &[u8], number: usize, colour: usize) -> u16 {
        let i = number * 8 + colour * 2;
        (palette[i] as u16 | ((palette[i + 1] as u16) << 8)) & 0x7FFF
    }

    /// LY as software reads it: on line 153 it reads 153 for only the first few dots and then
    /// 0 for the rest of the line (Pan Docs STAT.md; Gambatte hwtests/ly0, lyc153int_m2irq).
    pub fn visible_ly(&self) -> i32 {
        let early = if self.double_speed {
            2 + self.ly_lead
        } else {
            0
        };
        if self.ly == LAST_LINE && self.dot >= self.ly153_dots - early {
            0
        } else {
            self.ly
        }
    }

    fn update_stat_line(&mut self, ctx: &mut IrqCtx) {
        // (and for the first 4 dots of a new line it reads as false: Mooneye lcdon_timing-GS)
        self.lyc_flag = self.lyc_match();
        let select = self.stat_select;
        let oam = select & 0x20 != 0 && self.oam_pulse();
        let sources = [
            select & 0x40 != 0 && (self.lyc_flag || self.lyc_early_now()),
            select & 0x08 != 0
                && (if self.glitching {
                    self.read_mode == 0
                } else {
                    (self.mode == 0 || self.hblank_early)
                        && !(self.cgb
                            && select & 0x20 == 0
                            && self.dot >= DOTS_PER_LINE - M0_SRC_END
                            && self.ly < VBLANK_LINE - 1)
                })
                && !(self.line0 && self.dot < self.mode3_start),
            select & 0x10 != 0
                && self.mode == 1
                && !(self.ly == VBLANK_LINE && self.dot < M1_SRC_DOT && select & 0x08 == 0),
            oam,
        ];
        let line = sources.iter().any(|&s| s);
        // without blocking each source's own rising edge interrupts, line held high or not
        let rising = if self.stat_blocking {
            line && !self.stat_line
        } else {
            sources
                .iter()
                .zip(self.sources.iter())
                .any(|(&now, &before)| now && !before)
        };
        if self.stat_blocking && sources[1] && !self.sources[1] && self.stat_line {
            // a STAT or LYC write later in this M-cycle lands first: dropping the source that
            // holds the line high leaves the HBlank rise an edge (Gambatte m0enable/lycdisable_*)
            let phase = if self.cgb {
                self.sub >= M0_CANCEL_CGB && !self.in_lead()
            } else {
                true
            };
            if (if self.cgb {
                M0_CANCEL_CGB != 0
            } else {
                M0_CANCEL_DMG != 0
            }) && phase
            {
                self.m0_hidden = if self.in_lead() { 2 } else { 1 }; // a lead dot's: the next cycle's
            }
        }
        let raw = self.oam_pulse();
        if self.stat_blocking && raw && !self.oam_raw && self.stat_line && self.ly == 0 {
            // so does line 0's OAM pulse for a write enabling it (Gambatte late_*m1disable_ly0_*,
            // lyc0_late_m2enable_lycdisable_*): on DMG in every phase, on CGB in double speed
            if !self.cgb {
                self.oam_hidden = OAM_HIDDEN_DMG != 0 && !self.in_lead();
            } else if self.double_speed {
                self.oam_hidden = OAM_HIDDEN_DS != 0 && self.sub >= STAT_FALL_CGB_DS;
            }
        }
        self.oam_raw = raw;
        let prior = self.sources;
        self.sources = sources;
        let early_dot = DOTS_PER_LINE - OAM_IRQ_EARLY;
        if OAM_IRQ_EARLY != 0 && select & 0x20 != 0 && self.dot == early_dot && !self.stat_line {
            let following = if self.ly == LAST_LINE { 0 } else { self.ly + 1 };
            if 0 < following && following <= VBLANK_LINE {
                // the interrupt the CPU's dispatch sees a cycle before the line's pulse is real
                self.early_fired = true;
                self.irq_halt_delay = OAM_IRQ_EARLY;
                self.irq_early_by = OAM_IRQ_EARLY;
                self.irq_defer = self.in_lead();
                ppu_irq(self, ctx, 1);
            }
        }
        let line0_dot = DOTS_PER_LINE - EARLY_LINE0_DS;
        let line0_early = self.double_speed && self.ly == LAST_LINE && self.dot == line0_dot;
        if line0_early && select & 0x20 != 0 && !self.stat_line {
            self.early_fired = true;
            self.irq_halt_delay = EARLY_LINE0_DS;
            self.irq_early_by = EARLY_LINE0_DS;
            ppu_irq(self, ctx, 1);
        }
        if rising {
            if self.early_lyc153 && self.ly == LAST_LINE && !oam {
                self.early_lyc153 = false; // already raised, early
            } else if self.early_fired && oam {
                // already raised, early (CGB line 0: a STAT write in this dot can still cancel it)
                self.early_fired = self.cgb && self.ly == 0;
            } else {
                // the early HBlank interrupt, like the OAM one, wakes a halted CPU a cycle later
                self.irq_halt_delay = self.halt_delay(oam);
                let hblank = select & 0x08 != 0 && (self.mode == 0 || self.hblank_early);
                self.irq_early_by = if hblank { HBLANK_IRQ_EARLY } else { 0 };
                self.irq_defer = (hblank || oam) && self.in_lead();
                if select & 0x40 != 0 && self.lyc_early_now() && !self.lyc_flag && !hblank {
                    self.irq_early_by = self.lyc_lead(); // the dispatch sees it before IF does
                    self.lyc_early = self.double_speed;
                }
                if select & 0x10 != 0 && self.mode == 1 && self.ly == VBLANK_LINE && self.dot < 4 {
                    // the VBlank STAT source: the dispatch sees it on time, IF shows it later
                    self.irq_early_by = MODE1_IRQ_DOT;
                    self.irq_halt_delay = 0;
                    self.m1_early = M1_CANCEL_CGB != 0 && self.cgb;
                }
                if oam && self.ly == 0 && self.irq_early_by == 0 {
                    // line 0's OAM interrupt, raised on time, shows in IF a cycle later (Gambatte
                    // lyc153int_m2irq_1, ly0/lycint152_m2irq_1)
                    self.irq_early_by = if self.cgb {
                        OAM_LY0_IF_CGB
                    } else {
                        OAM_LY0_IF_DMG
                    };
                }
                ppu_irq(self, ctx, 1);
                if sources[0] && !prior[0] && !self.cgb && !self.in_lead() {
                    self.lyc_rose = LYC_CANCEL_DMG != 0;
                }
                if oam && self.cgb && self.ly == 0 {
                    self.early_fired = true; // a STAT write in this dot can still cancel it
                }
            }
        }
        if self.dot == 1 {
            self.early_fired = false;
            self.lyc_early = false;
            self.m1_early = false;
            self.early_lyc153 = false;
        }
        if self.stat_line && !line && !self.in_lead() {
            let fell = if !self.cgb {
                STAT_FALL_DMG != 0
            } else if self.double_speed {
                STAT_FALL_CGB_DS != 0 && self.sub >= STAT_FALL_CGB_DS
            } else {
                STAT_FALL_CGB != 0 && self.sub >= STAT_FALL_CGB && !prior[0]
            };
            if fell {
                // the sources that held the line: an LYC write beats only LY=LYC's or mode 1's
                self.stat_fell = prior
                    .iter()
                    .enumerate()
                    .filter(|(_, &high)| high)
                    .map(|(i, _)| 1 << i)
                    .sum();
            }
        }
        self.stat_line = line;
    }

    /// Dots after its raise at which a halted CPU sees this interrupt: the HBlank one lands
    /// 2 dots after it is raised (GBMicrotest int_hblank_halt_*, Mooneye
    /// hblank_ly_scx_timing), whether it was raised early or on time.
    fn halt_delay(&self, oam: bool) -> i32 {
        if oam {
            return 0;
        }
        let late = if self.cgb { HALT_LATE_CGB } else { 0 };
        if self.hblank_early && self.mode != 0 {
            return 2 + late;
        }
        (if self.mode == 0 && self.select_has_hblank() {
            2
        } else {
            0
        }) + late
    }

    pub fn select_has_hblank(&self) -> bool {
        self.stat_select & 0x08 != 0
    }

    /// The LY the comparator sees: 153 for line 153's first 8 dots (a read sees it for 4);
    /// on a CGB D or E the whole sequence runs 4 dots later (machines.rs lyc153_4_dots_later).
    fn compare_ly(&self) -> i32 {
        if self.ly == LAST_LINE {
            return if self.dot < 8 + self.lyc153_shift() {
                LAST_LINE
            } else {
                0
            };
        }
        self.ly
    }

    /// Dots line 153's LY=LYC sequence runs later on a CGB D or E than on a DMG or CGB C.
    fn lyc153_shift(&self) -> i32 {
        if self.lyc153_late {
            4
        } else {
            0
        }
    }

    /// The LY=LYC flag: false for 4 dots after the comparator's LY changes (at a line's start,
    /// and on line 153 at dot 8; line 0 keeps LYC=0's flag across the wrap): GBMicrotest
    /// line_153_lyc*_stat_timing_*, Mooneye lcdon_timing-GS.
    fn lyc_match(&self) -> bool {
        let dot = self.dot;
        if dot < 4 && self.ly != 0 {
            return false;
        }
        if self.ly == LAST_LINE
            && (8 + self.lyc153_shift()..12 + self.lyc153_shift()).contains(&dot)
        {
            return false;
        }
        self.compare_ly() == self.lyc as i32
    }

    fn lyc_lead(&self) -> i32 {
        if self.cgb {
            LYC_IRQ_EARLY
        } else {
            LYC_IRQ_EARLY_DMG
        }
    }

    /// The LY=LYC interrupt comes LYC_IRQ_EARLY(_DMG) dots before the flag (GBMicrotest
    /// int_lyc_*).
    fn lyc_early_now(&self) -> bool {
        self.dot < 4
            && self.dot >= 4 - self.lyc_lead()
            && self.ly != 0
            && self.visible_ly() == self.lyc as i32
    }

    /// Called by the board after each M-cycle's bus access, with the registers as written.
    /// Line 153's LY=LYC interrupt for LYC=0 reaches the CPU's dispatch at the end of the cycle
    /// that ends at dot 8, so a LYC write in that cycle still cancels it (GBMicrotest
    /// line_153_lyc0_int_inc_sled, line_153_lyc_int_b).
    pub fn after_access(&mut self, ctx: &mut IrqCtx) {
        if self.ly == LAST_LINE
            && self.dot == 8 + self.lyc153_shift()
            && self.lcd_on()
            && self.stat_select & 0x40 != 0
            && self.lyc == 0
            && !self.stat_line
        {
            self.early_lyc153 = true;
            self.irq_halt_delay = 0;
            self.irq_early_by = self.lyc_lead();
            ppu_irq(self, ctx, 1);
        }
        let owed = std::mem::take(&mut self.owed);
        if owed != 0 && self.lcdc & 0x80 != 0 {
            for sub in 4 - owed..4 {
                self.sub = sub;
                self.dot_once(ctx);
            }
        }
    }

    /// This dot is one the PPU runs after the CPU's access (IRQ_LEAD).
    pub fn in_lead(&self) -> bool {
        !self.double_speed && self.sub >= 4 - IRQ_LEAD
    }

    /// WY==LY is compared whenever either changes, not only at a line's start: a WY written to
    /// the current line number late in the line still starts the window on the next one
    /// (Gambatte window/arg/late_wy_*).
    pub fn compare_wy(&mut self) {
        if self.cgb && WY_WAIT_CGB != 0 {
            self.wy_wait = WY_WAIT_CGB;
            return;
        }
        self.compare_wy_now();
    }

    fn compare_wy_now(&mut self) {
        if self.lcd_on()
            && self.ly < VBLANK_LINE
            && self.ly == self.wy as i32
            && self.lcdc & 0x20 != 0
        {
            self.wy_hit = true;
        }
    }

    /// The line-start OAM source: a short pulse at dots 0-3 of each line, VBlank's first line
    /// included (Mooneye vblank_stat_intr-GS, GBMicrotest lcdon_to_oam_int_l1).
    fn oam_pulse(&self) -> bool {
        let width = if self.double_speed && self.ly != 0 {
            OAM_IRQ_WIDTH_DS
        } else {
            OAM_IRQ_WIDTH
        };
        self.dot < width && self.ly <= VBLANK_LINE && !self.line0
    }

    // ----- registers --------------------------------------------------------------------

    pub fn read_stat(&self) -> u8 {
        if !self.lcd_on() {
            // mode 0, the comparison flag frozen
            return 0x80 | self.stat_select | if self.lyc_flag { 4 } else { 0 };
        }
        0x80 | self.stat_select | if self.lyc_flag { 4 } else { 0 } | self.read_mode
    }

    /// Write STAT; true when the write cancels the OAM interrupt raised early for the CPU's
    /// dispatch (CGB: its source rises later and there is no write glitch to raise it anyway).
    pub fn write_stat(&mut self, value: u8, ctx: &mut IrqCtx) -> bool {
        let mut cancel = self.cgb
            && self.early_fired
            && value & 0x20 == 0
            && self.dot
                < if self.ly == 0 {
                    OAM_RISE_LY0_CGB
                } else {
                    DOTS_PER_LINE - OAM_RISE_CGB
                };
        if cancel {
            self.early_fired = false;
        }
        if self.lyc_early && value & 0x40 == 0 {
            self.lyc_early = false;
            cancel = true;
        }
        if self.m0_early && value & 0x08 == 0 {
            self.m0_early = false;
            cancel = true;
        }
        if self.m1_early && value & 0x10 == 0 {
            self.m1_early = false;
            cancel = true;
        }
        let hidden = self.m0_hidden; // a rise the write itself shows (enabling bit 3) is no hidden one
        let oam_hidden = self.oam_hidden;
        if self.stat_fell != 0 && self.stat_blocking {
            self.stat_line = true; // the write lands before the fall earlier in its M-cycle
        }
        if self.lcd_on() && self.stat_write_glitch {
            // DMG: for the write's cycle every source is enabled, so a source that is true raises
            // the interrupt if the line was low (Pan Docs STAT.md; GBMicrotest stat_write_glitch_*)
            let saved = self.stat_select;
            self.stat_select = 0x78;
            self.glitching = true; // the glitch sees the mode as a read does
            self.update_stat_line(ctx);
            self.glitching = false;
            self.stat_select = saved;
        }
        self.stat_select = value & 0x78;
        if self.lcd_on() {
            self.update_stat_line(ctx);
            self.m0_hidden = hidden;
            self.unhide_m0(hidden != 0, ctx);
            if oam_hidden && self.sources[3] && !self.sources[..3].iter().any(|&s| s) {
                self.oam_hidden = false;
                self.irq_early_by = 0;
                self.irq_halt_delay = 0;
                self.irq_defer = false;
                ppu_irq(self, ctx, 1);
            }
        }
        cancel && !self.stat_line
    }

    /// A write in the M-cycle of a hidden HBlank rise dropped every other source first.
    fn unhide_m0(&mut self, hidden: bool, ctx: &mut IrqCtx) {
        if hidden && self.sources[1] && ![0, 2, 3].iter().any(|&i| self.sources[i]) {
            self.m0_hidden = 0;
            self.irq_early_by = 0;
            self.irq_halt_delay = 0;
            self.irq_defer = false;
            ppu_irq(self, ctx, 1);
        }
    }

    pub fn write_lcdc(&mut self, value: u8, ctx: &mut IrqCtx) {
        let was_on = self.lcd_on();
        self.lcdc = value;
        if was_on && !self.lcd_on() {
            // LCD off: LY, the dot and the mode reset
            self.ly = 0;
            self.dot = 0;
            self.mode = 0; // the LY=LYC flag and the STAT line stay as they were (stat_lyc_onoff)
            self.window_line = 0;
            self.window_used = false;
            self.window_active = false;
            self.wy_hit = false;
            self.win_carry = false;
            self.line_objs.clear();
            self.settle(0);
            self.px_mut().lcd_off();
            self.frame = self.blank(); // the screen goes blank (white)
        } else if !was_on && self.lcd_on() {
            // LCD on: line 0 starts now, but late
            self.ly = 0;
            self.dot = LINE0_START_DOT + if self.double_speed { LCD_ON_DS } else { 0 };
            self.ly_lead = if self.double_speed { LY_LEAD_DS } else { 0 };
            self.line0 = true;
            self.mode3_start = MODE2_END;
            self.mode = 0; // line 0 starts in mode 0 (Mooneye ppu/lcdon_timing-GS)
            self.skip_scan = true; // and has no OAM scan
            self.settle(0);
            self.wy_hit = self.wy == 0 && self.lcdc & 0x20 != 0;
            self.px_mut().lcd_on();
            self.update_stat_line(ctx);
        } else if self.lcd_on() && value & 0x20 != 0 {
            // LCDC.5 set: WY compared again
            if self.cgb {
                self.wy_wait = if self.double_speed {
                    WIN_EN_WAIT_DS
                } else {
                    WIN_EN_WAIT
                };
            } else {
                self.compare_wy_now();
            }
        }
    }

    /// Make `mode` the current and the lagged mode (a jump, not a transition).
    fn settle(&mut self, mode: u8) {
        self.mode = mode;
        self.read_mode = mode;
        self.recent = [mode; MODE_READ_LAG];
    }

    /// Put the beam at a line, dot and mode (the skip-boot hand-off state).
    pub fn place(&mut self, ly: i32, dot: i32, mode: u8, ctx: &mut IrqCtx) {
        self.ly = ly;
        self.dot = dot + IRQ_LEAD; // the PPU is the lead ahead of the access
        self.settle(mode);
        self.update_stat_line(ctx);
    }

    /// (The DMG's one-dot OR of the old and new BGP is drawn by pixels::Pixels.)
    pub fn write_bgp(&mut self, value: u8) {
        self.bgp = value;
    }

    // ----- CGB palette RAM ($FF68-$FF6B) ------------------------------------------------

    /// Palette RAM is out of reach of the CPU while the LCD is drawing (Pan Docs Palettes.md).
    pub fn palettes_blocked(&self) -> bool {
        self.lcd_on() && (self.read_mode == 3 || self.pal_hold > 0)
    }

    pub fn read_palette(&self, obj: bool) -> u8 {
        if self.palettes_blocked() {
            return 0xFF;
        }
        let spec = if obj { self.ocps } else { self.bcps };
        let pal = if obj { &self.obj_pal } else { &self.bg_pal };
        pal[(spec & 0x3F) as usize]
    }

    pub fn write_palette(&mut self, obj: bool, value: u8) {
        let mut spec = if obj { self.ocps } else { self.bcps };
        if !self.palettes_blocked() {
            let pal = if obj {
                &mut self.obj_pal
            } else {
                &mut self.bg_pal
            };
            pal[(spec & 0x3F) as usize] = value;
        }
        if spec & 0x80 != 0 {
            // auto-increment, even when the write was blocked
            spec = 0x80 | (spec.wrapping_add(1) & 0x3F);
            if obj {
                self.ocps = spec;
            } else {
                self.bcps = spec;
            }
        }
    }

    /// Write LYC; true when the write cancels an LY=LYC interrupt raised early in this dot.
    pub fn write_lyc(&mut self, value: u8, ctx: &mut IrqCtx) -> bool {
        if self.lyc_rose && value as i32 != self.visible_ly() {
            self.lyc_rose = false; // DMG: the write lands before the match earlier in its M-cycle
            self.write_lyc_now(value, ctx);
            return !self.stat_line;
        }
        if self.lyc_early && value as i32 != self.visible_ly() {
            self.lyc_early = false;
            self.write_lyc_now(value, ctx);
            return !self.stat_line;
        }
        if self.cgb && !self.double_speed && LYC_WAIT_CGB != 0 && self.lcd_on() {
            self.lyc_wait = LYC_WAIT_CGB;
            self.lyc_next = value;
            return false;
        }
        let fell = self.stat_fell & 5 != 0 && self.stat_blocking && !self.cgb && LYC_FALL_DMG != 0;
        if fell {
            self.stat_line = true; // DMG: as a STAT write does, it lands before the fall
        }
        self.write_lyc_now(value, ctx);
        if fell
            && self.stat_select & 0x40 != 0
            && self.lyc as i32 == self.visible_ly()
            && self.dot < 4
        {
            self.stat_line = true; // the match it makes carries over into the new line's
        }
        false
    }

    fn write_lyc_now(&mut self, value: u8, ctx: &mut IrqCtx) {
        self.lyc = value;
        if self.lcd_on() {
            self.update_stat_line(ctx);
            let hidden = self.m0_hidden == 1 || self.m0_hidden == 2;
            self.unhide_m0(hidden, ctx);
        }
    }

    pub fn read_ly(&self) -> u8 {
        if !self.lcd_on() {
            return 0;
        }
        if LY_LEAD != 0 && !self.line0 && self.dot >= DOTS_PER_LINE - LY_LEAD && self.ly < 143 {
            return (self.ly + 1) as u8; // LY reads as the next line a little before it starts
        }
        if self.ly_lead != 0 && self.dot >= DOTS_PER_LINE - self.ly_lead {
            return if self.ly == LAST_LINE {
                0
            } else {
                (self.ly + 1) as u8
            };
        }
        self.visible_ly() as u8
    }
}

/// The OAM-bug access pattern (the reference's 'read', 'write', 'readinc').
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OamBugKind {
    Read,
    Write,
    ReadInc,
}
