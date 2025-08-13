use crate::cpu::CPU;
use crate::cartridge::Cartridge;
use crate::mmu::MMU;
use crate::ppu::{GREEN_PALETTE, COLOR_PALETTE, PPU};

/// GB façade: connects the CPU, MMU (bus/memory), and PPU together.
///
/// Responsibilities:
/// - Initializes the system: sets up the MMU with the cartridge and instantiates CPU and PPU.
/// - Executes one CPU instruction per call to `step`, then advances the PPU
///   by the number of T-cycles the instruction consumed.
/// - Exposes input methods that pass button states to the MMU.
/// - When the PPU signals a completed frame, the RGB data is written directly
///   into the provided SDL framebuffer and `step` returns `true`.
///
/// Timing contract:
/// - `CPU::step` returns the number of T-cycles taken by the executed instruction.
/// - `PPU::step(mmu, t_cycles, framebuffer, pitch)` advances video timing by that amount.
/// - `is_frame_ready()` indicates whether a full frame has been rendered since the last check.
pub const BTN_RIGHT:  u8 = 1 << 0;
pub const BTN_LEFT:   u8 = 1 << 1;
pub const BTN_UP:     u8 = 1 << 2;
pub const BTN_DOWN:   u8 = 1 << 3;
pub const BTN_A:      u8 = 1 << 4;
pub const BTN_B:      u8 = 1 << 5;
pub const BTN_SELECT: u8 = 1 << 6;
pub const BTN_START:  u8 = 1 << 7;

/// High-level Game Boy system wrapper that orchestrates CPU, MMU, and PPU.
pub struct GB {
    cpu: CPU,
    mmu: MMU,
    ppu: PPU,
}

impl GB {
    /// Creates a new Game Boy instance with the given cartridge loaded.
    pub fn new(cartridge: Cartridge) -> Self {
        let mmu = MMU::new(cartridge);

        GB {
            cpu: CPU::new(),
            mmu,
            ppu: PPU::new(),
        }
    }

    /// Executes a single CPU instruction and advances the PPU accordingly.
    ///
    /// The framebuffer passed in is an SDL texture buffer; the PPU writes RGB
    /// pixels directly into it using the provided `pitch` (bytes per row).
    ///
    /// Returns `true` if a new frame has been rendered and is ready to be presented.
    pub fn step(&mut self, framebuffer: &mut [u8], pitch: usize) -> bool {
        let t = self.cpu.step(&mut self.mmu);
        self.ppu.step(&mut self.mmu, t, framebuffer, pitch);
        self.ppu.is_frame_ready()
    }

    /// Marks one or more input buttons as pressed.
    pub fn input_press(&mut self, mask: u8) {
        self.mmu.input_press(mask);
    }

    /// Marks one or more input buttons as released.
    pub fn input_release(&mut self, mask: u8) {
        self.mmu.input_release(mask);
    }

    /// Toggles between the greenish DMG palette and the color palette.
    pub fn toggle_palette(&mut self) {
        if self.ppu.get_palette() == GREEN_PALETTE {
            self.ppu.set_palette(COLOR_PALETTE);
        } else {
            self.ppu.set_palette(GREEN_PALETTE);
        }
    }
}
