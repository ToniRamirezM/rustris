use crate::cartridge::Cartridge;
use crate::apu::APU;
use crate::gb::{BTN_RIGHT, BTN_LEFT, BTN_UP, BTN_DOWN, BTN_A, BTN_B, BTN_SELECT, BTN_START};

// MMU: implements the DMG memory map and bus access.
// Responsibilities:
//   - Owns VRAM/WRAM/OAM/HRAM, I/O registers, and IE.
//   - Serves CPU reads/writes and mirrors (e.g., E000–FDFF mirrors C000–DDFF).
//   - Exposes the joypad matrix via P1 (0xFF00).
//   - Performs simple DMA-OAM transfers on writes to 0xFF46.
//   - Applies a post-BIOS register initialization in `new()`.

pub struct MMU {
    rom: [u8; 0x8000],  // 32KB ROM
    vram: [u8; 0x2000], // 8KB VRAM
    eram: [u8; 0x2000], // 8KB ERAM
    wram: [u8; 0x2000], // 8KB WRAM
    oam: [u8; 0xA0],    // 160 bytes Object Attribute Memory
    io: [u8; 0x80],     // 128 bytes IO registers
    hram: [u8; 0x7F],   // 127 bytes HRAM
    ie: u8,             // Interrupt Enable
    buttons: u8,        // Input buttons
    pub apu: APU,
}

impl MMU {
    pub fn new(cartridge: Cartridge, apu: APU) -> Self {
        let mmu = Self {
            rom: cartridge.rom.clone().try_into().expect("incorrect ROM size"),
            vram: [0; 0x2000],
            eram: [0; 0x2000],
            wram: [0; 0x2000],
            oam:  [0; 0xA0],
            hram: [0; 0x7F],
            io:   [0; 0x80],
            ie: 0,
            buttons: 0,
            apu
        };

        mmu
    }

    pub fn read_byte(&self, addr: u16) -> u8 {
        match addr {
            0xFF00 => {
                let p1 = self.io[0x00];
                let sel_buttons = (p1 & 0b0010_0000) == 0; // P15=0
                let sel_dpad    = (p1 & 0b0001_0000) == 0; // P14=0

                let mut low = 0b0000_1111; // default: no buttons pressed

                match (sel_buttons, sel_dpad) {
                    (true, false) => {
                        // Buttons only (A B Select Start) -> bits 0..3
                        if (self.buttons & BTN_A)      != 0 { low &= !0b0001; }
                        if (self.buttons & BTN_B)      != 0 { low &= !0b0010; }
                        if (self.buttons & BTN_SELECT) != 0 { low &= !0b0100; }
                        if (self.buttons & BTN_START)  != 0 { low &= !0b1000; }
                    }
                    (false, true) => {
                        // D-pad only (Right Left Up Down) -> bits 0..3
                        if (self.buttons & BTN_RIGHT) != 0 { low &= !0b0001; } // bit0 = Right
                        if (self.buttons & BTN_LEFT)  != 0 { low &= !0b0010; } // bit1 = Left
                        if (self.buttons & BTN_UP)    != 0 { low &= !0b0100; } // bit2 = Up
                        if (self.buttons & BTN_DOWN)  != 0 { low &= !0b1000; } // bit3 = Down
                    }
                    _ => {
                        // Neither or both groups selected: don't mix nibbles.
                        // Keep low at 0x0F (no press)
                    }
                }

                (p1 & 0b0011_0000) | 0b1100_0000 | low
            }

            0xFF04 => {
                // DIV (Divider register = upper 8 bits of an internal 16-bit counter).
                // We return a random byte instead of emulating the divider/timers.
                // Proper behavior: DIV = (divider >> 8), increments at ~16,384 Hz (every 256 T-cycles),
                // and writing to FF04 resets it to 0, as implemented in write_byte.
                let mut rng = rand::rng();
                rand::Rng::random(&mut rng)
            }

            0x0000..=0x7FFF => self.rom[addr as usize],
            0x8000..=0x9FFF => self.vram[(addr - 0x8000) as usize],
            0xA000..=0xBFFF => self.eram[(addr - 0xA000) as usize],
            0xC000..=0xDFFF => self.wram[(addr - 0xC000) as usize],
            0xE000..=0xFDFF => self.wram[(addr - 0xE000) as usize],
            0xFE00..=0xFE9F => self.oam[(addr - 0xFE00) as usize],
            0xFEA0..=0xFEFF => 0xFF,
            0xFF00..=0xFF7F => self.io[(addr - 0xFF00) as usize],
            0xFF80..=0xFFFE => self.hram[(addr - 0xFF80) as usize],
            0xFFFF => self.ie,
        }
    }

    pub fn write_byte(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x7FFF => {}
            0x8000..=0x9FFF => self.vram[(addr - 0x8000) as usize] = value,
            0xA000..=0xBFFF => self.eram[(addr - 0xA000) as usize] = value,
            0xC000..=0xDFFF => self.wram[(addr - 0xC000) as usize] = value,
            0xE000..=0xFDFF => self.wram[(addr - 0xE000) as usize] = value,
            0xFE00..=0xFE9F => self.oam[(addr - 0xFE00) as usize] = value,
            0xFEA0..=0xFEFF => {}
            0xFF00..=0xFF7F => {
                match addr {
                    0xFF00 => {
                        // Bits 4–5 select group (0 = selected). Bits 6–7 are always 1.
                        let cur = self.io[0x00];
                        let newp1 = (cur & 0b1100_1111) | (value & 0b0011_0000) | 0b1100_0000;
                        self.io[0x00] = newp1;
                        return;
                    }
                    0xFF04 => { self.io[(addr - 0xFF00) as usize] = 0; return; }
                    0xFF46 => {
                        // OAM DMA: copy 160 bytes from (value << 8) .. (value << 8) + 0x9F to OAM
                        let src = (value as u16) << 8;
                        for i in 0..0xA0 {
                            let b = self.read_byte(src + i);
                            self.oam[i as usize] = b;
                        }
                    }
                    0xFF26 => {
                        // NR52 — master enable
                        self.io[(addr - 0xFF00) as usize] = value;
                        let enable = (value & 0x80) != 0;
                        self.apu.master_enable(enable);
                        self.apu.write(addr, value);
                        
                        return;
                    }
                    _ => {}
                }
                self.io[(addr - 0xFF00) as usize] = value;
                self.apu.write(addr, value);
            }
            0xFF80..=0xFFFE => self.hram[(addr - 0xFF80) as usize] = value,
            0xFFFF => self.ie = value,
        }
    }

    pub fn input_press(&mut self, mask: u8) {
        // Anti-ghosting for opposite directions
        let mut new = self.buttons | mask;
        if (new & BTN_RIGHT) != 0 { new &= !BTN_LEFT; }
        if (new & BTN_LEFT)  != 0 { new &= !BTN_RIGHT; }
        if (new & BTN_UP)    != 0 { new &= !BTN_DOWN; }
        if (new & BTN_DOWN)  != 0 { new &= !BTN_UP; }

        self.buttons = new;
    }

    pub fn input_release(&mut self, mask: u8) {
        self.buttons &= !mask;
    }
}
