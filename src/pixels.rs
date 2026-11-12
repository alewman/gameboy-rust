//! The picture: OAM scan and mode 3 as a background fetcher, an object fetcher and two FIFOs
//! (gameboy-python `pixels.py`).
//!
//! This machine only draws. STAT, the interrupts and the VRAM/OAM locks keep their timing from
//! `ppu::Ppu` (fitted to Mooneye and Gambatte); this one runs alongside it, `DRAW_LAG` dots
//! behind, reading the registers, VRAM and OAM as they are on each of its dots, and writes the
//! picture into its own frame, on DMG and CGB hardware alike (a CGB in its own mode or in the
//! compatibility mode a DMG cartridge gets, `cgb_mode` false).
//!
//! It is a translation of gameboy-python's translation of docboy's pixel pipeline
//! (`src/docboy/docboy/ppu/ppu.cpp`; github.com/Docheinstein/docboy, MIT licence, notice below),
//! the emulator that draws all of Mealybug Tearoom's DMG and CGB-D pictures and cgb-acid-hell.
//! What it models, in its words:
//!
//! * the first fetch of a line is thrown away (dots 80-82), and `SCX % 8` pixels are dropped;
//! * `lx` counts from 0 and the first 8 pixels are popped but not shown, so an object at OAM X
//!   is met at `lx == X`: objects hanging off the left edge are fetched during those 8 pixels;
//! * the background fetcher: GetTile (the map address from SCX, SCY, LCDC.3; the tile number
//!   read), GetTileDataLow and GetTileDataHigh (each works out its address from SCY and LCDC.4
//!   *at that dot*, so a write between them mixes two rows or two tiles), then Push, which waits
//!   for an empty FIFO;
//! * an object due at this `lx` is fetched only from the Push step; the tile already fetched is
//!   kept and pushed after it, and the background FIFO does not move while the object is
//!   fetched;
//! * the window starts when `lx` meets WX, and each start moves the window a row on; a Push on
//!   the dot `lx == WX` after the window was seen this frame pushes one colour-0 pixel instead.
//!
//! The DMG: WX is seen a dot late, and the window starts a pixel late if LCDC.5 was off a dot
//! before; a pixel is coloured with LCDC.0/.1 as they were a dot before (except at `lx == 8`)
//! and with `BGP | the previous dot's BGP` (Mealybug m3_bgp_change); with LCDC.0 clear the
//! background's colour 0 still goes through BGP (Gambatte, SameBoy; docboy draws white).
//!
//! The CGB: LCDC, SCX, SCY and WX writes reach it `CGB_REG_DELAY` dots late (one more in double
//! speed); no BGP glitch; SCY is taken once per tile, at GetTile; three glitches: TILE_SEL
//! changed between a data read's two dots, WX met again during a window tile fetch, and the
//! window turned off on the dot it starts; and, in its own mode, each tile's attributes from
//! VRAM bank 1 (palette, bank, flips, priority), objects in OAM order, and SameBoy's TILE_SEL
//! glitch (`number_glitch`).
//!
//! Not docboy's, and taken from Gambatte's hardware pictures: the map column follows the pixel
//! position and SCX (`scx_during_m3`), the `SCX % 8` drop follows SCX as it changes, and the
//! DMG's WX=166 rule (see `check_window`).
//!
//! Where Rust forced a difference: the reference's bound-method stages (`stage`, `fetch`) are
//! enums dispatched by `run_stage` and `run_fetch`; the PPU is passed to `tick` instead of held.
//!
//! docboy, Copyright (c) 2025 Docheinstein. Permission is hereby granted, free of charge, to any
//! person obtaining a copy of this software and associated documentation files (the "Software"),
//! to deal in the Software without restriction, including without limitation the rights to use,
//! copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and
//! to permit persons to whom the Software is furnished to do so, subject to the following
//! conditions: The above copyright notice and this permission notice shall be included in all
//! copies or substantial portions of the Software. THE SOFTWARE IS PROVIDED "AS IS", WITHOUT
//! WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
//! MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
//! AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN
//! AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE
//! SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

use serde::{Deserialize, Serialize};

use crate::ppu::Ppu;

pub const SCREEN_W: usize = 160;
pub const SCREEN_H: usize = 144;
/// lx runs 0..167: 8 hidden pixels, then the 160 shown ones
pub const LAST_X: i32 = 168;
/// the dot pixel transfer starts at, on every line
pub const PIXELS_START: i32 = 80;
/// the thrown-away first fetch: the FIFO is filled with junk on its last dot
pub const DUMMY_DOTS: i32 = 3;
pub const DMA_INDEX_BACK: i32 = 1;
/// CGB: dots before an LCDC, SCX, SCY or WX write reaches the pixel pipeline
pub const CGB_REG_DELAY: usize = 2;
/// FITTED (Gambatte cgb04c scx_during_m3 *_ds_*): ... in double speed
pub const CGB_REG_DELAY_DS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stage {
    Idle,
    Dummy,
    Discard,
    DiscardWx0,
    Pixels,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Fetch {
    FetchIdle,
    BgwinTile0,
    BgTile1,
    BgLow0,
    BgLow1,
    BgHigh0,
    BgHigh1,
    WinTile1,
    WinLow0,
    WinLow1,
    WinHigh0,
    WinHigh1,
    ObjTile0,
    ObjTile1,
    ObjLow0,
    ObjLow1,
    ObjHigh0,
    ObjHigh1,
}

/// The drawing machine (see the module notes).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pixels {
    pub cgb: bool,
    pub framebuffer: Vec<u16>,
    /// the registers as the pipeline sees them on this dot (the CGB's arrive late)
    pub lcdc: u8,
    pub scx: u8,
    pub scy: u8,
    pub wx: u8,
    /// CGB: what LCDC will be on the next dot
    pub next_lcdc: u8,
    pub last_bgp: u8,
    pub last_wx: u8,
    pub last_lcdc: u8,
    /// WY met LY with the window on, at some dot of this frame
    pub active_for_frame: bool,
    /// the window's line counter (goes to 0 at its first start)
    pub wly: i32,
    /// the first line after the LCD is turned on has no OAM scan
    pub after_on: bool,
    /// DMG, WX=166: the next line starts inside the window
    pub carry: bool,
    pub stage: Stage,
    pub oam_entries: Vec<Vec<(u8, u8)>>,
    pub scan_count: i32,
    pub scan_y: u8,
    pub scan_x: u8,
    /// CGB: the last data the fetcher read, for its glitches
    pub last_low: u8,
    pub last_high: u8,
    pub last_unsigned: u8,
    pub last_fetch: u8,
    pub last_fetch_decay: i32,
    pub ly: i32,
    // the fetcher (reset_fetcher)
    pub lx: i32,
    pub popped: bool,
    /// colour number | the tile's CGB attributes << 2
    pub bg_fifo: Vec<u16>,
    /// CGB mode: the tile's attributes, from VRAM bank 1
    pub tile_attrs: u8,
    /// (colour, attributes, entry, x)
    pub obj_fifo: Vec<(u8, u8, u8, i32)>,
    pub push_enabled: bool,
    pub pending_fill: bool,
    /// CGB: the window-off glitch refills only this many pixels
    pub partial_fill: usize,
    pub fetching_object: bool,
    pub w_activating: bool,
    pub w_active: bool,
    pub w_just_activated: bool,
    /// CGB: the window was turned off on the dot it started
    pub w_off_while_starting: bool,
    pub tile_number: u8,
    /// CGB: SCY taken at GetTile
    pub tile_scy: u8,
    pub map_address: usize,
    pub data_address: usize,
    pub data_low: u8,
    pub data_high: u8,
    /// a fetched tile held over an object fetch
    pub kept: Option<(u8, u8, u8)>,
    /// the window fetcher's tile counter
    pub win_x: i32,
    pub obj_entry: (u8, u8),
    pub obj_tile: u8,
    pub obj_attrs: u8,
    pub discarded: i32,
    pub dummy_left: i32,
    pub fetch: Fetch,
}

impl Pixels {
    pub fn new(cgb: bool) -> Pixels {
        let mut p = Pixels {
            cgb,
            framebuffer: vec![if cgb { 0x7FFF } else { 0 }; SCREEN_W * SCREEN_H],
            lcdc: 0,
            scx: 0,
            scy: 0,
            wx: 0,
            next_lcdc: 0,
            last_bgp: 0,
            last_wx: 0,
            last_lcdc: 0,
            active_for_frame: false,
            wly: 255,
            after_on: false,
            carry: false,
            stage: Stage::Idle,
            oam_entries: vec![Vec::new(); LAST_X as usize],
            scan_count: 0,
            scan_y: 0,
            scan_x: 0,
            last_low: 0,
            last_high: 0,
            last_unsigned: 0,
            last_fetch: 0,
            last_fetch_decay: 0,
            ly: 0,
            lx: 0,
            popped: false,
            bg_fifo: Vec::new(),
            tile_attrs: 0,
            obj_fifo: Vec::new(),
            push_enabled: false,
            pending_fill: false,
            partial_fill: 0,
            fetching_object: false,
            w_activating: false,
            w_active: false,
            w_just_activated: false,
            w_off_while_starting: false,
            tile_number: 0,
            tile_scy: 0,
            map_address: 0,
            data_address: 0,
            data_low: 0,
            data_high: 0,
            kept: None,
            win_x: 0,
            obj_entry: (0, 0),
            obj_tile: 0,
            obj_attrs: 0,
            discarded: 0,
            dummy_left: 0,
            fetch: Fetch::BgwinTile0,
        };
        p.reset_fetcher();
        p
    }

    // ----- per dot ----------------------------------------------------------------------

    /// One dot at line `ly`, dot `dot`.
    pub fn tick(&mut self, p: &Ppu, ly: i32, dot: i32) {
        if self.cgb {
            let d = if p.double_speed {
                CGB_REG_DELAY_DS
            } else {
                CGB_REG_DELAY
            };
            self.lcdc = p.lcdc_hist.ago(1 + d);
            self.scx = p.scx_hist.ago(1 + d);
            self.scy = p.scy_hist.ago(1 + d);
            self.wx = p.wx_hist.ago(1 + d);
            self.next_lcdc = p.lcdc_hist.ago(d);
        } else {
            self.lcdc = p.lcdc;
            self.scx = p.scx;
            self.scy = p.scy;
            self.wx = p.wx;
        }
        self.ly = ly;
        if ly < SCREEN_H as i32 {
            if dot == 0 {
                if ly == 0 {
                    self.wly = 255;
                    self.active_for_frame = false;
                }
                self.scan_count = 0;
                for entries in &mut self.oam_entries {
                    entries.clear();
                }
                self.stage = Stage::Idle;
            }
            if dot < 80 && dot & 1 == 0 && !self.after_on {
                self.scan(p, dot >> 1);
            }
            if dot == PIXELS_START {
                self.reset_fetcher();
                if self.carry {
                    self.carry = false;
                    self.wly = (self.wly + 1) & 255;
                    self.w_active = true;
                    self.win_x = 1;
                }
                self.dummy_left = DUMMY_DOTS;
                self.stage = Stage::Dummy;
            }
            self.run_stage(p);
        }
        // the window's WY condition is checked on every dot of the frame
        if !self.active_for_frame && self.lcdc & 0x20 != 0 && ly == p.wy as i32 {
            self.active_for_frame = true;
        }
        self.last_bgp = p.bgp;
        self.last_wx = self.wx;
        self.last_lcdc = self.lcdc;
    }

    fn run_stage(&mut self, p: &Ppu) {
        match self.stage {
            Stage::Idle => {}
            Stage::Dummy => self.dummy(),
            Stage::Discard => self.discard(p),
            Stage::DiscardWx0 => self.discard_wx0(),
            Stage::Pixels => self.pixels(p),
        }
    }

    pub fn lcd_off(&mut self) {
        let fill = if self.cgb { 0x7FFF } else { 0 };
        self.framebuffer.iter_mut().for_each(|v| *v = fill);
        self.carry = false;
        self.reset_fetcher();
        for entries in &mut self.oam_entries {
            entries.clear();
        }
        self.stage = Stage::Idle;
    }

    pub fn lcd_on(&mut self) {
        self.after_on = true;
        self.active_for_frame = false;
        self.wly = 255;
    }

    pub fn line_done(&mut self) {
        self.after_on = false;
    }

    // ----- OAM scan ---------------------------------------------------------------------

    fn dma_busy(p: &Ppu) -> bool {
        p.dma.active
    }

    fn scan(&mut self, p: &Ppu, entry: i32) {
        if entry >= 40 {
            return;
        }
        let e = entry as usize;
        if !Self::dma_busy(p) {
            // an OAM DMA holds the bus: the scan does not read, and goes on with the Y and X it
            // read last (docboy oam_scan_flush_read_request; strikethrough.gb)
            self.scan_y = p.oam[e * 4];
            self.scan_x = p.oam[e * 4 + 1];
        }
        if self.scan_count >= 10 {
            return;
        }
        let y = self.scan_y as i32;
        let height = if self.lcdc & 4 != 0 { 16 } else { 8 };
        let top = y - 16;
        if top <= self.ly && self.ly < top + height {
            let x = self.scan_x as i32;
            if x < LAST_X {
                self.oam_entries[x as usize].push((entry as u8, self.scan_y));
            }
            self.scan_count += 1;
        }
    }

    // ----- pixel transfer stages --------------------------------------------------------

    fn reset_fetcher(&mut self) {
        self.lx = 0;
        self.popped = false;
        self.bg_fifo.clear();
        self.tile_attrs = 0;
        self.obj_fifo.clear();
        self.push_enabled = false;
        self.pending_fill = false;
        self.partial_fill = 0;
        self.fetching_object = false;
        self.w_activating = false;
        self.w_active = false;
        self.w_just_activated = false;
        self.w_off_while_starting = false;
        self.tile_number = 0;
        self.tile_scy = 0;
        self.last_fetch_decay = 0; // CGB: the last read is not reused across a line start
        self.map_address = 0;
        self.data_address = 0;
        self.data_low = 0;
        self.data_high = 0;
        self.kept = None;
        self.win_x = 0;
        self.obj_entry = (0, 0);
        self.obj_tile = 0;
        self.obj_attrs = 0;
        self.discarded = 0;
        self.dummy_left = 0;
        self.fetch = Fetch::BgwinTile0;
    }

    fn dummy(&mut self) {
        self.dummy_left -= 1;
        if self.dummy_left != 0 {
            return;
        }
        self.bg_fifo = vec![0; 8]; // junk: these 8 pixels are never shown
        self.discarded = 0;
        self.stage = Stage::Discard;
        if self.scx & 7 != 0 {
            self.check_window(); // with SCX % 8 > 0 the window can start before the background
            if self.w_activating {
                self.stage = Stage::DiscardWx0;
            }
        }
    }

    /// SCX % 8 pixels are dropped (lx does not move). The drop ends when its count meets SCX's
    /// low bits *now*, not as they were when it began (Gambatte scx_during_m3, hardware;
    /// SameBoy compares on every dropped pixel; docboy reads SCX once).
    fn discard(&mut self, p: &Ppu) {
        if self.discarded == (self.scx & 7) as i32 {
            self.stage = Stage::Pixels;
            self.pixels(p);
            return;
        }
        self.pending_fill_now();
        if self.can_pop() {
            self.bg_fifo.remove(0);
            self.discarded = (self.discarded + 1) & 7;
        }
        self.tick_fetcher(p);
        self.push(p);
    }

    fn discard_wx0(&mut self) {
        self.pending_window();
        self.stage = Stage::Discard; // a window at WX=0 starts a dot late: no fetcher dot here
    }

    fn pixels(&mut self, p: &Ppu) {
        self.pending_window();
        self.pending_fill_now();
        let mut popped = false;
        if self.can_pop() {
            let colour = self.bg_fifo.remove(0);
            let obj = if self.obj_fifo.is_empty() {
                None
            } else {
                Some(self.obj_fifo.remove(0))
            };
            let lx = self.lx;
            if lx >= 8 {
                self.colour(p, colour, obj, lx);
                if lx + 1 == LAST_X {
                    self.next_lx();
                    self.stage = Stage::Idle;
                    // DMG: WX=166 never opens the window on its own line, but the next line
                    // starts inside it, at its second tile (SameBoy; Gambatte window/on_screen/
                    // wxA6_*). The CGB starts the window at 166 as at any other WX (docboy).
                    self.carry = !self.cgb
                        && (self.active_for_frame && self.lcdc & 0x20 != 0 && self.wx == 166);
                    return;
                }
            }
            popped = true;
            self.check_window();
        }
        self.popped = popped; // the fetcher sees the position after this dot's pixel
        self.tick_fetcher(p);
        self.popped = false;
        if self.cgb && (self.w_activating || self.w_just_activated) && self.disabling_window() {
            self.push_enabled = true; // CGB: the window turned off as it starts pushes at once
        }
        self.push(p);
        if popped {
            self.next_lx();
        }
    }

    fn colour(&mut self, p: &Ppu, pixel: u16, obj: Option<(u8, u8, u8, i32)>, lx: i32) {
        let mut colour = (pixel & 3) as usize;
        let i = (self.ly as usize) * SCREEN_W + lx as usize - 8;
        let lcdc = if self.cgb || lx == 8 {
            self.lcdc
        } else {
            self.last_lcdc // LCDC.0/.1 a dot late, but at lx 8
        };
        if p.cgb_mode {
            // CGB mode: LCDC.0 clear takes the background's priority away, it still draws; a
            // background pixel of colour 1-3 is over the object when either asks for it
            let attrs = (pixel >> 2) as u8;
            if let Some(o) = obj {
                if o.0 != 0
                    && lcdc & 2 != 0
                    && (lcdc & 1 == 0 || colour == 0 || (attrs | o.1) & 0x80 == 0)
                {
                    self.framebuffer[i] = Ppu::rgb(&p.obj_pal, (o.1 & 7) as usize, o.0 as usize);
                    return;
                }
            }
            self.framebuffer[i] = Ppu::rgb(&p.bg_pal, (attrs & 7) as usize, colour);
            return;
        }
        if let Some(o) = obj {
            if o.0 != 0 && lcdc & 2 != 0 && (lcdc & 1 == 0 || colour == 0 || o.1 & 0x80 == 0) {
                let palette = if o.1 & 0x10 != 0 { p.obp1 } else { p.obp0 };
                let shade = ((palette >> (o.0 * 2)) & 3) as usize;
                self.framebuffer[i] = if self.cgb {
                    Ppu::rgb(&p.obj_pal, ((o.1 >> 4) & 1) as usize, shade)
                } else {
                    shade as u16
                };
                return;
            }
        }
        if self.cgb {
            let shade = if lcdc & 1 != 0 {
                ((p.bgp >> (colour * 2)) & 3) as usize
            } else {
                0
            };
            self.framebuffer[i] = Ppu::rgb(&p.bg_pal, 0, shade);
        } else {
            // LCDC.0 clear: the background's colour number is 0, still looked up in BGP
            // (Gambatte ppu.cpp tileword, SameBoy; Megami Tensei Gaiden: Last Bible II fades this
            // way). docboy and Pan Docs draw white, which is the same for the usual BGP
            if lcdc & 1 == 0 {
                colour = 0;
            }
            self.framebuffer[i] = (((p.bgp | self.last_bgp) >> (colour * 2)) & 3) as u16;
        }
    }

    fn next_lx(&mut self) {
        self.oam_entries[self.lx as usize].clear(); // objects not fetched at this lx are dropped
        self.lx += 1;
    }

    fn can_pop(&self) -> bool {
        !self.bg_fifo.is_empty()
            && !self.fetching_object
            && (self.oam_entries[self.lx as usize].is_empty() || self.lcdc & 2 == 0)
    }

    fn obj_due(&self) -> bool {
        !self.oam_entries[self.lx as usize].is_empty() && self.lcdc & 2 != 0
    }

    fn pending_fill_now(&mut self) {
        if self.pending_fill {
            self.pending_fill = false;
            self.bg_fifo = self.tile_row();
        } else if self.partial_fill != 0 {
            // CGB, the window turned off as it starts: the FIFO's pixels left are overwritten
            // with whatever the fetcher holds
            let size = std::mem::take(&mut self.partial_fill);
            let mut row = self.tile_row();
            row.truncate(size);
            self.bg_fifo = row;
        }
    }

    fn tile_row(&self) -> Vec<u16> {
        let attrs = self.tile_attrs;
        row(self.data_low, self.data_high, attrs & 0x20 != 0, attrs)
    }

    /// The Push step: into an empty FIFO, or an object fetch from here.
    fn push(&mut self, p: &Ppu) {
        if !self.push_enabled || self.fetching_object {
            return;
        }
        let can_push = self.bg_fifo.is_empty();
        if can_push {
            if self.active_for_frame
                && self.lx == self.wx as i32
                && !self.w_just_activated
                && (self.w_active || !self.cgb)
            {
                self.bg_fifo.push(0); // the WX glitch: a colour-0 pixel, the tile waits a dot
                return;
            }
            self.pending_fill = true;
            self.push_off();
            self.fetch = Fetch::BgwinTile0;
            if self.w_just_activated {
                // no object fetch just after the window's first push
                self.w_just_activated = false;
                return;
            }
        }
        if self.obj_due() {
            if !can_push {
                self.kept = Some((self.data_low, self.data_high, self.tile_attrs));
            }
            self.fetching_object = true;
            self.obj_entry = self.oam_entries[self.lx as usize].pop().expect("object");
            self.obj_tile_0(p);
        }
    }

    fn push_off(&mut self) {
        self.push_enabled = false;
    }

    fn tick_fetcher(&mut self, p: &Ppu) {
        if self.last_fetch_decay != 0 {
            self.last_fetch_decay -= 1;
        }
        self.run_fetch(p);
    }

    fn run_fetch(&mut self, p: &Ppu) {
        match self.fetch {
            Fetch::FetchIdle => {}
            Fetch::BgwinTile0 => self.bgwin_tile_0(p),
            Fetch::BgTile1 => self.bg_tile_1(),
            Fetch::BgLow0 => self.bg_low_0(p),
            Fetch::BgLow1 => self.bg_low_1(p),
            Fetch::BgHigh0 => self.bg_high_0(p),
            Fetch::BgHigh1 => self.bg_high_1(p),
            Fetch::WinTile1 => self.win_tile_1(),
            Fetch::WinLow0 => self.win_low_0(p),
            Fetch::WinLow1 => self.win_low_1(p),
            Fetch::WinHigh0 => self.win_high_0(p),
            Fetch::WinHigh1 => self.win_high_1(p),
            Fetch::ObjTile0 => self.obj_tile_0(p),
            Fetch::ObjTile1 => self.obj_tile_1(p),
            Fetch::ObjLow0 => self.obj_low_0(),
            Fetch::ObjLow1 => self.obj_low_1(p),
            Fetch::ObjHigh0 => self.obj_high_0(),
            Fetch::ObjHigh1 => self.obj_high_1(p),
        }
    }

    // ----- window -----------------------------------------------------------------------

    /// CGB: a write turning the window off reaches the pipeline on the next dot.
    fn disabling_window(&self) -> bool {
        self.next_lcdc != self.lcdc && self.next_lcdc & 0x20 == 0
    }

    fn check_window(&mut self) {
        let lx = self.lx;
        if !(self.active_for_frame && !self.w_active && self.lcdc & 0x20 != 0) {
            return;
        }
        if self.cgb {
            if lx != self.wx as i32 {
                return;
            }
        } else if !(lx == self.last_wx as i32
            || (lx == self.last_wx as i32 + 1 && self.last_lcdc & 0x20 == 0))
        {
            return;
        }
        if !self.cgb && self.last_wx == 166 {
            // DMG: WX=166 moves the window a row on but does not open it (SameBoy)
            self.wly = (self.wly + 1) & 255;
            return;
        }
        self.w_activating = true;
        self.wly = (self.wly + 1) & 255;
        self.push_off();
        if self.cgb && self.disabling_window() {
            self.w_off_while_starting = true; // the fetcher is not held this time
            return;
        }
        self.fetch = Fetch::FetchIdle; // the next fetcher dot is lost
    }

    fn pending_window(&mut self) {
        if self.w_activating {
            if self.cgb && self.lcdc & 0x20 == 0 {
                // CGB: turned off on the dot it started: the background goes on, and the FIFO's
                // pixels left are refilled with what the fetcher holds (all 8 if it is empty)
                self.partial_fill = if self.bg_fifo.is_empty() {
                    8
                } else {
                    self.bg_fifo.len()
                };
                self.w_off_while_starting = false;
            } else {
                self.bg_fifo.clear();
                self.w_active = true;
                self.w_just_activated = true;
                self.win_x = 0;
            }
            self.w_activating = false;
            self.fetch = Fetch::BgwinTile0;
        }
    }

    // ----- background / window fetcher --------------------------------------------------

    fn bgwin_tile_0(&mut self, p: &Ppu) {
        if self.w_active {
            if self.lcdc & 0x20 != 0 {
                self.win_tile_0(p);
                return;
            }
            self.w_active = false; // the window was turned off: back to the background
        }
        self.bg_tile_0(p);
    }

    fn bg_tile_0(&mut self, p: &Ppu) {
        // the map column is worked out from the pixel position and SCX at this dot, as
        // SameBoy's position_in_line + SCX: a write that changes SCX's low bits mid-line moves
        // the column (Gambatte scx_during_m3, hardware; docboy keeps a tile counter, which
        // agrees while SCX stands still). The DMG counts the pixel popped on this dot, the CGB
        // does not.
        let position = if self.cgb {
            self.lx
        } else {
            self.lx + self.popped as i32
        };
        let tile_x = ((position + self.scx as i32) >> 3) & 31;
        let tile_y = ((self.ly + self.scy as i32) & 255) >> 3;
        self.map_address =
            ((if self.lcdc & 8 != 0 { 0x1C00 } else { 0x1800 }) + tile_y * 32 + tile_x) as usize;
        if self.cgb && (self.w_off_while_starting || self.last_fetch_decay == 1) {
            self.tile_number = self.last_fetch; // CGB: the window-off glitch reuses the last read
        } else {
            self.read_tile_number(p);
        }
        self.tile_scy = self.scy;
        self.fetch = Fetch::BgTile1;
    }

    fn bg_tile_1(&mut self) {
        self.fetch = Fetch::BgLow0;
    }

    fn data_base(&self) -> usize {
        let n = self.tile_number as i32;
        (if self.lcdc & 0x10 != 0 {
            n * 16
        } else {
            0x1000 + ((n ^ 0x80) - 0x80) * 16
        }) as usize
    }

    fn bg_data_address(&mut self) {
        let scy = if self.cgb { self.tile_scy } else { self.scy }; // the CGB takes SCY once, at GetTile
        let mut row = (self.ly + scy as i32) & 7;
        let attrs = self.tile_attrs;
        if attrs & 0x40 != 0 {
            row ^= 7; // CGB mode: vertical flip
        }
        self.data_address =
            self.data_base() + row as usize * 2 + if attrs & 8 != 0 { 0x2000 } else { 0 };
    }

    fn bg_low_0(&mut self, p: &Ppu) {
        self.bg_data_address();
        if self.cgb && self.w_off_while_starting {
            self.data_low = self.last_fetch;
        } else {
            self.read_low(p, false);
        }
        self.fetch = Fetch::BgLow1;
    }

    fn bg_low_1(&mut self, p: &Ppu) {
        if self.cgb {
            self.sel_glitch(p, false, false);
        }
        self.fetch = Fetch::BgHigh0;
    }

    fn bg_high_0(&mut self, p: &Ppu) {
        self.bg_data_address();
        if self.cgb && self.w_off_while_starting {
            self.data_high = self.last_fetch;
        } else {
            self.read_high(p, false);
        }
        self.fetch = Fetch::BgHigh1;
    }

    fn bg_high_1(&mut self, p: &Ppu) {
        if self.cgb {
            self.sel_glitch(p, false, true);
        }
        self.bgwin_high_1();
    }

    fn win_tile_0(&mut self, p: &Ppu) {
        if self.lcdc & 0x20 == 0 {
            self.bg_tile_0(p);
            return;
        }
        let tile_x = self.win_x & 31;
        self.win_x += 1;
        self.map_address = ((if self.lcdc & 0x40 != 0 { 0x1C00 } else { 0x1800 })
            + (self.wly >> 3) * 32
            + tile_x) as usize;
        if !(self.cgb && self.lx == self.wx as i32) {
            // CGB: WX met again keeps the last tile
            self.read_tile_number(p);
        }
        self.fetch = Fetch::WinTile1;
    }

    fn win_tile_1(&mut self) {
        if self.lcdc & 0x20 == 0 {
            self.bg_tile_1();
            return;
        }
        self.fetch = Fetch::WinLow0;
    }

    fn win_data_address(&mut self) {
        let mut row = self.wly & 7;
        let attrs = self.tile_attrs;
        if attrs & 0x40 != 0 {
            row ^= 7; // CGB mode: vertical flip
        }
        self.data_address =
            self.data_base() + row as usize * 2 + if attrs & 8 != 0 { 0x2000 } else { 0 };
    }

    fn win_low_0(&mut self, p: &Ppu) {
        if self.lcdc & 0x20 == 0 {
            self.bg_low_0(p);
            return;
        }
        self.win_data_address();
        if self.cgb && self.lx == self.wx as i32 {
            self.data_low = self.last_high; // CGB: WX met again: the last high byte instead
        } else {
            self.read_low(p, false);
        }
        self.fetch = Fetch::WinLow1;
    }

    fn win_low_1(&mut self, p: &Ppu) {
        if self.lcdc & 0x20 == 0 {
            self.bg_low_1(p);
            return;
        }
        if self.cgb {
            self.sel_glitch(p, true, false);
        }
        self.fetch = Fetch::WinHigh0;
    }

    fn win_high_0(&mut self, p: &Ppu) {
        if self.lcdc & 0x20 == 0 {
            self.bg_high_0(p);
            return;
        }
        self.win_data_address();
        if self.cgb && self.lx == self.wx as i32 {
            self.data_high = self.last_low; // CGB: WX met again: the last low byte instead
        } else {
            self.read_high(p, false);
        }
        self.fetch = Fetch::WinHigh1;
    }

    fn win_high_1(&mut self, p: &Ppu) {
        if self.cgb {
            self.sel_glitch(p, true, true);
        }
        self.bgwin_high_1();
    }

    fn bgwin_high_1(&mut self) {
        self.fetch = Fetch::FetchIdle;
        self.push_enabled = true;
    }

    /// CGB: LCDC.4 changed between a data read's two dots. Cleared: the read is made again
    /// with the signed address; set: the data is the last read made with unsigned addresses
    /// (an object's, maybe), for tiles in $9000-$97FF only (docboy; Mealybug
    /// m3_lcdc_tile_sel_change, CGB D).
    fn sel_glitch(&mut self, p: &Ppu, window: bool, high: bool) {
        if self.last_lcdc & 0x10 != 0 && self.lcdc & 0x10 == 0 {
            if window {
                self.win_data_address();
            } else {
                self.bg_data_address();
            }
            if high {
                self.read_high(p, false);
            } else {
                self.read_low(p, false);
            }
        } else if self.last_lcdc & 0x10 == 0
            && self.lcdc & 0x10 != 0
            && self.data_address & 0x800 == 0
        {
            // bit 11 of the address in its bank
            if high {
                self.data_high = self.last_unsigned;
            } else {
                self.data_low = self.last_unsigned;
            }
        }
    }

    // ----- VRAM reads -------------------------------------------------------------------

    fn read_tile_number(&mut self, p: &Ppu) {
        self.tile_number = p.vram[self.map_address];
        self.tile_attrs = if p.cgb_mode {
            p.vram[0x2000 + self.map_address]
        } else {
            0
        };
        if self.cgb {
            self.data_high = self.tile_number; // the CGB's tile number and high byte share a latch
            self.last_fetch = self.tile_number;
            self.last_fetch_decay = 2;
        }
    }

    /// CGB mode, single speed: a background data read on the dot LCDC.4's fall reaches the
    /// pipeline returns the tile's number, for numbers below $80 (SameBoy
    /// data_for_tile_sel_glitch; cgb-acid-hell, column 80).
    fn number_glitch(&self, p: &Ppu) -> bool {
        if !(p.cgb_mode && !p.double_speed && self.tile_number < 0x80) {
            return false;
        }
        let h = &p.lcdc_hist;
        let k = CGB_REG_DELAY;
        h.ago(2 + k) & !h.ago(1 + k) & 0x10 != 0
    }

    fn read_low(&mut self, p: &Ppu, obj: bool) {
        if !obj && self.number_glitch(p) {
            self.data_low = self.tile_number;
            return;
        }
        let value = p.vram[self.data_address];
        self.data_low = value;
        if self.cgb {
            self.last_low = value;
            if obj || self.lcdc & 0x10 != 0 {
                self.last_unsigned = value;
            }
            self.last_fetch = value;
            self.last_fetch_decay = 2;
        }
    }

    fn read_high(&mut self, p: &Ppu, obj: bool) {
        if !obj && self.number_glitch(p) {
            self.data_high = self.tile_number;
            return;
        }
        let value = p.vram[self.data_address + 1];
        self.data_high = value;
        if self.cgb {
            self.last_high = value;
            if obj || self.lcdc & 0x10 != 0 {
                self.last_unsigned = value;
            }
            self.last_fetch = value;
            self.last_fetch_decay = 2;
        }
    }

    // ----- object fetcher ---------------------------------------------------------------

    fn obj_tile_0(&mut self, _p: &Ppu) {
        self.fetch = Fetch::ObjTile1;
    }

    fn obj_tile_1(&mut self, p: &Ppu) {
        let mut address = self.obj_entry.0 as i32 * 4 + 2;
        if Self::dma_busy(p) {
            // the PPU does not drive OAM's address while a DMA writes: the read gets the pair of
            // bytes at the DMA's address (docboy OamBus::read_word_request; strikethrough.gb)
            address = (p.dma.index as i32 - DMA_INDEX_BACK) & 0xFE;
        }
        self.obj_tile = p.oam[address as usize];
        self.obj_attrs = p.oam[address as usize + 1];
        if !p.cgb_mode {
            self.obj_attrs &= 0xF0; // the low nibble is a CGB game's
        }
        self.fetch = Fetch::ObjLow0;
    }

    fn obj_data_address(&mut self) {
        let tall = self.lcdc & 4 != 0;
        let mask = if tall { 15 } else { 7 };
        let mut row = (self.ly - (self.obj_entry.1 as i32 - 16)) & mask;
        if self.obj_attrs & 0x40 != 0 {
            row ^= mask;
        }
        let tile = if tall {
            self.obj_tile & 0xFE
        } else {
            self.obj_tile
        } as usize;
        self.data_address =
            tile * 16 + row as usize * 2 + if self.obj_attrs & 8 != 0 { 0x2000 } else { 0 };
    }

    fn obj_low_0(&mut self) {
        self.fetch = Fetch::ObjLow1;
    }

    fn obj_low_1(&mut self, p: &Ppu) {
        self.obj_data_address();
        self.read_low(p, true);
        self.fetch = Fetch::ObjHigh0;
    }

    fn obj_high_0(&mut self) {
        self.fetch = Fetch::ObjHigh1;
    }

    fn obj_high_1(&mut self, p: &Ppu) {
        self.obj_data_address();
        self.read_high(p, true);
        let attrs = self.obj_attrs;
        let number = self.obj_entry.0;
        let lx = self.lx;
        let colours = row(self.data_low, self.data_high, attrs & 0x20 != 0, 0);
        let by_x = !self.cgb || p.opri & 1 != 0; // the CGB in its own mode: OAM order only
        for (i, &c) in colours.iter().enumerate() {
            let c = c as u8;
            if i < self.obj_fifo.len() {
                let old = self.obj_fifo[i];
                // a new opaque pixel wins over a transparent one, or over an opaque one of an
                // object at the same X (any X, by OAM order) that comes later in OAM
                if c != 0 && (old.0 == 0 || ((old.3 == lx || !by_x) && number < old.2)) {
                    self.obj_fifo[i] = (c, attrs, number, lx);
                }
            } else {
                self.obj_fifo.push((c, attrs, number, lx));
            }
        }
        if self.obj_due() {
            self.obj_entry = self.oam_entries[self.lx as usize].pop().expect("object");
            self.fetch = Fetch::ObjTile0;
        } else {
            if let Some((low, high, attrs)) = self.kept.take() {
                self.data_low = low;
                self.data_high = high;
                self.tile_attrs = attrs;
            }
            self.fetching_object = false;
            // back to the Push step with the kept tile; with none kept (the object came on a
            // push into an empty FIFO, which docboy leaves idle) the next tile is fetched
            self.fetch = if self.push_enabled {
                Fetch::FetchIdle
            } else {
                Fetch::BgwinTile0
            };
        }
    }
}

/// The 8 colour numbers of a tile row, leftmost first, each with `attrs << 2` added.
fn row(low: u8, high: u8, flip: bool, attrs: u8) -> Vec<u16> {
    let tag = (attrs as u16) << 2;
    (0..8)
        .map(|i| {
            let b = if flip { i } else { 7 - i };
            ((low as u16 >> b) & 1) | (((high as u16 >> b) & 1) << 1) | tag
        })
        .collect()
}
