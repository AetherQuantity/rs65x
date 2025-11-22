//! Minimal 6502 core scaffold (per-instruction stepping) wired to the shared Bus.
//!
//! This file intentionally starts tiny so we can iterate file-by-file under the
//! editor constraint. We'll later move opcode tables and addressing helpers into
//! `src/isa/` and expand to per-cycle micro-ops. For now: NOP and LDA #imm so we
//! can smoke-test the Bus and reset vector logic.

pub mod flavor;
pub mod tests;

use core::marker::PhantomData;

use crate::bus::Bus;
use crate::isa::JumpType;
use crate::isa::microcycle::{DecodeContext, UcycQueue};
use crate::isa::op::{MicroContext, MicroExecutor, StepResult};
use crate::isa::table::{Instruction, Mnemonic};
use crate::isa::{AddressMode, address_mode_subtypes::NoMemType};
use crate::psr;

use flavor::DecimalSemantics;

pub use flavor::Flavor; // re-export for convenience

/// CPU registers/state for a plain 6502-like core (8-bit A/X/Y, 16-bit PC).
pub struct Cpu6502<F: Flavor, B: Bus> {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub s: u8,   // stack pointer
    pub p: u8,   // processor status
    pub pc: u16, // program counter
    pub cycles: u64,
    ucycs: UcycQueue,
    scratch: MicroExecutor,
    current_inst: Option<Instruction>,
    current_opcode: u8,
    _f: PhantomData<(F, B)>,
}

impl<F: Flavor, B: Bus> Cpu6502<F, B> {
    #[inline(always)]
    pub fn new() -> Self {
        Self {
            a: 0,
            x: 0,
            y: 0,
            s: 0xFD,                 // reset default
            p: psr::I | psr::U_6502, // I set, U set in pushes on many parts
            pc: 0,
            cycles: 0,
            ucycs: UcycQueue::default(),
            scratch: MicroExecutor::new(),
            current_inst: None,
            current_opcode: 0,
            _f: PhantomData,
        }
    }

    /// Read little-endian 16-bit vector from bank 0.
    /// TODO this is just a helper for something that's gonna get replaced with real cycle-accurate
    /// stuff later
    #[inline(always)]
    fn read16(bus: &mut B, addr: u16) -> u16 {
        let (lo, w0) = bus.read(addr as u32, /*vda=*/ true, /*vpa=*/ false);
        let (hi, w1) = bus.read(
            addr.wrapping_add(1) as u32,
            /*vda=*/ true,
            /*vpa=*/ false,
        );
        let _ = (w0, w1); // wait-states are accumulated by the caller as needed later
        u16::from_le_bytes([lo, hi])
    }

    /// Reset sequence: fetch PC from $FFFC/$FFFD. (IRQ/NMI vectors not handled here.)
    /// TODO this is not cycle accurate but whatever, i'll fix it later
    #[inline(always)]
    pub fn reset(&mut self, bus: &mut B) {
        self.p |= psr::I; // mask IRQ
        self.pc = Self::read16(bus, 0xFFFC); // TODO: implement the real cycle-accurate reset sequence
        self.cycles = 0;
        self.ucycs.clear();
        self.scratch.reset();
        self.current_inst = None;
        self.current_opcode = 0;
    }

    fn fetch_opcode(&mut self, bus: &mut B) -> u8 {
        let (opcode, wait) = bus.read(self.pc as u32, false, true);
        self.tick(wait);
        self.pc = self.pc.wrapping_add(1);
        opcode
    }

    fn prepare_instruction(&mut self, opcode: u8) {
        let instruction = Instruction::from_byte(opcode, F::OPCODE_TABLE);
        self.current_opcode = opcode;
        self.current_inst = Some(instruction);
        self.scratch.reset();
        self.scratch.memory_action = instruction.memory_action;
        let ctx = DecodeContext {
            e_flag: true,
            m_flag: true,
            x_flag: true,
            action: instruction.memory_action,
            modify_read: !F::RMW_DUMMY_WRITE,
        };
        instruction
            .address_mode
            .emit_ucycs::<F::Micro>(&mut self.ucycs, ctx);
    }

    fn finish_read(&mut self, op: Mnemonic) {
        use Mnemonic::*;
        let read = self.scratch.op0;
        match op {
            Lda => self.a = self.alu_set_zn(read),
            Ldx => self.x = self.alu_set_zn(read),
            Ldy => self.y = self.alu_set_zn(read),
            And => self.a = self.alu_set_zn(self.a & read),
            Eor => self.a = self.alu_set_zn(self.a ^ read),
            Ora => self.a = self.alu_set_zn(self.a | read),
            Adc => execute_adc::<F, _>(self, read),
            Sbc | Usbc => execute_sbc::<F, _>(self, read),
            Cmp | Cpx | Cpy => {
                let byte = match op {
                    Cmp => self.a,
                    Cpx => self.x,
                    Cpy => self.y,
                    _ => unreachable!(),
                };
                self.alu_set_zn(byte.wrapping_sub(read));
                self.alu_set_flag(psr::C, byte >= read);
            }
            Pla => self.a = self.alu_set_zn(read),
            Plx => self.x = self.alu_set_zn(read),
            Ply => self.y = self.alu_set_zn(read),
            Plp => self.p = (read | psr::U_6502) & !psr::B_6502,
            Bit => {
                let status = self.p & !(psr::Z | psr::N | psr::V);
                let z = psr::Z * if self.a & read == 0 { 1 } else { 0 };
                self.p = status | z | (read & 0xC0);
            }
            Anc => {
                self.a = self.alu_set_zn(self.a & read);
                self.alu_set_flag(psr::C, self.a & 0x80 != 0);
            }
            Alr => {
                self.a = self.alu_set_zn(self.a & read);
                self.a = self.alu_lsr(self.a);
            }
            Arr => {
                // Illegal ARR: (A & operand) then ROR with weird flag/decimal behaviour.
                // For binary mode we use the commonly documented behaviour:
                //   result = ROR(A & M)
                //   N/Z from result
                //   C = bit 6 of result
                //   V = bit 6 XOR bit 5 of result
                // For NMOS decimal mode we mirror the NESdev reference implementation.
                let decimal_mode =
                    (self.p & psr::D) != 0 && matches!(F::DECIMAL, DecimalSemantics::Nmos6502);

                if !decimal_mode {
                    // Binary (or NES-style no-decimal) behaviour.
                    let result = self.alu_ror(self.a & read);
                    // alu_ror already set Z/N from result.
                    self.a = result;
                    let c = (result & 0x40) != 0;
                    let v = ((result >> 6) ^ (result >> 5)) & 1 != 0;
                    self.alu_set_flag(psr::C, c);
                    self.alu_set_flag(psr::V, v);
                } else {
                    // NMOS decimal-mode ARR, ported from NESdev's C reference
                    // THANK YOU NESDEV
                    let carry = self.alu_carry();
                    let anded = self.a & read;
                    let ah = anded >> 4;
                    let al = anded & 0x0F;

                    // Perform ROR(anded) with carry-in C, using the regular ALU helper.
                    // This sets Z and a temporary N from the rotate result and C from bit 0.
                    let mut a = self.alu_ror(anded);
                    self.a = a;

                    // Now impose ARR's odd flag behaviour:
                    //   N = old carry
                    //   Z = from result (already set by alu_ror)
                    //   V = (t ^ A) & 0x40
                    self.alu_set_flag(psr::N, carry);
                    let v = ((anded ^ a) & 0x40) != 0;
                    self.alu_set_flag(psr::V, v);

                    // Decimal fixup for low nibble: if AL + (AL & 1) > 5 then add 6 to low nibble.
                    if al.wrapping_add(al & 1) > 5 {
                        a = (a & 0xF0) | (a.wrapping_add(6) & 0x0F);
                    }

                    // Decimal fixup for high nibble and final carry flag.
                    let high_cond = ah.wrapping_add(ah & 1) > 5;
                    if high_cond {
                        a = a.wrapping_add(0x60);
                    }
                    self.a = a;
                    self.alu_set_flag(psr::C, high_cond);
                    // Note: N and Z remain as set immediately after the rotate,
                    // matching the real NMOS behavior where BCD adjustment
                    // does not update them.
                }
            }
            Ane => {
                // so... this opcode sucks.
                // apparently there's a magic constant, quote:
                //     The value of this constant depends on temerature, the chip series,
                //     and maybe other factors, as well.
                // the tests i downloaded, after trial and error, seem to expect 0xEE. so.. yeah.
                let magic = 0xEE; // this is apparently the magic constant my tests expect
                self.a = self.alu_set_zn((self.a | magic) & self.x & read);
            }
            Lxa => {
                let magic = 0xEE;
                self.a = self.alu_set_zn((self.a | magic) & read);
                self.x = self.a;
            }
            Lax => {
                self.a = self.alu_set_zn(read);
                self.x = read;
            }
            Las => {
                let result = self.alu_set_zn(read & self.s);
                self.a = result;
                self.x = result;
                self.s = result;
            }
            Sbx => {
                let result = self.a & self.x;
                self.x = self.alu_set_zn(result.wrapping_sub(read));
                self.alu_set_flag(psr::C, result >= read);
            }
            Brk => {
                if matches!(F::DECIMAL, DecimalSemantics::Cmos65C02) {
                    // on CMOS chips, interrupts clear the decimal flag!
                    self.alu_set_flag(psr::D, false);
                }
            }
            _ => {}
        }
    }

    fn finish_implied(&mut self, op: Mnemonic) {
        use Mnemonic::*;
        match op {
            Asl => self.a = self.alu_asl(self.a),
            Lsr => self.a = self.alu_lsr(self.a),
            Rol => self.a = self.alu_rol(self.a),
            Ror => self.a = self.alu_ror(self.a),
            Clc => self.p &= !psr::C,
            Cld => self.p &= !psr::D,
            Cli => self.p &= !psr::I,
            Clv => self.p &= !psr::V,
            Sec => self.p |= psr::C,
            Sed => self.p |= psr::D,
            Sei => self.p |= psr::I,
            Dex => self.x = self.alu_set_zn(self.x.wrapping_sub(1)),
            Dey => self.y = self.alu_set_zn(self.y.wrapping_sub(1)),
            Inx => self.x = self.alu_set_zn(self.x.wrapping_add(1)),
            Iny => self.y = self.alu_set_zn(self.y.wrapping_add(1)),
            Tax => self.x = self.alu_set_zn(self.a),
            Tay => self.y = self.alu_set_zn(self.a),
            Tsx => self.x = self.alu_set_zn(self.s),
            Txa => self.a = self.alu_set_zn(self.x),
            Tya => self.a = self.alu_set_zn(self.y),
            Txs => self.s = self.x, // don't set Z/N
            _ => {}
        }
    }

    pub fn step(&mut self, bus: &mut B) -> StepResult {
        if self.ucycs.front().is_none() {
            let opcode = self.fetch_opcode(bus);
            self.prepare_instruction(opcode);
            return StepResult::Pending;
        }

        let instruction = self
            .current_inst
            .expect("micro-op execution without decoded instruction");

        let result = {
            let mut ctx = MicroCtx6502::<F, B> {
                pc: &mut self.pc,
                sp: &mut self.s,
                status: &mut self.p,
                a: &mut self.a,
                x: &mut self.x,
                y: &mut self.y,
                cycles: &mut self.cycles,
                instruction,
                opcode: self.current_opcode,
                _marker: PhantomData,
            };
            self.scratch.execute_next(&mut ctx, bus, &mut self.ucycs)
        };

        if matches!(result, StepResult::InstructionFinished) {
            // for read instructions, the byte is stored in Op0
            if instruction.memory_action == crate::isa::MemoryAction::Read {
                self.finish_read(instruction.mnemonic);
            }
            match instruction.address_mode {
                AddressMode::NoMemory(NoMemType::Implied) => {
                    self.finish_implied(instruction.mnemonic)
                }
                AddressMode::Jump(JumpType::ToInterrupt) => {
                    self.p |= psr::I;
                }
                _ => (),
            }
            // the instruction finishes here, but we need to read the next opcode for next cycle
            let opcode = self.fetch_opcode(bus);
            self.prepare_instruction(opcode);
        }

        result
    }

    #[inline(always)]
    fn tick(&mut self, wait: u8) {
        self.cycles += 1 + wait as u64;
    }
}

pub(crate) fn execute_adc<F: Flavor, S: AluOps>(state: &mut S, operand: u8) {
    let v_flag = |lhs, rhs, result| ((lhs ^ result) & 0x80) != 0 && ((lhs ^ rhs) & 0x80) == 0;
    let decimal = (state.status() & psr::D) != 0;
    let acc = state.accumulator();
    let carry = u16::from(state.alu_carry());
    let sum = acc as u16 + operand as u16 + carry;
    let bin_result = sum as u8;
    let binary_overflow = v_flag(acc, operand, bin_result);
    if !decimal || matches!(F::DECIMAL, DecimalSemantics::None) {
        // if we're in binary mode, we're done!
        state.set_accumulator(bin_result);
        state.alu_set_flag(psr::C, sum > 0xFF);
        state.alu_set_flag(psr::V, binary_overflow);
        state.alu_set_zn(bin_result);
        return;
    }
    // otherwise, we need to adjust for decimal mode
    let mut low = (acc & 0x0F) as u16 + (operand & 0x0F) as u16 + carry;
    let mut high = (acc >> 4) as u16 + (operand >> 4) as u16 + u16::from(low > 9);
    if low > 9 {
        low += 6;
    }
    let compose = |hi: u16, lo: u16| -> u8 { (((hi << 4) | (lo & 0x0F)) & 0xFF) as u8 };
    let pre_high = compose(high, low);
    if high > 9 {
        high += 6;
    }
    let carry_out = high > 0x0F;
    let dec_result = compose(high, low);
    state.set_accumulator(dec_result);
    state.alu_set_flag(psr::C, carry_out);
    match F::DECIMAL {
        DecimalSemantics::Nmos6502 => {
            state.alu_set_flag(psr::Z, bin_result == 0);
            state.alu_set_flag(psr::V, v_flag(acc, operand, pre_high));
            state.alu_set_flag(psr::N, (pre_high & psr::N) != 0);
        }
        DecimalSemantics::Cmos65C02 => {
            state.alu_set_flag(psr::Z, dec_result == 0);
            state.alu_set_flag(psr::V, binary_overflow);
            state.alu_set_flag(psr::N, (dec_result & psr::N) != 0);
        }
        DecimalSemantics::None => unreachable!(),
    }
}

pub(crate) fn execute_sbc<F: Flavor, S: AluOps>(state: &mut S, operand: u8) {
    // We implement SBC as A + (~operand) + C, mirroring execute_adc's structure
    // and then apply optional BCD correction depending on DecimalSemantics.
    let v_flag =
        |lhs: u8, rhs: u8, result: u8| ((lhs ^ result) & 0x80) != 0 && ((lhs ^ rhs) & 0x80) == 0;

    let decimal = (state.status() & psr::D) != 0;
    let acc = state.accumulator();
    let carry = if state.alu_carry() { 1u16 } else { 0u16 };

    // Binary core: A + (~M) + C
    let value = operand ^ 0xFF;
    let sum = acc as u16 + value as u16 + carry;
    let bin_result = sum as u8;
    let binary_overflow = v_flag(acc, value, bin_result);

    // If decimal mode is disabled or we model a CPU without decimal support,
    // this is just plain binary SBC.
    if !decimal || matches!(F::DECIMAL, DecimalSemantics::None) {
        state.set_accumulator(bin_result);
        state.alu_set_flag(psr::C, sum > 0xFF);
        state.alu_set_flag(psr::V, binary_overflow);
        state.alu_set_zn(bin_result);
        return;
    }

    match F::DECIMAL {
        // NMOS 6502: perform nibble-wise BCD subtraction (including invalid digits),
        // but set flags from the *binary* core result (like real hardware).
        DecimalSemantics::Nmos6502 => {
            let a = acc;
            let carry_in = state.alu_carry(); // C = 1 means "no borrow"

            // Low nibble: (A_lo - M_lo - !C), with wrap and BCD correction.
            let mut tmp =
                (a & 0x0F) as i16 - (operand & 0x0F) as i16 - if carry_in { 0 } else { 1 };
            if tmp < 0 {
                // Wrap back into the 0–15 range, then subtract 6 for BCD,
                // and propagate a borrow into the high nibble via -0x10.
                tmp = ((tmp - 6) & 0x0F) - 0x10;
            }

            // High nibble: (A_hi - M_hi + low_nibble_result), again with wrap fixup.
            tmp = (a & 0xF0) as i16 - (operand & 0xF0) as i16 + tmp;
            if tmp < 0 {
                tmp -= 0x60;
            }

            let dec_result = (tmp as u8) & 0xFF;
            state.set_accumulator(dec_result);

            // Flags are taken from the binary SBC core (sum/bin_result/binary_overflow).
            state.alu_set_flag(psr::C, sum > 0xFF);
            state.alu_set_flag(psr::V, binary_overflow);
            state.alu_set_zn(bin_result);
        }

        // 65C02: keep the simpler "good BCD" behaviour for now — treat A and operand
        // as packed BCD 00–99, do base‑10 subtraction, then convert back.
        DecimalSemantics::Cmos65C02 => {
            let a_tens = (acc >> 4) & 0x0F;
            let a_ones = acc & 0x0F;
            let m_tens = (operand >> 4) & 0x0F;
            let m_ones = operand & 0x0F;

            let a_dec = (a_tens as i16) * 10 + (a_ones as i16);
            let m_dec = (m_tens as i16) * 10 + (m_ones as i16);
            let borrow = if state.alu_carry() { 0i16 } else { 1i16 }; // C=1 means "no borrow"

            let mut diff = a_dec - m_dec - borrow;
            let carry_out = diff >= 0;
            if diff < 0 {
                // Wrap into the 0–99 range, modelling the fact we only keep two BCD digits.
                diff += 100;
            }

            let dec_u8 = diff as u8; // now in 0–99
            let tens = dec_u8 / 10;
            let ones = dec_u8 % 10;
            let dec_result = (tens << 4) | ones;

            state.set_accumulator(dec_result);
            state.alu_set_flag(psr::C, carry_out);

            // 65C02: Z/N come from the *decimal* result, V from the binary overflow.
            state.alu_set_flag(psr::Z, dec_result == 0);
            state.alu_set_flag(psr::V, binary_overflow);
            state.alu_set_flag(psr::N, (dec_result & psr::N) != 0);
        }

        DecimalSemantics::None => unreachable!(),
    }
}

pub(crate) trait AluOps {
    fn status(&self) -> u8;
    fn status_mut(&mut self) -> &mut u8;
    fn accumulator(&self) -> u8;
    fn accumulator_mut(&mut self) -> &mut u8;

    #[inline(always)]
    fn set_accumulator(&mut self, value: u8) {
        *self.accumulator_mut() = value;
    }

    #[inline(always)]
    fn alu_set_flag(&mut self, mask: u8, value: bool) {
        let status = self.status_mut();
        if value {
            *status |= mask;
        } else {
            *status &= !mask;
        }
    }

    #[inline(always)]
    fn alu_set_zn(&mut self, value: u8) -> u8 {
        self.alu_set_flag(psr::Z, value == 0);
        self.alu_set_flag(psr::N, (value & psr::N) != 0);
        value
    }

    #[inline(always)]
    fn alu_carry(&self) -> bool {
        (self.status() & psr::C) != 0
    }

    #[inline(always)]
    fn alu_asl(&mut self, value: u8) -> u8 {
        let carry = (value & 0x80) != 0;
        let result = value.wrapping_shl(1);
        self.alu_set_flag(psr::C, carry);
        self.alu_set_zn(result);
        result
    }

    #[inline(always)]
    fn alu_lsr(&mut self, value: u8) -> u8 {
        let carry = (value & 0x01) != 0;
        let result = value >> 1;
        self.alu_set_flag(psr::C, carry);
        self.alu_set_zn(result);
        result
    }

    #[inline(always)]
    fn alu_rol(&mut self, value: u8) -> u8 {
        let carry_out = (value & 0x80) != 0;
        let carry_in = if self.alu_carry() { 1 } else { 0 };
        let result = value.wrapping_shl(1) | carry_in;
        self.alu_set_flag(psr::C, carry_out);
        self.alu_set_zn(result);
        result
    }

    #[inline(always)]
    fn alu_ror(&mut self, value: u8) -> u8 {
        let carry_out = (value & 0x01) != 0;
        let carry_in = if self.alu_carry() { 0x80 } else { 0 };
        let result = (value >> 1) | carry_in;
        self.alu_set_flag(psr::C, carry_out);
        self.alu_set_zn(result);
        result
    }

    #[inline(always)]
    fn alu_inc(&mut self, value: u8) -> u8 {
        let result = value.wrapping_add(1);
        self.alu_set_zn(result);
        result
    }

    #[inline(always)]
    fn alu_dec(&mut self, value: u8) -> u8 {
        let result = value.wrapping_sub(1);
        self.alu_set_zn(result);
        result
    }

    #[inline(always)]
    fn alu_tsb(&mut self, value: u8) -> u8 {
        let a = self.accumulator();
        self.alu_set_flag(psr::Z, (a & value) == 0);
        value | a
    }

    #[inline(always)]
    fn alu_trb(&mut self, value: u8) -> u8 {
        let a = self.accumulator();
        self.alu_set_flag(psr::Z, (a & value) == 0);
        value & !a
    }

    #[inline(always)]
    fn alu_clear_bit(&mut self, value: u8, bit: u8) -> u8 {
        value & !(1 << bit)
    }

    #[inline(always)]
    fn alu_set_bit(&mut self, value: u8, bit: u8) -> u8 {
        value | (1 << bit)
    }
}

impl<F: Flavor, B: Bus> AluOps for Cpu6502<F, B> {
    #[inline(always)]
    fn status(&self) -> u8 {
        self.p
    }

    #[inline(always)]
    fn status_mut(&mut self) -> &mut u8 {
        &mut self.p
    }

    #[inline(always)]
    fn accumulator(&self) -> u8 {
        self.a
    }

    #[inline(always)]
    fn accumulator_mut(&mut self) -> &mut u8 {
        &mut self.a
    }
}

struct MicroCtx6502<'a, F: Flavor, B: Bus> {
    pc: &'a mut u16,
    sp: &'a mut u8,
    status: &'a mut u8,
    a: &'a mut u8,
    x: &'a mut u8,
    y: &'a mut u8,
    cycles: &'a mut u64,
    instruction: Instruction,
    opcode: u8,
    _marker: PhantomData<(F, B)>,
}

impl<'a, F: Flavor, B: Bus> MicroCtx6502<'a, F, B> {
    #[inline(always)]
    fn mnemonic(&self) -> Mnemonic {
        self.instruction.mnemonic
    }

    #[inline(always)]
    fn bit_index_rmb(op: u8) -> Option<u8> {
        let low = op & 0x0F;
        if low == 7 {
            let high = op >> 4;
            if high < 8 {
                return Some(high);
            }
        }
        None
    }

    #[inline(always)]
    fn bit_index_smb(op: u8) -> Option<u8> {
        let low = op & 0x0F;
        if low == 7 {
            let high = op >> 4;
            if high > 7 {
                return Some(high - 8);
            }
        }
        None
    }

    #[inline(always)]
    fn branch_bit_info(op: u8) -> Option<(u8, bool)> {
        if op & 0xF != 0xF {
            return None;
        }
        let high = op >> 4;
        Some((high % 8, high / 8 > 0))
    }
}

impl<'a, F: Flavor, B: Bus> AluOps for MicroCtx6502<'a, F, B> {
    #[inline(always)]
    fn status(&self) -> u8 {
        *self.status
    }

    #[inline(always)]
    fn status_mut(&mut self) -> &mut u8 {
        &mut *self.status
    }

    #[inline(always)]
    fn accumulator(&self) -> u8 {
        *self.a
    }

    #[inline(always)]
    fn accumulator_mut(&mut self) -> &mut u8 {
        &mut *self.a
    }
}

impl<'a, F: Flavor, B: Bus> MicroContext for MicroCtx6502<'a, F, B> {
    #[inline(always)]
    fn pc(&self) -> u16 {
        *self.pc
    }

    #[inline(always)]
    fn set_pc(&mut self, value: u16) {
        *self.pc = value;
    }

    #[inline(always)]
    fn set_pc_hi(&mut self, value: u8) {
        *self.pc = (*self.pc & 0x00FF) | ((value as u16) << 8);
    }

    #[inline(always)]
    fn set_pc_lo(&mut self, value: u8) {
        *self.pc = (*self.pc & 0xFF00) | value as u16;
    }

    #[inline(always)]
    fn inc_pc(&mut self) {
        let next = (*self.pc).wrapping_add(1);
        *self.pc = next;
    }

    #[inline(always)]
    fn sp(&self) -> u8 {
        *self.sp
    }

    #[inline(always)]
    fn set_sp(&mut self, value: u8) {
        *self.sp = value;
    }

    #[inline(always)]
    fn status(&self) -> u8 {
        *self.status
    }

    #[inline(always)]
    fn set_status(&mut self, value: u8) {
        *self.status = (value | psr::U_6502) & !psr::B_6502;
    }

    #[inline(always)]
    fn reg_x(&self) -> u16 {
        (*self.x) as u16
    }

    #[inline(always)]
    fn reg_y(&self) -> u16 {
        (*self.y) as u16
    }

    #[inline(always)]
    fn reg_a(&self) -> u16 {
        (*self.a) as u16
    }

    #[inline(always)]
    fn tick(&mut self, wait_states: u8) {
        *self.cycles += 1 + wait_states as u64;
    }

    #[inline(always)]
    fn rmw_dummy_write(&self) -> bool {
        F::RMW_DUMMY_WRITE
    }

    #[inline(always)]
    fn jmp_indirect_wrap_bug(&self) -> bool {
        F::JMP_INDIRECT_WRAP_BUG
    }

    #[inline(always)]
    fn direct_page_base(&self) -> u16 {
        0
    }

    #[inline(always)]
    fn data_bank(&self) -> u8 {
        0
    }

    #[inline(always)]
    fn stack_base(&self) -> u16 {
        0x0100
    }
    fn alu_prepare_store(&mut self, scratch: &mut MicroExecutor) {
        use Mnemonic::*;
        if scratch.memory_action == crate::isa::MemoryAction::ReadModifyWrite {
            // we just need to make sure the value in op0 we saved on the modify phase
            // is in data_latch to be written
            scratch.data_latch = scratch.op0;
            return;
        }
        // otherwise, this is a write instruction, and each has a different value it requires
        match self.mnemonic() {
            Sta | Pha => scratch.data_latch = *self.a,
            Stx | Phx => scratch.data_latch = *self.x,
            Sty | Phy => scratch.data_latch = *self.y,
            Stz => scratch.data_latch = 0,
            Php => scratch.data_latch = *self.status | psr::B_6502,
            Sax => scratch.data_latch = *self.a & *self.x,
            Sha => {
                scratch.data_latch = h_plus_one_nonsense(&mut scratch.ea, *self.a, *self.y, *self.x)
            }
            Tas => {
                let ax = *self.a & *self.x;
                *self.sp = ax;
                scratch.data_latch =
                    h_plus_one_nonsense(&mut scratch.ea, *self.a, *self.y, *self.x);
            }
            Shy => {
                scratch.data_latch = h_plus_one_nonsense(&mut scratch.ea, 0xFF, *self.x, *self.y);
            }
            Shx => {
                scratch.data_latch = h_plus_one_nonsense(&mut scratch.ea, 0xFF, *self.y, *self.x);
            }
            _ => {}
        }
    }

    fn alu_modify(&mut self, scratch: &mut MicroExecutor) {
        use Mnemonic::*;
        let mnemonic = self.mnemonic();
        match mnemonic {
            Asl => scratch.op0 = self.alu_asl(scratch.op0),
            Lsr => scratch.op0 = self.alu_lsr(scratch.op0),
            Rol => scratch.op0 = self.alu_rol(scratch.op0),
            Ror => scratch.op0 = self.alu_ror(scratch.op0),
            Inc => scratch.op0 = self.alu_inc(scratch.op0),
            Dec => scratch.op0 = self.alu_dec(scratch.op0),
            Tsb => scratch.op0 = self.alu_tsb(scratch.op0),
            Trb => scratch.op0 = self.alu_trb(scratch.op0),
            // NMOS illegal opcodes:
            Slo => {
                scratch.op0 = self.alu_asl(scratch.op0); // will be written back
                *self.a = self.alu_set_zn(*self.a | scratch.op0);
            }
            Rla => {
                scratch.op0 = self.alu_rol(scratch.op0); // will be written back
                *self.a = self.alu_set_zn(*self.a & scratch.op0);
            }
            Sre => {
                scratch.op0 = self.alu_lsr(scratch.op0); // will be written back
                *self.a = self.alu_set_zn(*self.a ^ scratch.op0);
            }
            Rra => {
                scratch.op0 = self.alu_ror(scratch.op0);
                execute_adc::<F, _>(self, scratch.op0);
            }
            Dcp => {
                let result = scratch.op0.wrapping_sub(1);
                scratch.op0 = result;
                self.alu_set_zn(self.a.wrapping_sub(result));
                self.alu_set_flag(psr::C, *self.a >= result);
            }
            Isc => {
                scratch.op0 = scratch.op0.wrapping_add(1);
                execute_sbc::<F, _>(self, scratch.op0);
            }
            _ => {
                if let Some(bit) = Self::bit_index_rmb(self.opcode) {
                    scratch.op0 = self.alu_clear_bit(scratch.op0, bit);
                } else if let Some(bit) = Self::bit_index_smb(self.opcode) {
                    scratch.op0 = self.alu_set_bit(scratch.op0, bit);
                }
            }
        }
    }

    fn alu_branch(&mut self, scratch: &mut MicroExecutor, _queue: &mut UcycQueue) -> bool {
        use Mnemonic::*;
        let status = *self.status;
        let mnemonic = self.mnemonic();
        match mnemonic {
            Bcc => (status & psr::C) == 0,
            Bcs => (status & psr::C) != 0,
            Beq => (status & psr::Z) != 0,
            Bne => (status & psr::Z) == 0,
            Bmi => (status & psr::N) != 0,
            Bpl => (status & psr::N) == 0,
            Bvc => (status & psr::V) == 0,
            Bvs => (status & psr::V) != 0,
            Bra => true,
            _ => {
                if let Some((bit, expect_set)) = Self::branch_bit_info(self.opcode) {
                    let mask = 1 << bit;
                    let set = (scratch.op0 & mask) != 0;
                    set == expect_set
                } else {
                    false
                }
            }
        }
    }
}

fn h_plus_one_nonsense(ea: &mut u32, a: u8, offset: u8, op_reg: u8) -> u8 {
    // this opcode was the bane of my existence to implement. holy moly.
    let eff_lo = ea.clone() as u8;
    let wrapped = eff_lo < offset; // we need the pre-fixed hi byte. subtract if we previously overflowed
    let eff_hi = ((ea.clone() >> 8) as u8).wrapping_sub(if wrapped { 1 } else { 0 });
    // base value: A & X & (H+1)
    let value = a & op_reg & eff_hi.wrapping_add(1);
    // so.
    // apparently the address to write to is INSANE here, if we wrapped to get here, the value we're
    // writing becomes the high byte of the address to write to. otherwise, same page high byte. i think.
    let store_hi = if wrapped { value } else { eff_hi };
    let target_addr = ((store_hi as u16) << 8) | eff_lo as u16;
    *ea = target_addr as u32;
    value
}
