use crate::mmu::MMU;

/// CPU core: fetch–decode–execute loop for the Game Boy CPU (Sharp LR35902).
/// Responsibilities:
///   - Holds all CPU registers and flags (AF, BC, DE, HL, SP, PC).
///   - Implements instruction decoding/execution and flag updates.
///   - Handles IME/EI timing, interrupt pending/service logic, and cycle counts.
/// Conventions:
///   - Little-endian immediate fetch (lo then hi).
///   - Flag register F uses bits: Z(7) N(6) H(5) C(4); lower 4 bits are always zero.
///   - Cycle counts returned by opcode handlers include memory access cost.

pub struct CPU {
    pc: u16, // Program Counter
    sp: u16, // Stack Pointer

    // 8-bit registers (AF, BC, DE, HL as pairs)
    a: u8,
    f: u8, // flags: Z N H C in bits 7..4
    b: u8,
    c: u8,
    d: u8,
    e: u8,
    h: u8,
    l: u8,

    // Interrupt state
    ei_pending: bool, // EI takes effect after the next instruction
    ime: bool, // master interrupt enable
}

impl CPU {
    /// Create a CPU with the post-BIOS state.
    pub fn new() -> Self {
        CPU {
            pc: 0x0100,
            sp: 0xFFFE,
            a: 0x01,
            f: 0xB0,
            b: 0x00,
            c: 0x13,
            d: 0x00,
            e: 0xD8,
            h: 0x01,
            l: 0x4D,
            ei_pending: false,
            ime: false,
        }
    }

    /// Execute one CPU step:
    /// - If IME is set and a VBlank interrupt (IE&IF bit 0) is pending, service it
    ///   immediately (push PC, clear IF.VBlank, IME=0, jump to 0x0040) and return 20 T-cycles.
    /// - Otherwise fetch–decode–execute one opcode at PC and return its T-cycle cost.
    /// - EI takes effect after the *next* instruction (delayed IME enable).
    /// Notes: 1 M-cycle = 4 T-cycles. This is a Tetris-only fast path (VBlank only).
    pub fn step(&mut self, mmu: &mut MMU) -> u32 {
        if self.ime && self.vblank_pending(mmu) {
            let t = self.service_interrupt(mmu);
            return t;
        }

        let t = self.opcode(mmu);

        if self.ei_pending {
            self.ime = true;
            self.ei_pending = false;
        }
        
        t
    }

    /// Fetch–decode–execute a single opcode at PC.
    /// Each opcode returns the number of t-cycles consumed.
    fn opcode(&mut self, memory: &mut MMU) -> u32 {
        let opcode = memory.read_byte(self.pc);
        self.pc = self.pc.wrapping_add(1);

        match opcode {
            0x00 => {
                // NOP
                4
            }

            0x01 => {
                // LD BC,d16
                let val = self.fetch_u16(memory);
                self.set_bc(val);
                12
            }

            0x02 => {
                // LD (BC),A
                memory.write_byte(self.get_bc(), self.a);
                8
            }

            0x03 => { 
                // INC BC
                let val = self.get_bc().wrapping_add(1);
                self.set_bc(val);
                8
            }

            0x04 => { 
                // INC B
                let old_b = self.b;
                self.b = self.b.wrapping_add(1);
                self.set_flag_z(self.b == 0);
                self.set_flag_n(false);
                self.set_flag_h((old_b & 0x0F) == 0x0F);
                4
            }

            0x05 => {
                // DEC B
                self.b = self.b.wrapping_sub(1);
                self.set_flag_z(self.b == 0);
                self.set_flag_n(true);
                self.set_flag_h((self.b & 0x0F) == 0x0F);
                4
            }

            0x06 => {
                // LD B,d8
                let val = self.fetch_u8(memory);
                self.b = val;
                8
            }

            0x07 => { 
                // RLCA
                let carry = (self.a & 0x80) != 0;
                self.a = self.a.rotate_left(1);
                self.set_flag_z(false);
                self.set_flag_n(false);
                self.set_flag_h(false);
                self.set_flag_c(carry);
                4
            }

            0x09 => {
                // ADD HL,BC
                let hl = self.get_hl();
                let bc = self.get_bc();
                let res = hl.wrapping_add(bc);
                self.set_flag_n(false);
                self.set_flag_h(((hl & 0x0FFF) + (bc & 0x0FFF)) > 0x0FFF);
                self.set_flag_c(hl > 0xFFFF - bc);
                self.set_hl(res);
                8
            }

            0x0A => { 
                // LD A,(BC)
                self.a = memory.read_byte(self.get_bc());
                8
            }

            0x0B => {
                // DEC BC
                let val = self.get_bc().wrapping_sub(1);
                self.set_bc(val);
                8
            }

            0x0C => {
                // INC C    
                let old_val = self.c;
                self.c = self.c.wrapping_add(1);
                
                self.set_flag_z(self.c == 0);
                self.set_flag_n(false);
                self.set_flag_h((old_val & 0x0F) == 0x0F);
                4
            }

            0x0D => {
                // DEC C
                self.c = self.c.wrapping_sub(1);
                self.set_flag_z(self.c == 0);
                self.set_flag_n(true);
                self.set_flag_h((self.c & 0x0F) == 0x0F);
                4
            }

            0x0E => {
                // LD C,d8
                let val = self.fetch_u8(memory);
                self.c = val;
                8
            }

            0x11 => {
                // LD DE,d16
                let val = self.fetch_u16(memory);
                self.set_de(val);
                12
            }

            0x12 => {
                // LD (DE),A
                memory.write_byte(self.get_de(), self.a);
                8
            }

            0x13 => {
                // INC DE
                let val = self.get_de().wrapping_add(1);
                self.set_de(val);
                8
            }

            0x16 => {
                // LD D,d8
                let val = self.fetch_u8(memory);
                self.d = val;
                8
            }

            0x18 => {
                // JR r8
                let offset = self.fetch_u8(memory) as i8 as i16;
                self.pc = ((self.pc as i16).wrapping_add(offset)) as u16;
                12
            }

            0x19 => {
                // ADD HL,DE
                let hl = self.get_hl();
                let de = self.get_de();
                let res = hl.wrapping_add(de);
                self.set_flag_n(false);
                self.set_flag_h(((hl & 0x0FFF) + (de & 0x0FFF)) > 0x0FFF);
                self.set_flag_c(hl > 0xFFFF - de);
                self.set_hl(res);
                8
            }

            0x1A => {
                // LD A,(DE)
                self.a = memory.read_byte(self.get_de());
                8
            }

            0x1B => {
                // DEC DE
                let val = self.get_de().wrapping_sub(1);
                self.set_de(val);
                8
            }

            0x1C => {
                // INC E
                let old_e = self.e;
                self.e = self.e.wrapping_add(1);
                self.set_flag_z(self.e == 0);
                self.set_flag_n(false);
                self.set_flag_h((old_e & 0x0F) == 0x0F);
                4
            }

            0x1D => {
                // DEC E
                self.e = self.e.wrapping_sub(1);
                self.set_flag_z(self.e == 0);
                self.set_flag_n(true);
                self.set_flag_h((self.e & 0x0F) == 0x0F);
                4
            }

            0x1E => { 
                // LD E,d8
                let val = self.fetch_u8(memory);
                self.e = val;
                8
            }

            0x20 => {
                // JR NZ,r8
                let offset = self.fetch_u8(memory) as i8 as i16;
                if !self.get_flag_z() {
                    self.pc = ((self.pc as i16).wrapping_add(offset)) as u16;
                    12
                } else {
                    8
                }
            }

            0x21 => {
                // LD HL,d16
                let val = self.fetch_u16(memory);
                self.set_hl(val);
                12
            }

            0x22 => {
                // LD (HL+),A
                let hl = self.get_hl();
                memory.write_byte(hl, self.a);
                self.set_hl(hl.wrapping_add(1));
                8
            }

            0x23 => {
                // INC HL
                let val = self.get_hl().wrapping_add(1);
                self.set_hl(val);
                8
            }

            0x25 => {
                // DEC H
                self.h = self.h.wrapping_sub(1);
                self.set_flag_z(self.h == 0);
                self.set_flag_n(true);
                self.set_flag_h((self.h & 0x0F) == 0x0F);
                4
            }

            0x26 => { 
                // LD H,d8
                let val = self.fetch_u8(memory);
                self.h = val;
                8
            }

            0x27 => {
                // DAA (Decimal Adjust Accumulator)
                let mut a = self.a;
                let mut adjust = 0;
                let mut carry = self.get_flag_c();

                if !self.get_flag_n() {
                    if self.get_flag_h() || (a & 0x0F) > 0x09 {
                        adjust |= 0x06;
                    }
                    if carry || a > 0x99 {
                        adjust |= 0x60;
                        carry = true;
                    }
                    a = a.wrapping_add(adjust);
                } else {
                    if self.get_flag_h() {
                        adjust |= 0x06;
                    }
                    if carry {
                        adjust |= 0x60;
                    }
                    a = a.wrapping_sub(adjust);
                }

                self.a = a;
                self.set_flag_z(self.a == 0);
                self.set_flag_h(false);
                self.set_flag_c(carry);
                4
            }

            0x28 => {
                // JR Z,r8
                let offset = self.fetch_u8(memory) as i8 as i16;
                if self.get_flag_z() {
                    self.pc = ((self.pc as i16).wrapping_add(offset)) as u16;
                    12
                } else {
                    8
                }
            }

            0x2A => {
                // LD A,(HL+)
                let hl = self.get_hl();
                self.a = memory.read_byte(hl);
                self.set_hl(hl.wrapping_add(1));
                8
            }

            0x2B => {
                // DEC HL
                let val = self.get_hl().wrapping_sub(1);
                self.set_hl(val);
                8
            }

            0x2C => {
                // INC L
                self.l = self.l.wrapping_add(1);
                self.set_flag_z(self.l == 0);
                self.set_flag_n(false);
                self.set_flag_h((self.l & 0x0F) == 0);
                4
            }

            0x2D => { 
                // DEC L
                self.l = self.l.wrapping_sub(1);
                self.set_flag_z(self.l == 0);
                self.set_flag_n(true);
                self.set_flag_h((self.l & 0x0F) == 0x0F);
                4
            }

            0x2E => { 
                // LD L,d8
                let v = self.fetch_u8(memory);
                self.l = v;
                8
            }

            0x2F => {
                // CPL
                self.a = !self.a;
                self.set_flag_n(true);
                self.set_flag_h(true);
                4
            }

            0x30 => {
                // JR NC,r8
                let offset = self.fetch_u8(memory) as i8 as i16;
                if !self.get_flag_c() {
                    self.pc = ((self.pc as i16).wrapping_add(offset)) as u16;
                    12
                } else {
                    8
                }
            }

            0x31 => {
                // LD SP,d16
                let val = self.fetch_u16(memory);
                self.sp = val;
                12
            }

            0x32 => {
                // LD (HL-),A
                let hl = self.get_hl();
                memory.write_byte(hl, self.a);
                self.set_hl(hl.wrapping_sub(1));
                8
            }

            0x34 => {
                // INC (HL)
                let addr = self.get_hl();
                let val = memory.read_byte(addr);
                let res = val.wrapping_add(1);
                memory.write_byte(addr, res);
                self.set_flag_z(res == 0);
                self.set_flag_n(false);
                self.set_flag_h((val & 0x0F) + 1 > 0x0F);
                12
            }

            0x35 => {
                // DEC (HL)
                let addr = self.get_hl();
                let value = memory.read_byte(addr);
                let result = value.wrapping_sub(1);
                memory.write_byte(addr, result);
                self.set_flag_z(result == 0);
                self.set_flag_n(true);
                self.set_flag_h((value & 0x0F) == 0x00);
                12
            }

            0x36 => {
                // LD (HL),d8
                let val = self.fetch_u8(memory);
                memory.write_byte(self.get_hl(), val);
                12
            }

            0x38 => {
                // JR C,r8
                let offset = self.fetch_u8(memory) as i8 as i16;
                if self.get_flag_c() {
                    self.pc = ((self.pc as i16).wrapping_add(offset)) as u16;
                    12
                } else {
                    8
                }
            }

            0x3C => {
                // INC A
                let val = self.a;
                self.a = self.a.wrapping_add(1);
                self.set_flag_z(self.a == 0);
                self.set_flag_n(false);
                self.set_flag_h((val & 0x0F) + 1 > 0x0F);
                4
            }

            0x3A => { 
                // LD A,(HL-)
                let hl = self.get_hl();
                self.a = memory.read_byte(hl);
                self.set_hl(hl.wrapping_sub(1));
                8
            }

            0x3D => {
                // DEC A
                self.a = self.a.wrapping_sub(1);
                self.set_flag_z(self.a == 0);
                self.set_flag_n(true);
                self.set_flag_h((self.a & 0x0F) == 0x0F);
                4
            }

            0x3E => {
                // LD A,d8
                let val = self.fetch_u8(memory);
                self.a = val;
                8
            }

            0x40 => { 
                // LD B,B
                4
            }

            0x46 => {
                let addr = self.get_hl();
                self.b = memory.read_byte(addr);
                8
            }

            0x47 => {
                // LD B,A
                self.b = self.a;
                4
            }

            0x4E => {
                let addr = self.get_hl();
                self.c = memory.read_byte(addr);
                8
            }

            0x4F => {
                // LD C,A
                self.c = self.a;
                4
            }

            0x54 => { 
                // LD D,H
                self.d = self.h;
                4
            }

            0x56 => {
                // LD D,(HL)
                let addr = self.get_hl();
                self.d = memory.read_byte(addr);
                8
            }

            0x57 => { 
                // LD D,A
                self.d = self.a;
                4
            }

            0x5D => { 
                // LD E,L
                self.e = self.l;
                4
            }

            0x5E => {
                // LD E,(HL)
                let addr = self.get_hl();
                self.e = memory.read_byte(addr);
                8
            }

            0x5F => {
                // LD E,A
                self.e = self.a;
                4
            }

            0x60 => { 
                // LD H,B
                self.h = self.b;
                4
            }

            0x61 => { 
                // LD H,C
                self.h = self.c;
                4
            }

            0x62 => { 
                // LD H,D
                self.h = self.d;
                4
            }

            0x67 => { 
                // LD H,A
                self.h = self.a;
                4
            }

            0x69 => { 
                // LD L, C
                self.l = self.c;
                4
            }

            0x6B => { 
                // LD L,E
                self.l = self.e;
                4
            }

            0x6F => { 
                // LD L,A
                self.l = self.a;
                4
            }

            0x70 => {
                // LD (HL),B
                let addr = self.get_hl();
                memory.write_byte(addr, self.b);
                8
            }

            0x71 => { 
                // LD (HL),C
                memory.write_byte(self.get_hl(), self.c);
                8
            }

            0x72 => { 
                // LD (HL),D
                memory.write_byte(self.get_hl(), self.d);
                8
            }

            0x73 => { 
                // LD (HL),E
                memory.write_byte(self.get_hl(), self.e);
                8
            }

            0x77 => {
                // LD (HL),A
                memory.write_byte(self.get_hl(), self.a);
                8
            }

            0x78 => {
                // LD A,B
                self.a = self.b;
                4
            }

            0x79 => {
                // LD A,C
                self.a = self.c;
                4
            }

            0x7A => { 
                // LD A,D
                self.a = self.d;
                4
            }

            0x7B => { // LD A,E
                self.a = self.e;
                4
            }

            0x7C => {
                // LD A,H
                self.a = self.h;
                4
            }

            0x7D => { 
                // LD A,L
                self.a = self.l;
                4
            }

            0x7E => {
                // LD A,(HL)
                self.a = memory.read_byte(self.get_hl());
                8
            }

            0x80 => {
                // ADD A,B
                self.a = self.add8(self.a, self.b, false);
                4
            }

            0x82 => {
                // ADD A,D
                self.a = self.add8(self.a, self.d, false);
                4
            }

            0x83 => {
                // ADD A,E
                self.a = self.add8(self.a, self.e, false);
                4
            }

            0x85 => {
                // ADD A,L
                self.a = self.add8(self.a, self.l, false);
                4
            }

            0x86 => {
                // ADD A,(HL)
                let val = memory.read_byte(self.get_hl());
                self.a = self.add8(self.a, val, false);
                8
            }

            0x87 => {
                // ADD A,A
                let a = self.a;
                self.a = self.add8(a, a, false);
                4
            }

            0x89 => { 
                // ADC A,C
                self.a = self.add8(self.a, self.c, self.get_flag_c());
                4
            }

            0x8E => {
                // ADC A,(HL)
                let val = memory.read_byte(self.get_hl());
                self.a = self.add8(self.a, val, self.get_flag_c());
                8
            }

            0x90 => {
                // SUB B
                self.a = self.sub8(self.a, self.b, false);
                4
            }

            0x96 => {
                // SUB (HL)
                let val = memory.read_byte(self.get_hl());
                self.a = self.sub8(self.a, val, false);
                8
            }

            0xA0 => {
                // AND B
                self.a &= self.b;
                self.set_flag_z(self.a == 0);
                self.set_flag_n(false);
                self.set_flag_h(true);
                self.set_flag_c(false);
                4
            }

            0xA1 => {
                // AND C
                self.a &= self.c;
                self.set_flag_z(self.a == 0);
                self.set_flag_n(false);
                self.set_flag_h(true);
                self.set_flag_c(false);
                4
            }

            0xA7 => {
                // AND A
                self.a &= self.a;
                self.set_flag_z(self.a == 0);
                self.set_flag_n(false);
                self.set_flag_h(true);
                self.set_flag_c(false);
                4
            }

            0xA8 => {
                // XOR B
                self.a ^= self.b;
                self.set_flag_z(self.a == 0);
                self.set_flag_n(false);
                self.set_flag_h(false);
                self.set_flag_c(false);
                4
            }

            0xA9 => {
                // XOR C
                self.a ^= self.c;
                self.set_flag_z(self.a == 0);
                self.set_flag_n(false);
                self.set_flag_h(false);
                self.set_flag_c(false);
                4
            }

            0xAF => {
                // XOR A
                self.a = 0;
                self.set_flag_z(self.a == 0);
                self.set_flag_n(false);
                self.set_flag_h(false);
                self.set_flag_c(false);
                4
            }

            0xB0 => {
                // OR B
                self.a |= self.b;
                self.set_flag_z(self.a == 0);
                self.set_flag_n(false);
                self.set_flag_h(false);
                self.set_flag_c(false);
                4
            }

            0xB1 => {
                // OR C
                self.a |= self.c;
                self.set_flag_z(self.a == 0);
                self.set_flag_n(false);
                self.set_flag_h(false);
                self.set_flag_c(false);
                4
            }

            0xB2 => {
                // OR D
                self.a |= self.d;
                self.set_flag_z(self.a == 0);
                self.set_flag_n(false);
                self.set_flag_h(false);
                self.set_flag_c(false);
                4
            }

            0xB7 => {
                // OR A
                self.a |= self.a;
                self.set_flag_z(self.a == 0);
                self.set_flag_n(false);
                self.set_flag_h(false);
                self.set_flag_c(false);
                4
            }

            0xB8 => {
                // CP B
                let res = self.a.wrapping_sub(self.b);
                self.set_flag_z(res == 0);
                self.set_flag_n(true);
                self.set_flag_h((self.a & 0x0F) < (self.b & 0x0F));
                self.set_flag_c(self.a < self.b);
                4
            }

            0xB9 => {
                // CP C
                let res = self.a.wrapping_sub(self.c);
                self.set_flag_z(res == 0);
                self.set_flag_n(true);
                self.set_flag_h((self.a & 0x0F) < (self.c & 0x0F));
                self.set_flag_c(self.a < self.c);
                4
            }

            0xBE => {
                // CP (HL)
                let val = memory.read_byte(self.get_hl());
                let res = self.a.wrapping_sub(val);
                self.set_flag_z(res == 0);
                self.set_flag_n(true);
                self.set_flag_h((self.a & 0x0F) < (val & 0x0F));
                self.set_flag_c(self.a < val);
                8
            }

            0xC2 => { 
                // JP NZ,nn
                let addr = self.fetch_u16(memory);
                if !self.get_flag_z() {
                    self.pc = addr;
                    16
                } else {
                    12
                }
            }

            0xCA => {
                // JP Z,nn
                let addr = self.fetch_u16(memory);
                if self.get_flag_z() {
                    self.pc = addr;
                    16
                } else {
                    12
                }
            }

            0xC0 => {
                // RET NZ
                if !self.get_flag_z() {
                    let addr = self.pop(memory);
                    self.pc = addr;
                    20
                } else {
                    8
                }
            }

            0xC1 => {
                // POP BC
                let (b, c) = self.pop_reg_pair(memory);
                self.b = b;
                self.c = c;
                12
            }   
  
            0xC3 => {
                // JP nn
                let addr = self.fetch_u16(memory);
                self.pc = addr;
                16
            }

            0xC5 => {
                // PUSH BC
                self.push_reg_pair(memory, self.b, self.c);
                16
            }

            0xC6 => { 
                // ADD A,d8
                let value = self.fetch_u8(memory);
                self.a = self.add8(self.a, value, false);
                8
            }

            0xC8 => {
                // RET Z
                if self.get_flag_z() {
                    let addr = self.pop(memory);
                    self.pc = addr;
                    20
                } else {
                    8
                }
            }

            0xC9 => {
                // RET
                let addr = self.pop(memory);
                self.pc = addr;
                16
            }

            0xCD => {
                // CALL nn
                let addr = self.fetch_u16(memory);
                self.push(memory, self.pc);
                self.pc = addr;
                24
            }

            0xD0 => {
                // RET NC
                if !self.get_flag_c() {
                    let addr = self.pop(memory);
                    self.pc = addr;
                    20
                } else {
                    8
                }
            }

            0xD1 => {
                // POP DE
                let (d, e) = self.pop_reg_pair(memory);
                self.d = d;
                self.e = e;
                12
            }

            0xD5 => {
                // PUSH DE
                self.push_reg_pair(memory, self.d, self.e);
                16
            }

            0xD6 => {
                // SUB A, n
                let value = self.fetch_u8(memory);
                let a = self.a;
                let result = a.wrapping_sub(value);
                self.a = result;
                self.set_flag_z(result == 0);
                self.set_flag_n(true);
                self.set_flag_h((a & 0x0F) < (value & 0x0F));
                self.set_flag_c(a < value);
                8
            }

            0xD8 => {
                // RET C
                if self.get_flag_c() {
                    let addr = self.pop(memory);
                    self.pc = addr;
                    20
                } else {
                    8
                }
            }

            0xD9 => {
                // RETI
                let addr = self.pop(memory);
                self.pc = addr;
                self.ime = true;
                16
            }

            0xE0 => {
                // LDH (n),A
                let offset = self.fetch_u8(memory) as u16;
                memory.write_byte(0xFF00 | offset, self.a);
                12
            }

            0xE1 => {
                // POP HL
                let (h, l) = self.pop_reg_pair(memory);
                self.h = h;
                self.l = l;
                12
            }

            0xE2 => {
                // LD (FF00+C),A
                let addr = 0xFF00u16 + self.c as u16;
                memory.write_byte(addr, self.a);
                8
            }

            0xE5 => {
                // PUSH HL
                self.push_reg_pair(memory, self.h, self.l);
                16
            }

            0xE6 => {
                // AND d8
                let val = self.fetch_u8(memory);
                self.a &= val;
                self.set_flag_z(self.a == 0);
                self.set_flag_n(false);
                self.set_flag_h(true);
                self.set_flag_c(false);
                8
            }

            0xE9 => {
                // JP (HL)
                self.pc = self.get_hl();
                4
            }

            0xEA => {
                // LD (nn),A
                let addr = self.fetch_u16(memory);
                memory.write_byte(addr, self.a);
                16
            }

            0xEE => { 
                // XOR d8
                let val = self.fetch_u8(memory);
                self.a ^= val;
                self.set_flag_z(self.a == 0);
                self.set_flag_n(false);
                self.set_flag_h(false);
                self.set_flag_c(false);
                8
            }

            0xEF => {
                // RST 28H
                self.push(memory, self.pc);
                self.pc = 0x28;
                16
            }

            0xF1 => {
                // POP AF
                self.pop_af(memory);
                12
            }

            0xF6 => { 
                // OR d8
                let val = self.fetch_u8(memory);
                self.a |= val;
                self.set_flag_z(self.a == 0);
                self.set_flag_n(false);
                self.set_flag_h(false);
                self.set_flag_c(false);
                8
            }

            0xFB => {
                // EI (Enable Interrupts)
                self.ei_pending = true; // IME will be enabled on next instruction
                4
            }

            0xF0 => {
                // LD A,(FF00+n)
                let offset = self.fetch_u8(memory) as u16;
                self.a = memory.read_byte(0xFF00 | offset);
                12
            }

            0xF3 => {
                // DI
                self.ime = false;
                4
            }

            0xF5 => {
                // PUSH AF
                self.push_af(memory);
                16
            }

            0xFA => {
                // LD A,(nn)
                let addr = self.fetch_u16(memory);
                self.a = memory.read_byte(addr);
                16
            }

            0xFE => {
                // CP d8
                let val = self.fetch_u8(memory);
                let res = self.a.wrapping_sub(val);
                self.set_flag_z(res == 0);
                self.set_flag_n(true);
                self.set_flag_h((self.a & 0x0F) < (val & 0x0F));
                self.set_flag_c(self.a < val);
                8
            }

            0xCB => {
                // PREFIX CB
                let cb_opcode = self.fetch_u8(memory);
                match cb_opcode {
                    0x27 => {
                        // SLA A
                        let carry = (self.a & 0x80) != 0;
                        self.a <<= 1;
                        self.set_flag_z(self.a == 0);
                        self.set_flag_n(false);
                        self.set_flag_h(false);
                        self.set_flag_c(carry);
                        8
                    }

                    0x37 => {
                        // SWAP A
                        self.a = (self.a >> 4) | (self.a << 4);
                        self.set_flag_z(self.a == 0);
                        self.set_flag_n(false);
                        self.set_flag_h(false);
                        self.set_flag_c(false);
                        8
                    }

                    0x3F => {
                        // SRL A
                        let carry = self.a & 0x01 != 0;
                        self.a >>= 1;
                        self.set_flag_z(self.a == 0);
                        self.set_flag_n(false);
                        self.set_flag_h(false);
                        self.set_flag_c(carry);
                        8
                    }

                    0x40 => { 
                        // BIT 0,B
                        self.set_flag_z((self.b & (1 << 0)) == 0);
                        self.set_flag_n(false);
                        self.set_flag_h(true);
                        8
                    }

                    0x41 => { 
                        // BIT 0,C
                        self.set_flag_z((self.c & (1 << 0)) == 0);
                        self.set_flag_n(false);
                        self.set_flag_h(true);
                        8
                    }

                    0x47 => { 
                        // BIT 0,A
                        self.set_flag_z((self.a & (1 << 0)) == 0);
                        self.set_flag_n(false);
                        self.set_flag_h(true);
                        8
                    }

                    0x48 => { 
                        // BIT 1,B
                        self.set_flag_z((self.b & (1 << 1)) == 0);
                        self.set_flag_n(false);
                        self.set_flag_h(true);
                        8
                    }

                    0x50 => { 
                        // BIT 2,B
                        self.set_flag_z((self.b & (1 << 2)) == 0);
                        self.set_flag_n(false);
                        self.set_flag_h(true);
                        8
                    }

                    0x57 => { 
                        // BIT 2,A
                        self.set_flag_z((self.a & (1 << 2)) == 0);
                        self.set_flag_n(false);
                        self.set_flag_h(true);
                        8
                    }

                    0x58 => { 
                        // BIT 3,B
                        self.set_flag_z((self.b & (1 << 3)) == 0);
                        self.set_flag_n(false);
                        self.set_flag_h(true);
                        8
                    }

                    0x5F => {
                        // BIT 3,A
                        let bit = (self.a >> 3) & 1;
                        self.set_flag_z(bit == 0);
                        self.set_flag_n(false);
                        self.set_flag_h(true);
                        8
                    }

                    0x60 => { 
                        // BIT 4,B
                        self.set_flag_z((self.b & (1 << 4)) == 0);
                        self.set_flag_n(false);
                        self.set_flag_h(true);
                        8
                    }

                    0x61 => { 
                        // BIT 4,C
                        self.set_flag_z((self.c & (1 << 4)) == 0);
                        self.set_flag_n(false);
                        self.set_flag_h(true);
                        8
                    }

                    0x68 => { 
                        // BIT 5,B
                        self.set_flag_z((self.b & (1 << 5)) == 0);
                        self.set_flag_n(false);
                        self.set_flag_h(true);
                        8
                    }

                    0x69 => { 
                        // BIT 5,C
                        self.set_flag_z((self.c & (1 << 5)) == 0);
                        self.set_flag_n(false);
                        self.set_flag_h(true);
                        8
                    }

                    0x6F => { 
                        // BIT 5,A
                        self.set_flag_z((self.a & (1 << 5)) == 0);
                        self.set_flag_n(false);
                        self.set_flag_h(true);
                        8
                    }

                    0x70 => { 
                        // BIT 6,B
                        self.set_flag_z((self.b & (1 << 6)) == 0);
                        self.set_flag_n(false);
                        self.set_flag_h(true);
                        8
                    }

                    0x71 => {
                        // BIT 6,C
                        let bit = (self.c >> 6) & 1;
                        self.set_flag_z(bit == 0);
                        self.set_flag_n(false);
                        self.set_flag_h(true);
                        8
                    }

                    0x77 => { 
                        // BIT 6,A
                        self.set_flag_z((self.a & (1 << 6)) == 0);
                        self.set_flag_n(false);
                        self.set_flag_h(true);
                        8
                    }

                    0x78 => { 
                        // BIT 7,B
                        self.set_flag_z((self.b & (1 << 7)) == 0);
                        self.set_flag_n(false);
                        self.set_flag_h(true);
                        8
                    }

                    0x79 => {
                        // BIT 7,C
                        let bit = (self.c >> 7) & 1;
                        self.set_flag_z(bit == 0);
                        self.set_flag_n(false);
                        self.set_flag_h(true);
                        8
                    }

                    0x7E => { 
                        // LD A,(HL)
                        self.a = memory.read_byte(self.get_hl());
                        8
                    }

                    0x7F => {
                        // BIT 7,A
                        let bit_set = (self.a & (1 << 7)) != 0;
                        self.set_flag_z(!bit_set);
                        self.set_flag_n(false);
                        self.set_flag_h(true);
                        8
                    }

                    0x86 => { 
                        // RES 0,(HL)
                        let addr = self.get_hl();
                        let mut val = memory.read_byte(addr);
                        val &= !(1 << 0);
                        memory.write_byte(addr, val);
                        16
                    }

                    0x87 => {
                        // RES 0,A
                        self.a &= !(1 << 0);
                        8
                    }

                    0x9E => {
                        // RES 3,(HL)
                        let addr = self.get_hl();
                        let val = memory.read_byte(addr) & !(1 << 3);
                        memory.write_byte(addr, val);
                        16
                    }

                    0xBE => {
                        // RES 7,(HL)
                        let addr = self.get_hl();
                        let mut val = memory.read_byte(addr);
                        val &= !(1 << 7);
                        memory.write_byte(addr, val);
                        16
                    }

                    0xDE => {
                        // SET 3,(HL)
                        let addr = self.get_hl();
                        let val = memory.read_byte(addr) | (1 << 3);
                        memory.write_byte(addr, val);
                        16
                    }

                    0xFE => {
                        // SET 7, (HL)
                        let addr = self.get_hl();
                        let val = memory.read_byte(addr) | (1 << 7);
                        memory.write_byte(addr, val);
                        16
                    }

                    _ => {
                        eprintln!("Unknown CB opcode: 0x{:02X}", cb_opcode);
                        std::process::exit(1);
                    }
                }
            }  
             _ => {
                eprintln!("Unknown opcode: 0x{:02X}", opcode);
                std::process::exit(1);
            }
        }   
    }

    /// Read an immediate byte at PC (little-endian helper).
    fn fetch_u8(&mut self, mmu: &MMU) -> u8 {
        let b = mmu.read_byte(self.pc);
        self.pc = self.pc.wrapping_add(1);
        b
    }

    /// Read an immediate word at PC: low byte then high byte.
    fn fetch_u16(&mut self, mmu: &MMU) -> u16 {
        let lo = self.fetch_u8(mmu) as u16;
        let hi = self.fetch_u8(mmu) as u16;
        (hi << 8) | lo
    }

    /// 8-bit addition with optional carry-in; updates Z N H C.
    fn add8(&mut self, a: u8, b: u8, carry: bool) -> u8 {
        let c = if carry && self.get_flag_c() { 1 } else { 0 };
        let (s1, c1) = a.overflowing_add(b);
        let (res, c2) = s1.overflowing_add(c);
        self.set_flag_z(res == 0);
        self.set_flag_n(false);
        self.set_flag_h(((a & 0x0F) + (b & 0x0F) + c) & 0x10 != 0);
        self.set_flag_c(c1 || c2);
        res
    }

    /// 8-bit subtraction with optional carry-in.
    fn sub8(&mut self, a: u8, b: u8, carry: bool) -> u8 {
        let c = if carry && self.get_flag_c() { 1 } else { 0 };
        let (s1, b1) = a.overflowing_sub(b);
        let (res, b2) = s1.overflowing_sub(c);
        self.set_flag_z(res == 0);
        self.set_flag_n(true);
        self.set_flag_h(((a & 0x0F) as i8 - (b & 0x0F) as i8 - c as i8) < 0);
        self.set_flag_c(b1 || b2);
        res
    }

    /// Push a 16-bit value to the stack (little-endian in memory).
    fn push(&mut self, mmu: &mut MMU, value: u16) {
        self.sp = self.sp.wrapping_sub(2);
        mmu.write_byte(self.sp, (value & 0xFF) as u8);      // Low byte
        mmu.write_byte(self.sp.wrapping_add(1), (value >> 8) as u8); // High byte
    }

    /// Pop a 16-bit value from the stack.
    fn pop(&mut self, mmu: &mut MMU) -> u16 {
        let lo = mmu.read_byte(self.sp) as u16;
        let hi = mmu.read_byte(self.sp.wrapping_add(1)) as u16;
        self.sp = self.sp.wrapping_add(2);
        (hi << 8) | lo
    }

    /// Push/pop helpers for AF respect that the lower nibble of F is always zero.
    fn push_af(&mut self, mmu: &mut MMU) { 
        self.push(mmu, self.get_af()); 
    }

    fn pop_af(&mut self, mmu: &mut MMU) {
        let v = self.pop(mmu);
        self.set_af(v); // masks F a 0xF0
    }

    /// Push a 16-bit register pair to the stack.
    fn push_reg_pair(&mut self, mmu: &mut MMU, high: u8, low: u8) {
        self.push(mmu, ((high as u16) << 8) | (low as u16));
    }

    // Pop a 16-bit register pair from the stack.
    fn pop_reg_pair(&mut self, mmu: &mut MMU) -> (u8, u8) {
        let value = self.pop(mmu);
        ((value >> 8) as u8, (value & 0xFF) as u8)
    }

    // ---- Flag helpers -------------------------------------------------------
    // set_flag_* and get_flag_* manipulate bits: Z=0x80, N=0x40, H=0x20, C=0x10.
    // ---

    fn set_flag_z(&mut self, v: bool) {
        if v {
            self.f |= 0x80;
        } else {
            self.f &= !0x80;
        }
    }

    fn set_flag_n(&mut self, v: bool) {
        if v {
            self.f |= 0x40;
        } else {
            self.f &= !0x40;
        }
    }

    fn set_flag_h(&mut self, v: bool) {
        if v {
            self.f |= 0x20;
        } else {
            self.f &= !0x20;
        }
    }

    fn set_flag_c(&mut self, v: bool) {
        if v {
            self.f |= 0x10;
        } else {
            self.f &= !0x10;
        }
    }

    fn get_flag_n(&self) -> bool {
        self.f & 0x40 != 0
    }

    fn get_flag_h(&self) -> bool {
        self.f & 0x20 != 0
    }

    fn get_flag_c(&self) -> bool {
        self.f & 0x10 != 0
    }

    fn get_flag_z(&self) -> bool {
        self.f & 0x80 != 0
    }

    fn set_af(&mut self, val: u16) {
        self.a = (val >> 8) as u8;
        self.f = (val & 0xF0) as u8; // only bits 7–4 of F are valid; lower 4 bits must be zero
    }

    fn get_af(&self) -> u16 {
        ((self.a as u16) << 8) | (self.f as u16)
    }

    fn set_bc(&mut self, val: u16) {
        self.b = (val >> 8) as u8;
        self.c = (val & 0xFF) as u8;
    }

    fn get_bc(&self) -> u16 {
        ((self.b as u16) << 8) | (self.c as u16)
    }

    fn set_hl(&mut self, val: u16) {
        self.h = (val >> 8) as u8;
        self.l = (val & 0xFF) as u8;
    }

    fn get_hl(&self) -> u16 {
        ((self.h as u16) << 8) | (self.l as u16)
    }

    fn set_de(&mut self, val: u16) {
        self.d = (val >> 8) as u8;
        self.e = (val & 0xFF) as u8;
    }

    fn get_de(&self) -> u16 {
        ((self.d as u16) << 8) | (self.e as u16)
    }

    fn vblank_pending(&self, mmu: &MMU) -> bool {
        (mmu.read_byte(0xFFFF) & mmu.read_byte(0xFF0F)) & 0x01 != 0
    }

    // Handle only VBlank (bit 0) for Tetris; ignore other sources.
    fn service_interrupt(&mut self, mmu: &mut MMU) -> u32 {
        // Clear IF.VBlank and jump to 0x0040
        let iflag = mmu.read_byte(0xFF0F) & !0x01;
        mmu.write_byte(0xFF0F, iflag);

        self.ime = false;
        self.push(mmu, self.pc);
        self.pc = 0x0040; // VBlank vector
        20 // t-cycles
    }
}
