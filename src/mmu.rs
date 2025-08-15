use crate::cartridge::Cartridge;
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
}

impl MMU {
    pub fn new(cartridge: Cartridge) -> Self {
        let mut mmu = Self {
            rom: cartridge.rom.clone().try_into().expect("incorrect ROM size"),
            vram: [0; 0x2000],
            eram: [0; 0x2000],
            wram: [0; 0x2000],
            oam:  [0; 0xA0],
            hram: [0; 0x7F],
            io:   [0; 0x80],
            ie: 0,
            buttons: 0,
        };

        // Post-BIOS initialization
        mmu.write_byte(0xFF00, 0xCF); // P1
        mmu.write_byte(0xFF01, 0x00); // SB
        mmu.write_byte(0xFF02, 0x7E); // SC
        mmu.write_byte(0xFF04, 0xAB); // DIV
        mmu.write_byte(0xFF05, 0x00); // TIMA
        mmu.write_byte(0xFF06, 0x00); // TMA
        mmu.write_byte(0xFF07, 0x00); // TAC
        mmu.write_byte(0xFF0F, 0xE1); // IF
        mmu.write_byte(0xFF10, 0x80);
        mmu.write_byte(0xFF11, 0xBF);
        mmu.write_byte(0xFF12, 0xF3);
        mmu.write_byte(0xFF14, 0xBF);
        mmu.write_byte(0xFF16, 0x3F);
        mmu.write_byte(0xFF17, 0x00);
        mmu.write_byte(0xFF18, 0xFF);
        mmu.write_byte(0xFF19, 0xBF);
        mmu.write_byte(0xFF1A, 0x7F);
        mmu.write_byte(0xFF1B, 0xFF);
        mmu.write_byte(0xFF1C, 0x9F);
        mmu.write_byte(0xFF1E, 0xBF);
        mmu.write_byte(0xFF20, 0xFF);
        mmu.write_byte(0xFF21, 0x00);
        mmu.write_byte(0xFF22, 0x00);
        mmu.write_byte(0xFF23, 0xBF);
        mmu.write_byte(0xFF24, 0x77);
        mmu.write_byte(0xFF25, 0xF3);
        mmu.write_byte(0xFF26, 0xF1);
        // mmu.write_byte(0xFF40, 0x91); // LCDC ON
        mmu.write_byte(0xFF42, 0x00); // SCY
        mmu.write_byte(0xFF43, 0x00); // SCX
        // mmu.write_byte(0xFF44, 0x90); // LY
        mmu.write_byte(0xFF45, 0x00); // LYC
        // mmu.write_byte(0xFF47, 0xE4); // BGP
        mmu.write_byte(0xFF48, 0xFF); // OBP0
        mmu.write_byte(0xFF49, 0xFF); // OBP1
        mmu.write_byte(0xFF4A, 0x00); // WY
        mmu.write_byte(0xFF4B, 0x00); // WX
        mmu.write_byte(0xFFFF, 0x00); // IE
    
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
                    0xFF44 => { self.io[(addr - 0xFF00) as usize] = value; return; }
                    0xFF46 => {
                        // OAM DMA: copy 160 bytes from (value << 8) .. (value << 8) + 0x9F to OAM
                        let src = (value as u16) << 8;
                        for i in 0..0xA0 {
                            let b = self.read_byte(src + i);
                            self.oam[i as usize] = b;
                        }
                    }
                    _ => {}
                }
                self.io[(addr - 0xFF00) as usize] = value;
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
