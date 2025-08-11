use crate::cpu::CPU;
use crate::cartridge::Cartridge;
use crate::mmu::MMU;
use crate::ppu::{GREEN_PALETTE, COLOR_PALETTE, PPU};

/// GB façade: ties together the CPU, MMU (bus/memory), and PPU.
/// Responsibilities:
///   - Bootstraps the system (MMU with the cartridge, CPU/PPU instances).
///   - Runs one CPU instruction per call to `step`, then advances the PPU
///     by the instruction’s T-cycle count.
///   - Exposes input methods that forward button state to the MMU.
///   - Provides a linear RGB framebuffer; when the PPU finishes a frame,
///     it is copied out and `step` returns `true`.
///
/// Timing contract:
///   - `CPU::step` returns T-cycles consumed by the executed instruction.
///   - `PPU::step(mmu, t_cycles)` advances video timing by that amount.
///   - `is_frame_ready()` indicates a complete frame since the last call.


pub const BTN_RIGHT:  u8 = 1 << 0;
pub const BTN_LEFT:   u8 = 1 << 1;
pub const BTN_UP:     u8 = 1 << 2;
pub const BTN_DOWN:   u8 = 1 << 3;
pub const BTN_A:      u8 = 1 << 4;
pub const BTN_B:      u8 = 1 << 5;
pub const BTN_SELECT: u8 = 1 << 6;
pub const BTN_START:  u8 = 1 << 7;

/// High-level Game Boy wrapper that orchestrates CPU, MMU, and PPU.
pub struct GB {
    cpu: CPU,
    mmu: MMU,
    ppu: PPU,
}

impl GB {
    pub fn new(cartridge: Cartridge) -> Self {
        let mmu = MMU::new(cartridge);

        GB {
            cpu: CPU::new(),
            mmu,
            ppu: PPU::new(),
        }
    }

    /// Execute one CPU instruction and advance the PPU accordingly.
    /// Returns `true` if a new frame has been rendered into `framebuffer`
    /// so the SDL layer can present it.
    pub fn step(&mut self, framebuffer: &mut [u8]) -> bool {
        let t = self.cpu.step(&mut self.mmu);
        self.ppu.step(&mut self.mmu, t);

        if self.ppu.is_frame_ready() {
            framebuffer.copy_from_slice(&self.ppu.fb);
            return true;
        }

        false
    }

    pub fn input_press(&mut self, mask: u8) {
        self.mmu.input_press(mask);
    }

    pub fn input_release(&mut self, mask: u8) {
        self.mmu.input_release(mask);
    }

    pub fn toggle_palette(&mut self) {
        if self.ppu.get_palette() == GREEN_PALETTE {
            self.ppu.set_palette(COLOR_PALETTE);
        } else {
            self.ppu.set_palette(GREEN_PALETTE);
        }
    }
}
