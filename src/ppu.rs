use crate::mmu::MMU;

pub const SCREEN_WIDTH:  u8 = 160;
pub const SCREEN_HEIGHT: u8 = 144;

/// PPU: DMG scanline-based renderer with a simple timing model.
/// - Keeps LY (current line), the dot counter within the line, and the LCD mode.
/// - Builds an RGB framebuffer (WIDTH*HEIGHT*3).
/// - Raises VBlank IRQ and optional STAT IRQs according to mode/LYC.
pub struct PPU {
    pub fb: Vec<u8>,     // RGB framebuffer (WIDTH*HEIGHT*3)
    ly: u8,              // current scanline (0..153)
    mode: PPUMode,       // 0,1,2,3
    dot: u16,            // t-cycles elapsed within the current line (0..455)
                         // Increments every T-cycle and wraps at 456 (T-cycles needed per scanline)
    frame_ready: bool,
    palette: Palette,
}

#[derive(Clone, Copy, PartialEq)]
pub struct Palette {
    pub colors: [[u8; 3]; 4], // 4 shades; each is [R,G,B]
}

/// Classic greenish DMG palette (light to dark).
pub const GREEN_PALETTE: Palette = Palette {
    colors: [
        [224, 248, 208], // #E0F8D0
        [136, 192, 112], // #88C070
        [52,  104,  86], // #346856
        [8,    24,  32], // #081820
    ],
};

/// Color palette
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
    HBlank = 0,  // ~204 t-cycles (end of scanline; CPU can access OAM/VRAM)
    VBlank = 1,  // 10 lines * 456 t-cycles (lines 144..153)
    Oam    = 2,  // 80 t-cycles (OAM search)
    Vram   = 3,  // 172 t-cycles (pixel transfer)
}

impl PPU {
    /// Create a PPU with LY=0, OAM mode, and the default (color) palette.
    pub fn new() -> Self {
        Self {
            fb: vec![0; SCREEN_WIDTH as usize * SCREEN_HEIGHT as usize * 3],
            ly: 0,
            mode: PPUMode::Oam,
            dot: 0,
            frame_ready: false,
            palette: COLOR_PALETTE,
        }
    }

    /// Advance the PPU by `tcycles` T-cycles.
    /// - Increments the dot counter, rolls to next line every 456 dots.
    /// - Updates mode (2/3/0 during visible lines; 1 during VBlank).
    /// - Renders the current background and sprites line when entering HBlank.
    /// - Raises STAT IRQs depending on enables and conditions.
    pub fn step(&mut self, mmu: &mut MMU, tcycles: u32) {
        for _ in 0..tcycles {
            // 1) Advance "dot" position within the scanline
            self.dot += 1;

            // 2) Line change every 456 dots
            if self.dot == 456 {
                self.dot = 0;
                self.next_line(mmu);
            }

            // 3) Determine LCD mode based on dot and LY
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

            // 4) Render visible line right when entering HBlank (end of mode 3)
            if self.mode == PPUMode::HBlank && self.dot == 252 && self.ly < 144 {
                self.render_bg_line(mmu);
                self.render_sprites_line(mmu);
            }
        }
    }

    /// Move to the next scanline and manage VBlank/LY wrap.
    /// - Updates LY and writes it to LY register (0xFF44).
    /// - Enters VBlank at LY=144, raises VBlank IRQ (IF bit 0), marks frame ready.
    /// - Wraps LY to 0 after LY=153 and goes back to OAM mode.
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

    /// Tetris-only fast path for STAT updates:
    /// - Update mode bits (0–1) and the LYC==LY flag (bit 2).
    /// - Do NOT generate STAT IRQ (IF bit 1) — Tetris relies on VBlank only.
    /// - Keep the rest of STAT bits as-is and avoid the write if unchanged.
    fn write_stat_mode(&self, mmu: &mut MMU) {
        const STAT_ADDR: u16 = 0xFF41;
        const LYC_ADDR:  u16 = 0xFF45;

        let stat = mmu.read_byte(STAT_ADDR);

        // Recompute coincidence bit (bit 2) and mode bits (0–1)
        let coincidence = (mmu.read_byte(LYC_ADDR) == self.ly) as u8;
        let new_stat = (stat & !0x07)                   // clear bits 0..2
            | ((self.mode as u8) & 0x03)               // mode -> bits 0..1
            | (coincidence << 2);                      // LYC==LY -> bit 2

        if new_stat != stat {
            mmu.write_byte(STAT_ADDR, new_stat);
        }
    }

    /// Render the current background scanline using live scroll and LCDC settings:
    /// - Honors LCDC: LCD enable (bit 7) and BG enable (bit 0); returns early if either is off.
    /// - Applies SCX/SCY scrolling (wrapping) to choose the source BG pixel.
    /// - Selects BG map base at 0x9800 or 0x9C00 depending on LCDC bit 3.
    /// - Selects tile data at 0x8000 (unsigned) or 0x8800/0x9000 (signed) depending on LCDC bit 4.
    /// - For each of the 160 screen pixels, fetches the tile row bytes, extracts the 2-bit color id,
    ///   maps it through BGP (FF47), and writes the RGB value using the current palette.
    /// Notes: window layer and tile priorities/attributes are not handled.
    fn render_bg_line(&mut self, mmu: &MMU) {
        let y = self.ly;
        if y >= 144 { return; }

        // Read registers
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

            // Choose tile data region: 0x8000 (unsigned) or 0x8800/0x9000 (signed)
            let tile_addr = if (lcdc & 0x10) != 0 {
                0x8000 + (tile_index as u16) * 16
            } else {
                // signed index
                0x9000u16.wrapping_add((tile_index as i8 as i16 as u16) * 16)
            };

            let bit = 7 - (src_x % 8);
            let b0 = mmu.read_byte(tile_addr + row_in_tile * 2);
            let b1 = mmu.read_byte(tile_addr + row_in_tile * 2 + 1);
            let color_id = ((b1 >> bit) & 1) << 1 | ((b0 >> bit) & 1);
            let shade = (bgp >> (color_id * 2)) & 0b11;

            put_px(&mut self.fb, x as usize, y as usize, shade, self.palette);
        }
    }

    /// Render sprites that intersect the current scanline (Tetris-oriented simplification with flip support):
    /// - Requires LCDC bits: LCD enable (bit 7) and OBJ enable (bit 1).
    /// - Assumes 8×8 sprites only (ignores OBJ size bit 2 and 8×16 layout).
    /// - Processes OAM in order and draws at most 10 sprites per line (DMG rule).
    /// - Supports X/Y flip attributes from OAM (bits 5 and 6 of attribute byte).
    /// - Ignores OBJ-to-BG priority; uses OBP0/OBP1 as selected by the attribute.
    /// - Color 0 is transparent; only nonzero pixels are drawn.
    /// - Sprite pixels are drawn "as is" without background priority checks.
    fn render_sprites_line(&mut self, mmu: &MMU) {
        let y = self.ly as i16;
        if y >= SCREEN_HEIGHT as i16 { return; }

        let lcdc = mmu.read_byte(0xFF40);
        if (lcdc & 0x80) == 0 { return; } // LCD off
        if (lcdc & 0x02) == 0 { return; } // Sprites off

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

            // Intersects 8x8 sprite?
            if y < sy || y >= sy + 8 { continue; }

            let pal = if (attr & 0x10) != 0 { obp1 } else { obp0 };

            // Calculate sprite line with vertical flip support
            let line = if (attr & 0x40) != 0 { 
                7 - (y - sy) as u16  // Y-flip: invert line order
            } else { 
                (y - sy) as u16 
            };

            let tile_addr = 0x8000u16 + (tile as u16) * 16 + line * 2;
            let b0 = mmu.read_byte(tile_addr);
            let b1 = mmu.read_byte(tile_addr + 1);

            // Draw 8 pixels with horizontal flip support
            for px in 0..8 {
                let bit = if (attr & 0x20) != 0 { 
                    px  // X-flip: read bits left-to-right
                } else { 
                    7 - px  // Normal: read bits right-to-left
                };

                let color_id = (((b1 >> bit) & 1) << 1) | ((b0 >> bit) & 1);
                if color_id == 0 { continue; }

                let x = sx + px as i16;
                if x < 0 || x >= SCREEN_WIDTH as i16 { continue; }

                let shade = (pal >> (color_id * 2)) & 0b11;
                put_px(&mut self.fb, x as usize, y as usize, shade, self.palette);
            }

            drawn += 1;
        }
    }

    /// Returns whether a frame has just completed; clears the flag on read.
    pub fn is_frame_ready(&mut self) -> bool {
        let r = self.frame_ready;
        self.frame_ready = false;
        r
    }

    /// Set the active output palette (used by BG and sprites in this renderer).
    pub fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
    }

    /// Get the current palette.
    pub fn get_palette(&self) -> Palette {
        self.palette
    }
}

/// Write one RGB pixel into the linear framebuffer using a 2-bit shade index.
/// Shade is mapped to RGB via the selected palette.
fn put_px(fb: &mut [u8], x: usize, y: usize, shade: u8, palette: Palette) {
    let i = (y * SCREEN_WIDTH as usize + x) * 3;
    let c = palette.colors[shade as usize];
    fb[i]     = c[0]; // R
    fb[i + 1] = c[1]; // G
    fb[i + 2] = c[2]; // B
}
