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

    /// Advances the PPU state by a given number of T-cycles.
    ///
    /// ## Behavior:
    /// - Increments the `dot` counter (0–455), wrapping every **456 dots** to start a new scanline.
    /// - Updates `LY` via [`next_line`](Self::next_line) when a scanline completes.
    /// - Determines the current PPU mode:
    ///   - **OAM** (Mode 2): Dots 0–79
    ///   - **VRAM** (Mode 3): Dots 80–251
    ///   - **HBlank** (Mode 0): Dots 252–455
    ///   - **VBlank** (Mode 1): All dots during `LY >= 144`
    /// - On entering **HBlank** for a visible scanline (`LY < 144`), renders:
    ///   - The background scanline.
    ///   - The sprites on that scanline.
    ///
    /// ## Timing Notes:
    /// - 456 dots per scanline.
    /// - 154 total scanlines (0–143 visible, 144–153 VBlank).
    ///
    /// ## Parameters:
    /// - `mmu`: Memory interface for register and VRAM access.
    /// - `tcycles`: Number of T-cycles to simulate.
    /// - `framebuffer`: Target buffer for pixel output.
    /// - `pitch`: Bytes per row in the framebuffer.
    pub fn step(&mut self, mmu: &mut MMU, tcycles: u32, framebuffer: &mut [u8], pitch: usize) {
        for _ in 0..tcycles {
            // Advance the current dot (pixel position within the scanline)
            self.dot += 1;

            // End of scanline: wrap dot counter and advance LY
            if self.dot == 456 {
                self.dot = 0;
                self.next_line(mmu); // Handles VBlank entry and LY wrapping
            }

            // Determine PPU mode based on LY and dot position
            let new_mode = if self.ly >= 144 {
                PPUMode::VBlank // All lines after 143 are VBlank
            } else if self.dot < 80 {
                PPUMode::Oam // Mode 2: OAM scan (sprite attribute fetch)
            } else if self.dot < 252 {
                PPUMode::Vram // Mode 3: Pixel transfer (rendering)
            } else {
                PPUMode::HBlank // Mode 0: Horizontal blanking
            };

            // Update mode if changed
            if new_mode != self.mode {
                self.mode = new_mode;
            }

            // When entering HBlank on a visible scanline, render the line
            if self.mode == PPUMode::HBlank && self.dot == 252 && self.ly < 144 {
                // Render background pixels for this scanline
                self.render_bg_line(mmu, framebuffer, pitch);

                // Render sprites for this scanline
                self.render_sprites_line(mmu, framebuffer, pitch);
            }
        }
    }


    /// Advances the PPU to the next scanline, handling VBlank entry and LY wrapping.
    ///
    /// ## Behavior:
    /// - Increments the `LY` (current scanline) register and writes it to `0xFF44`.
    /// - When `LY` reaches **144**, the PPU enters **VBlank** mode:
    ///   - Sets mode to `VBlank` (`PPUMode::VBlank`).
    ///   - Sets the VBlank interrupt flag (`IF` bit 0).
    ///   - Marks the current frame as ready to display (`frame_ready = true`).
    /// - When `LY` exceeds **153**, it wraps back to **0** and the PPU enters OAM mode.
    /// - For lines **0–143**, the PPU is in OAM mode, preparing to draw visible scanlines.
    /// - For lines **145–153**, the PPU stays in VBlank mode.
    ///
    /// ## Timing Notes:
    /// - `LY` increments once per scanline.
    /// - `LY=144..153` are the vertical blanking period.
    /// - `LY=0..143` are visible scanlines.
    ///
    /// ## Parameters:
    /// - `mmu`: Memory interface, used to write `LY` and set interrupt flags.
    fn next_line(&mut self, mmu: &mut MMU) {
        self.dot = 0; // Reset cycle counter for the new scanline
        self.ly = self.ly.wrapping_add(1); // Increment LY (wrap at 256, later adjusted)
        mmu.write_byte(0xFF44, self.ly); // Write LY to the hardware register

        if self.ly == 144 {
            // Reached the first VBlank line
            self.mode = PPUMode::VBlank; // Enter VBlank mode

            // Raise VBlank interrupt (IF bit 0)
            let iflag = mmu.read_byte(0xFF0F) | 0x01;
            mmu.write_byte(0xFF0F, iflag);

            // Signal that a full frame has been rendered
            self.frame_ready = true;

        } else if self.ly > 153 {
            // End of VBlank period, wrap to first visible line
            self.ly = 0;
            mmu.write_byte(0xFF44, self.ly); // Update LY register
            self.mode = PPUMode::Oam; // Start OAM search for the new frame

        } else if self.ly < 144 {
            // Still in visible scanlines
            self.mode = PPUMode::Oam; // Begin OAM search for the new line

        } else {
            // Lines 145..153: middle of the VBlank period
            self.mode = PPUMode::VBlank;
        }
    }


    /// Renders the current background scanline using LCDC and scroll registers.
    ///
    /// ## Requirements:
    /// - LCD must be enabled (`LCDC` bit 7).
    /// - Background rendering must be enabled (`LCDC` bit 0).
    ///
    /// ## Features & Limitations:
    /// - Applies SCX (scroll X) and SCY (scroll Y) with wrapping to determine source pixels.
    /// - Selects background tile map base at:
    ///   - `0x9800` when `LCDC` bit 3 = 0.
    ///   - `0x9C00` when `LCDC` bit 3 = 1.
    /// - Selects tile data from:
    ///   - `0x8000` (unsigned tile index) when `LCDC` bit 4 = 1.
    ///   - `0x8800`/`0x9000` (signed tile index) when `LCDC` bit 4 = 0.
    /// - Each pixel's 2-bit color index is mapped through the `BGP` register (`0xFF47`).
    /// - No support for the window layer or tile priority handling.
    ///
    /// ## Rendering Process:
    /// 1. Determine the source Y position using `LY` + `SCY` (with wrapping).
    /// 2. Identify the tile row and row offset inside the tile.
    /// 3. For each screen pixel (0..159), compute the source X position (`SCX` wrapping).
    /// 4. Fetch the tile index from the background tile map.
    /// 5. Compute the address of the tile graphics in VRAM.
    /// 6. Read the corresponding bitplanes, extract the 2-bit color index, map it via `BGP`, and draw.
    ///
    /// ## Parameters:
    /// - `mmu`: Memory interface for reading registers, tile maps, and tile data.
    /// - `fb`: Framebuffer buffer (indexed colors).
    /// - `pitch`: Bytes per row in the framebuffer.
    fn render_bg_line(&mut self, mmu: &MMU, fb: &mut [u8], pitch: usize) {
        let y = self.ly; // Current scanline (0..143)
        if y >= 144 { return; } // Outside visible area

        // Read LCDC control register
        let lcdc = mmu.read_byte(0xFF40);
        if (lcdc & 0x80) == 0 { return; } // LCD disabled
        if (lcdc & 0x01) == 0 { return; } // Background disabled

        // Read scroll registers and background palette
        let scx = mmu.read_byte(0xFF43); // Scroll X
        let scy = mmu.read_byte(0xFF42); // Scroll Y
        let bgp = mmu.read_byte(0xFF47); // Background palette

        // Compute Y position in the background map (wraps at 256)
        let src_y = y.wrapping_add(scy);
        let tile_row = (src_y as u16) / 8; // Which tile row in BG map
        let row_in_tile = (src_y % 8) as u16; // Which pixel row inside the tile

        // Select background map base depending on LCDC bit 3
        let bg_map_base = if (lcdc & 0x08) != 0 { 0x9C00 } else { 0x9800 };
        let bg_map_row_addr = bg_map_base + tile_row * 32; // 32 tiles per row in BG map

        // Loop over each screen pixel
        for x in 0..SCREEN_WIDTH {
            // Compute X position in the background (wraps at 256)
            let src_x = x.wrapping_add(scx);
            let tile_col = (src_x as u16) / 8; // Which tile column in BG map

            // Read tile index from BG map
            let tile_index = mmu.read_byte(bg_map_row_addr + tile_col);

            // Determine tile data address depending on LCDC bit 4
            let tile_addr = if (lcdc & 0x10) != 0 {
                // Unsigned tile index (0x8000..)
                0x8000 + (tile_index as u16) * 16
            } else {
                // Signed tile index (0x8800.. / 0x9000..)
                0x9000u16.wrapping_add((tile_index as i8 as i16 as u16) * 16)
            };

            // Bit position in the tile's row (most significant bit = leftmost pixel)
            let bit = 7 - (src_x % 8);

            // Fetch the two bitplanes for this row of the tile
            let b0 = mmu.read_byte(tile_addr + row_in_tile * 2);     // Low bitplane
            let b1 = mmu.read_byte(tile_addr + row_in_tile * 2 + 1); // High bitplane

            // Combine bits from both planes to form a 2-bit color index (0..3)
            let color_id = ((b1 >> bit) & 1) << 1 | ((b0 >> bit) & 1);

            // Map color index through BGP to get the shade (0..3)
            let shade = (bgp >> (color_id * 2)) & 0b11;

            // Draw pixel to framebuffer
            put_px(fb, pitch, x as usize, y as usize, shade, self.palette);
        }
    }

    /// Renders all 8×8 sprites that intersect the current scanline.
    ///
    /// ## Requirements:
    /// - LCD must be enabled (`LCDC` bit 7).
    /// - OBJ (sprite) rendering must be enabled (`LCDC` bit 1).
    ///
    /// ## Assumptions & Limitations:
    /// - Only supports 8×8 sprites. Ignores the `OBJ_SIZE` bit and 8×16 sprite layout.
    /// - Processes sprites in OAM order, respecting the DMG limit of **10 sprites per scanline**.
    /// - Uses `OBP0` or `OBP1` palette according to the OAM attribute bit 4.
    /// - Supports horizontal (`X flip`, OAM bit 5) and vertical (`Y flip`, OAM bit 6) flipping.
    /// - Does not handle OBJ-to-BG priority (OAM bit 7); sprites always draw over the background.
    /// - Color index 0 is treated as transparent and will not overwrite the framebuffer.
    ///
    /// ## Rendering Details:
    /// - Sprite coordinates are adjusted for the Game Boy's hardware offset:
    ///   - Y position in OAM is offset by -16 pixels.
    ///   - X position in OAM is offset by -8 pixels.
    /// - Each sprite pixel's final shade is determined by:
    ///   1. Extracting the 2-bit color ID from the sprite tile data.
    ///   2. Mapping that ID through the selected OBJ palette register.
    ///   3. Writing the resulting shade to the framebuffer if nonzero.
    ///
    /// ## Parameters:
    /// - `mmu`: Memory interface for reading LCDC, OAM, palette registers, and tile data.
    /// - `fb`: Framebuffer (8-bit per pixel indices into the system palette).
    /// - `pitch`: Number of bytes per framebuffer row.
    ///
    /// This function should be called once per scanline during PPU rendering.
    fn render_sprites_line(&mut self, mmu: &MMU, fb: &mut [u8], pitch: usize) {
        let y = self.ly as i16; // Current scanline (LY register)
        if y >= SCREEN_HEIGHT as i16 { return; } // Ignore lines beyond screen height

        // Read LCDC control register
        let lcdc = mmu.read_byte(0xFF40);
        if (lcdc & 0x80) == 0 { return; } // LCD disabled
        if (lcdc & 0x02) == 0 { return; } // OBJ rendering disabled

        // Read sprite palette registers
        let obp0 = mmu.read_byte(0xFF48);
        let obp1 = mmu.read_byte(0xFF49);

        // OAM base address (sprite attribute table)
        let oam_base = 0xFE00u16;

        let mut drawn = 0; // Count of sprites drawn on this scanline
        for i in 0..40 { // OAM has 40 sprite entries
            if drawn >= 10 { break; } // Hardware limit: max 10 sprites per scanline

            // Each sprite entry is 4 bytes in OAM
            let idx = oam_base + i * 4;
            let sy = mmu.read_byte(idx) as i16 - 16; // Y position (offset by -16 per hardware)
            let sx = mmu.read_byte(idx + 1) as i16 - 8; // X position (offset by -8 per hardware)
            let tile = mmu.read_byte(idx + 2); // Tile index in VRAM
            let attr = mmu.read_byte(idx + 3); // Attribute flags (palette, flip, priority)

            // Skip if the current scanline is outside this sprite's vertical range
            if y < sy || y >= sy + 8 { continue; }

            // Select palette: OBP0 or OBP1
            let pal = if (attr & 0x10) != 0 { obp1 } else { obp0 };

            // Determine which line of the tile to fetch (handle Y flip)
            let line = if (attr & 0x40) != 0 {
                7 - (y - sy) as u16 // Y-flip: read from opposite row
            } else {
                (y - sy) as u16 // Normal orientation
            };

            // Address in VRAM for the sprite's tile line (2 bytes per row)
            let tile_addr = 0x8000u16 + (tile as u16) * 16 + line * 2;
            let b0 = mmu.read_byte(tile_addr);     // Low bitplane
            let b1 = mmu.read_byte(tile_addr + 1); // High bitplane

            // Iterate over each pixel in the 8-pixel sprite row
            for px in 0..8 {
                // Handle X flip: choose bit position accordingly
                let bit = if (attr & 0x20) != 0 { px } else { 7 - px };
                
                // Extract 2-bit color ID from bitplanes
                let color_id = (((b1 >> bit) & 1) << 1) | ((b0 >> bit) & 1);
                if color_id == 0 { continue; } // Transparent pixel (color 0)

                // Calculate on-screen X position
                let x = sx + px as i16;
                if x < 0 || x >= SCREEN_WIDTH as i16 { continue; } // Skip off-screen pixels

                // Map color ID through palette register to get shade
                let shade = (pal >> (color_id * 2)) & 0b11;

                // Write pixel to framebuffer
                put_px(fb, pitch, x as usize, y as usize, shade, self.palette);
            }

            drawn += 1; // One more sprite rendered for this scanline
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
