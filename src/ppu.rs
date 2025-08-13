use crate::mmu::MMU;

pub const SCREEN_WIDTH:  u8 = 160;
pub const SCREEN_HEIGHT: u8 = 144;

/// PPU: DMG scanline-based renderer with a simple timing model.
/// - Tracks LY (current scanline), the dot counter within the line, and the LCD mode.
/// - Produces an RGB framebuffer (WIDTH*HEIGHT*3).
/// - Triggers VBlank IRQ and optional STAT IRQs according to mode/LYC.
pub struct PPU {
    ly: u8,              // Current scanline (0..153)
    mode: PPUMode,       // Current LCD mode (0, 1, 2, 3)
    dot: u16,            // T-cycles elapsed within the current line (0..455)
                         // Increments every T-cycle and wraps at 456 (T-cycles needed per scanline)
    frame_ready: bool,
    palette: Palette,
}

#[derive(Clone, Copy, PartialEq)]
pub struct Palette {
    pub colors: [[u8; 3]; 4], // 4 shades; each is [R,G,B]
}

/// Classic DMG greenish palette (light to dark).
pub const GREEN_PALETTE: Palette = Palette {
    colors: [
        [224, 248, 208], // #E0F8D0
        [136, 192, 112], // #88C070
        [52,  104,  86], // #346856
        [8,    24,  32], // #081820
    ],
};

/// Example color palette (light to dark).
pub const COLOR_PALETTE: Palette = Palette {
    colors: [
        [255, 255, 255], // #FFFFFF
        [255, 255,   0], // #FFFF00
        [255,   0,   0], // #FF0000
        [  0,   0,   0], // #000000
    ],
};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum PPUMode {
    HBlank = 0,  // ~204 T-cycles (end of scanline; CPU can access OAM/VRAM)
    VBlank = 1,  // 10 lines × 456 T-cycles (lines 144..153)
    Oam    = 2,  // 80 T-cycles (OAM search)
    Vram   = 3,  // 172 T-cycles (pixel transfer)
}

impl PPU {
    /// Creates a PPU with LY=0, OAM mode, and the default (color) palette.
    pub fn new() -> Self {
        Self {
            ly: 0,
            mode: PPUMode::Oam,
            dot: 0,
            frame_ready: false,
            palette: COLOR_PALETTE,
        }
    }

    /// Advances the PPU by `tcycles` T-cycles.
    /// - Increments the dot counter, wraps to the next line every 456 dots.
    /// - Updates mode (2/3/0 during visible lines; 1 during VBlank).
    /// - Renders the current background and sprite line when entering HBlank.
    /// - Updates STAT mode bits; triggers IRQs according to settings.
    pub fn step(&mut self, mmu: &mut MMU, tcycles: u32, framebuffer: &mut [u8], pitch: usize) {
        for _ in 0..tcycles {
            self.dot += 1;

            if self.dot == 456 {
                self.dot = 0;
                self.next_line(mmu);
            }

            let new_mode = if self.ly >= 144 {
                PPUMode::VBlank
            } else if self.dot < 80 {
                PPUMode::Oam
            } else if self.dot < 252 {
                PPUMode::Vram
            } else {
                PPUMode::HBlank
            };

            if new_mode != self.mode {
                self.mode = new_mode;
                self.write_stat_mode(mmu);
            }

            // Render the visible line right when entering HBlank
            if self.mode == PPUMode::HBlank && self.dot == 252 && self.ly < 144 {
                self.render_bg_line(mmu, framebuffer, pitch);
                self.render_sprites_line(mmu, framebuffer, pitch);
            }
        }
    }

    /// Moves to the next scanline and manages VBlank/LY wrap.
    /// - Updates LY and writes it to the LY register (0xFF44).
    /// - Enters VBlank at LY=144, raises the VBlank IRQ (IF bit 0), marks frame ready.
    /// - Wraps LY to 0 after LY=153 and returns to OAM mode.
    fn next_line(&mut self, mmu: &mut MMU) {
        self.dot = 0;
        self.ly = self.ly.wrapping_add(1);
        mmu.write_byte(0xFF44, self.ly);

        if self.ly == 144 {
            // Enter VBlank
            self.mode = PPUMode::VBlank;
            self.write_stat_mode(mmu);
            let iflag = mmu.read_byte(0xFF0F) | 0x01; // VBlank IRQ (bit 0)
            mmu.write_byte(0xFF0F, iflag);
            self.frame_ready = true;
        } else if self.ly > 153 {
            // Wrap to the first visible line
            self.ly = 0;
            mmu.write_byte(0xFF44, self.ly);
            self.mode = PPUMode::Oam;
            self.write_stat_mode(mmu);
        } else if self.ly < 144 {
            // Back to visible-line pipeline
            self.mode = PPUMode::Oam;
            self.write_stat_mode(mmu);
        } else {
            // Middle of VBlank (LY 145..153)
            self.mode = PPUMode::VBlank;
            self.write_stat_mode(mmu);
        }
    }

    /// Fast-path STAT update for Tetris:
    /// - Updates mode bits (0–1) and the LYC==LY flag (bit 2).
    /// - Does NOT generate STAT IRQ (IF bit 1) — Tetris only relies on VBlank IRQs.
    /// - Keeps the other STAT bits unchanged and skips the write if no change.
    fn write_stat_mode(&self, mmu: &mut MMU) {
        const STAT_ADDR: u16 = 0xFF41;
        const LYC_ADDR:  u16 = 0xFF45;

        let stat = mmu.read_byte(STAT_ADDR);

        // Recompute coincidence bit (bit 2) and mode bits (0–1)
        let coincidence = (mmu.read_byte(LYC_ADDR) == self.ly) as u8;
        let new_stat = (stat & !0x07)                   // clear bits 0..2
            | ((self.mode as u8) & 0x03)               // mode → bits 0..1
            | (coincidence << 2);                      // LYC==LY → bit 2

        if new_stat != stat {
            mmu.write_byte(STAT_ADDR, new_stat);
        }
    }

    /// Renders the current background scanline using scroll and LCDC settings:
    /// - Honors LCDC bits: LCD enable (bit 7) and BG enable (bit 0); returns early if either is off.
    /// - Applies SCX/SCY scrolling with wrapping to select the BG pixel source.
    /// - Selects BG map base at 0x9800 or 0x9C00 depending on LCDC bit 3.
    /// - Selects tile data at 0x8000 (unsigned) or 0x8800/0x9000 (signed) depending on LCDC bit 4.
    /// - For each of the 160 pixels, fetches the tile row, extracts the 2-bit color index,
    ///   maps it through BGP (FF47), and writes RGB via the active palette.
    /// Note: No window layer or tile priority handling.
    fn render_bg_line(&mut self, mmu: &MMU, fb: &mut [u8], pitch: usize) {
        let y = self.ly;
        if y >= 144 { return; }

        let lcdc = mmu.read_byte(0xFF40);
        if (lcdc & 0x80) == 0 { return; } // LCD off
        if (lcdc & 0x01) == 0 { return; } // BG off

        let scx  = mmu.read_byte(0xFF43);
        let scy  = mmu.read_byte(0xFF42);
        let bgp  = mmu.read_byte(0xFF47);

        let src_y = y.wrapping_add(scy);
        let tile_row = (src_y as u16) / 8;
        let row_in_tile = (src_y % 8) as u16;

        let bg_map_base = if (lcdc & 0x08) != 0 { 0x9C00 } else { 0x9800 };
        let bg_map_row_addr = bg_map_base + tile_row * 32;

        for x in 0..SCREEN_WIDTH {
            let src_x = x.wrapping_add(scx);
            let tile_col = (src_x as u16) / 8;
            let tile_index = mmu.read_byte(bg_map_row_addr + tile_col);

            let tile_addr = if (lcdc & 0x10) != 0 {
                0x8000 + (tile_index as u16) * 16
            } else {
                0x9000u16.wrapping_add((tile_index as i8 as i16 as u16) * 16)
            };

            let bit = 7 - (src_x % 8);
            let b0 = mmu.read_byte(tile_addr + row_in_tile * 2);
            let b1 = mmu.read_byte(tile_addr + row_in_tile * 2 + 1);
            let color_id = ((b1 >> bit) & 1) << 1 | ((b0 >> bit) & 1);
            let shade = (bgp >> (color_id * 2)) & 0b11;

            put_px(fb, pitch, x as usize, y as usize, shade, self.palette);
        }
    }

    /// Renders all sprites intersecting the current scanline (simplified, with flip support):
    /// - Requires LCDC bits: LCD enable (bit 7) and OBJ enable (bit 1).
    /// - Assumes 8×8 sprites only (ignores OBJ size bit and 8×16 layout).
    /// - Processes OAM in order and draws at most 10 sprites per scanline (DMG rule).
    /// - Supports X/Y flip from OAM attributes (bits 5 and 6).
    /// - Ignores OBJ-to-BG priority; uses OBP0/OBP1 according to attributes.
    /// - Color 0 is transparent; only nonzero pixels are drawn.
    fn render_sprites_line(&mut self, mmu: &MMU, fb: &mut [u8], pitch: usize) {
        let y = self.ly as i16;
        if y >= SCREEN_HEIGHT as i16 { return; }

        let lcdc = mmu.read_byte(0xFF40);
        if (lcdc & 0x80) == 0 { return; }
        if (lcdc & 0x02) == 0 { return; }

        let obp0 = mmu.read_byte(0xFF48);
        let obp1 = mmu.read_byte(0xFF49);
        let oam_base = 0xFE00u16;

        let mut drawn = 0;
        for i in 0..40 {
            if drawn >= 10 { break; }

            let idx = oam_base + i * 4;
            let sy = mmu.read_byte(idx) as i16 - 16;
            let sx = mmu.read_byte(idx + 1) as i16 - 8;
            let tile = mmu.read_byte(idx + 2);
            let attr = mmu.read_byte(idx + 3);

            if y < sy || y >= sy + 8 { continue; }

            let pal = if (attr & 0x10) != 0 { obp1 } else { obp0 };

            let line = if (attr & 0x40) != 0 {
                7 - (y - sy) as u16
            } else {
                (y - sy) as u16
            };

            let tile_addr = 0x8000u16 + (tile as u16) * 16 + line * 2;
            let b0 = mmu.read_byte(tile_addr);
            let b1 = mmu.read_byte(tile_addr + 1);

            for px in 0..8 {
                let bit = if (attr & 0x20) != 0 { px } else { 7 - px };
                let color_id = (((b1 >> bit) & 1) << 1) | ((b0 >> bit) & 1);
                if color_id == 0 { continue; }

                let x = sx + px as i16;
                if x < 0 || x >= SCREEN_WIDTH as i16 { continue; }

                let shade = (pal >> (color_id * 2)) & 0b11;
                put_px(fb, pitch, x as usize, y as usize, shade, self.palette);
            }

            drawn += 1;
        }
    }

    /// Returns `true` if a frame has just been completed; clears the flag.
    pub fn is_frame_ready(&mut self) -> bool {
        let r = self.frame_ready;
        self.frame_ready = false;
        r
    }

    pub fn set_palette(&mut self, palette: Palette) { 
        self.palette = palette; 
    }
    
    pub fn get_palette(&self) -> Palette { 
        self.palette
    }
}

#[inline]
fn put_px(fb: &mut [u8], pitch: usize, x: usize, y: usize, shade: u8, palette: Palette) {
    // Use SDL pitch (stride) in case lines have padding
    let row_start = y * pitch;
    let i = row_start + x * 3;
    let c = palette.colors[shade as usize];
    fb[i]     = c[0];
    fb[i + 1] = c[1];
    fb[i + 2] = c[2];
}
