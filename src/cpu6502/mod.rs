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
            Lda => {
                self.a = read;
                self.alu_set_zn(self.a);
            }
            Ldx => {
                self.x = read;
                self.alu_set_zn(self.x);
            }
            Ldy => {
                self.y = read;
                self.alu_set_zn(self.y);
            }
            And => {
                self.a &= read;
                self.alu_set_zn(self.a);
            }
            Eor => {
                self.a ^= read;
                self.alu_set_zn(self.a);
            }
            Ora => {
                self.a |= read;
                self.alu_set_zn(self.a);
            }
            Adc => {
                self.execute_adc(read);
            }
            Sbc => {
                self.execute_sbc(read);
            }
            Cmp | Cpx | Cpy => {
                let byte = match op {
                    Cmp => self.a,
                    Cpx => self.x,
                    Cpy => self.y,
                    _ => unreachable!(),
                };
                let result = byte.wrapping_sub(read);
                self.alu_set_zn(result);
                self.alu_set_flag(psr::C, byte >= read);
            }
            Pla | Plx | Ply => match op {
                Pla => self.a = self.alu_set_zn(read),
                Plx => self.x = self.alu_set_zn(read),
                Ply => self.y = self.alu_set_zn(read),
                _ => unreachable!(),
            },
            Bit => {
                let status = self.p & !(psr::Z | psr::N | psr::V);
                let z = psr::Z * if self.a & read == 0 { 1 } else { 0 };
                self.p = status | z | (read & 0xC0);
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
            Dex => self.x = self.x.wrapping_sub(1),
            Dey => self.y = self.y.wrapping_sub(1),
            Inx => self.x = self.x.wrapping_add(1),
            Iny => self.y = self.y.wrapping_add(1),
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
    fn execute_adc(&mut self, operand: u8) {
        let acc = self.a;
        let carry_in = self.alu_carry();
        let binary = Self::binary_add(acc, operand, carry_in);

        if (self.p & psr::D) == 0 || matches!(F::DECIMAL, DecimalSemantics::None) {
            self.finish_adc_binary(binary);
            return;
        }

        let adjust = Self::decimal_adjust_add(acc, operand, carry_in, binary.sum);
        match F::DECIMAL {
            DecimalSemantics::Nmos6502 => {
                self.finish_adc_decimal_nmos(acc, operand, binary, adjust);
            }
            DecimalSemantics::Cmos65C02 => {
                self.finish_adc_decimal_cmos(binary, adjust);
            }
            DecimalSemantics::None => unreachable!(),
        }
    }

    #[inline(always)]
    fn finish_adc_binary(&mut self, binary: BinaryAddResult) {
        self.finish_binary_result(binary.result, binary.carry, binary.overflow);
    }

    #[inline(always)]
    fn finish_adc_decimal_nmos(
        &mut self,
        acc: u8,
        operand: u8,
        binary: BinaryAddResult,
        adjust: DecimalAddAdjust,
    ) {
        self.a = adjust.result();
        self.alu_set_flag(psr::C, adjust.carry());
        let pre = adjust.pre_high();
        self.alu_set_flag(psr::V, Self::adc_overflow(acc, operand, pre));
        self.alu_set_flag(psr::N, (pre & psr::N) != 0);
        self.alu_set_flag(psr::Z, binary.result == 0);
    }

    #[inline(always)]
    fn finish_adc_decimal_cmos(&mut self, binary: BinaryAddResult, adjust: DecimalAddAdjust) {
        let result = adjust.result();
        self.a = result;
        self.alu_set_flag(psr::C, adjust.carry());
        self.alu_set_flag(psr::V, binary.overflow);
        self.alu_set_flag(psr::Z, result == 0);
        self.alu_set_flag(psr::N, (result & psr::N) != 0);
    }

    #[inline(always)]
    fn decimal_adjust_add(acc: u8, operand: u8, carry_in: bool, sum: u16) -> DecimalAddAdjust {
        let carry = u8::from(carry_in) as u16;
        let low_sum = (acc & 0x0F) as u16 + (operand & 0x0F) as u16 + carry;
        let mut adjusted = sum;
        if low_sum > 9 {
            adjusted = adjusted.wrapping_add(0x06);
        }
        let pre_high = adjusted as u8;
        if adjusted > 0x99 {
            adjusted = adjusted.wrapping_add(0x60);
        }
        DecimalAddAdjust {
            pre_high,
            result: adjusted as u8,
            carry: adjusted > 0xFF,
        }
    }

    #[inline(always)]
    fn adc_overflow(acc: u8, operand: u8, result: u8) -> bool {
        ((acc ^ result) & 0x80) != 0 && ((acc ^ operand) & 0x80) == 0
    }

    #[inline(always)]
    fn execute_sbc(&mut self, operand: u8) {
        let acc = self.a;
        let carry_in = self.alu_carry();
        let inverted = operand ^ 0xFF;
        let binary = Self::binary_add(acc, inverted, carry_in);
        let overflow = Self::sbc_overflow(acc, operand, binary.result);

        if (self.p & psr::D) == 0 || matches!(F::DECIMAL, DecimalSemantics::None) {
            self.finish_binary_result(binary.result, binary.carry, overflow);
            return;
        }

        let adjust = Self::decimal_adjust_sub(acc, operand, carry_in, binary.result);
        match F::DECIMAL {
            DecimalSemantics::Nmos6502 => {
                self.finish_sbc_decimal_nmos(binary, overflow, adjust);
            }
            DecimalSemantics::Cmos65C02 => {
                self.finish_sbc_decimal_cmos(binary.carry, overflow, adjust);
            }
            DecimalSemantics::None => {
                self.finish_binary_result(binary.result, binary.carry, overflow);
            }
        }
    }

    #[inline(always)]
    fn finish_sbc_decimal_nmos(
        &mut self,
        binary: BinaryAddResult,
        overflow: bool,
        adjust: DecimalSubAdjust,
    ) {
        self.a = adjust.result();
        self.alu_set_flag(psr::C, binary.carry);
        self.alu_set_flag(psr::V, overflow);
        self.alu_set_flag(psr::N, (binary.result & psr::N) != 0);
        self.alu_set_flag(psr::Z, binary.result == 0);
    }

    #[inline(always)]
    fn finish_sbc_decimal_cmos(&mut self, carry: bool, overflow: bool, adjust: DecimalSubAdjust) {
        let result = adjust.result();
        self.a = result;
        self.alu_set_flag(psr::C, carry);
        self.alu_set_flag(psr::V, overflow);
        self.alu_set_flag(psr::Z, result == 0);
        self.alu_set_flag(psr::N, (result & psr::N) != 0);
    }

    #[inline(always)]
    fn decimal_adjust_sub(
        acc: u8,
        operand: u8,
        carry_in: bool,
        binary_result: u8,
    ) -> DecimalSubAdjust {
        let borrow = if carry_in { 0u16 } else { 1u16 };
        let mut result = binary_result;
        let low_acc = (acc & 0x0F) as u16;
        let low_op = (operand & 0x0F) as u16;
        if low_acc < low_op + borrow {
            result = result.wrapping_sub(0x06);
        }
        let acc16 = acc as u16;
        let op16 = operand as u16;
        if acc16 < op16 + borrow {
            result = result.wrapping_sub(0x60);
        }
        DecimalSubAdjust { result }
    }

    #[inline(always)]
    fn sbc_overflow(acc: u8, operand: u8, result: u8) -> bool {
        ((acc ^ operand) & (acc ^ result) & 0x80) != 0
    }

    #[inline(always)]
    fn finish_binary_result(&mut self, result: u8, carry: bool, overflow: bool) {
        self.a = result;
        self.alu_set_flag(psr::C, carry);
        self.alu_set_flag(psr::V, overflow);
        self.alu_set_zn(result);
    }

    #[inline(always)]
    fn binary_add(lhs: u8, rhs: u8, carry_in: bool) -> BinaryAddResult {
        let carry = u8::from(carry_in) as u16;
        let sum = lhs as u16 + rhs as u16 + carry;
        let result = sum as u8;
        BinaryAddResult {
            sum,
            result,
            carry: sum > 0xFF,
            overflow: Self::adc_overflow(lhs, rhs, result),
        }
    }

    #[inline(always)]
    fn tick(&mut self, wait: u8) {
        self.cycles += 1 + wait as u64;
    }
}

#[derive(Clone, Copy)]
struct BinaryAddResult {
    sum: u16,
    result: u8,
    carry: bool,
    overflow: bool,
}

#[derive(Clone, Copy)]
struct DecimalAddAdjust {
    pre_high: u8,
    result: u8,
    carry: bool,
}

impl DecimalAddAdjust {
    #[inline(always)]
    fn result(self) -> u8 {
        self.result
    }

    #[inline(always)]
    fn carry(self) -> bool {
        self.carry
    }

    #[inline(always)]
    fn pre_high(self) -> u8 {
        self.pre_high
    }
}

#[derive(Clone, Copy)]
struct DecimalSubAdjust {
    result: u8,
}

impl DecimalSubAdjust {
    #[inline(always)]
    fn result(self) -> u8 {
        self.result
    }
}

trait AluOps {
    fn status(&self) -> u8;
    fn status_mut(&mut self) -> &mut u8;
    fn accumulator(&self) -> u8;

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
        *self.status = value;
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
