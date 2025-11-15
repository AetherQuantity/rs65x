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
use crate::isa::memory::{AddressMode, address_mode_subtypes::NoMemType};
use crate::isa::op::{MicroContext, MicroExecutor, StepResult, Uop, UopQueue};
use crate::isa::table::{Instruction, Mnemonic};
use crate::psr;

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
    uops: UopQueue,
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
            uops: UopQueue::default(),
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
        self.uops.clear();
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
        let instruction = Instruction::from_byte(opcode);
        self.current_opcode = opcode;
        self.current_inst = Some(instruction);
        self.scratch.reset();
        instruction
            .address_mode
            .emit_uops(&mut self.uops, instruction.memory_action);
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
            Adc | Sbc => {
                todo!()
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
                Pla => self.a = read,
                Plx => self.x = read,
                Ply => self.y = read,
                _ => unreachable!(),
            },
            Bit => todo!(),
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
            _ => {}
        }
    }

    pub fn step(&mut self, bus: &mut B) -> StepResult {
        if self.uops.front().is_none() {
            let opcode = self.fetch_opcode(bus);
            self.prepare_instruction(opcode);
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
            self.scratch.execute_next(&mut ctx, bus, &mut self.uops)
        };

        if matches!(result, StepResult::InstructionFinished) {
            // for read instructions, the byte is stored in Op0
            if instruction.memory_action == crate::isa::memory::MemoryAction::Read {
                self.finish_read(instruction.mnemonic);
            }
            if matches!(
                instruction.address_mode,
                AddressMode::NoMemory(NoMemType::Implied)
            ) {
                self.finish_implied(instruction.mnemonic);
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
    fn alu_set_zn(&mut self, value: u8) {
        self.alu_set_flag(psr::Z, value == 0);
        self.alu_set_flag(psr::N, (value & psr::N) != 0);
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
    fn alu_prepare_store(&mut self, scratch: &mut MicroExecutor) -> u8 {
        use Mnemonic::*;
        match self.mnemonic() {
            Sta => scratch.op0 = *self.a,
            Stx => scratch.op0 = *self.x,
            Sty => scratch.op0 = *self.y,
            Stz => scratch.op0 = 0,
            _ => {}
        }
        scratch.op0
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

    fn alu_branch(&mut self, scratch: &mut MicroExecutor, queue: &mut UopQueue) -> bool {
        use Mnemonic::*;
        let status = *self.status;
        let mnemonic = self.mnemonic();
        let taken = match mnemonic {
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
        };
        if taken {
            let old_pc = self.pc();
            let target = old_pc.wrapping_add(scratch.signed_offset8 as i16 as u16);
            let addr = ((old_pc & 0xFF00) | (target & 0x00FF)) as u16;
            // first: set the PC to this new offset address. could be valid or invalid
            self.set_pc(addr);
            let page_cross = (old_pc & 0xFF00) != (target & 0xFF00);
            if !page_cross {
                // easy mode: the PC is already correct, finish on next cycle (and fetch next opcode)
                queue.push(Uop::Finished);
            } else {
                // slightly harder mode: the PC is incorrect, so we need to fix it next cycle.
                // FixPc reads the wrong one next cycle and fixes, finished reads true addr as next opcode
                queue.push(Uop::FixPc(target));
                queue.push(Uop::Finished);
            }
        }
        // if branch not taken, push nothing--we read the next opcode on THIS CYCLE
        taken
    }

    fn alu_push_value(&mut self, _scratch: &mut MicroExecutor) -> u8 {
        use Mnemonic::*;
        match self.mnemonic() {
            Pha => *self.a,
            Php => {
                let mut value = *self.status;
                value |= psr::B_6502 | psr::U_6502;
                value
            }
            Phx => *self.x,
            Phy => *self.y,
            other => panic!("AluPush requested for unsupported mnemonic {:?}", other),
        }
    }
}
