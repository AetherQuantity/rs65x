//! Shared ISA (Instruction Set Architecture) metadata used by 6502/65C02/Rockwell/65C816 cores.
//!
//! This module intentionally contains **no core-specific state** and **no function pointers**.
//! It defines *metadata* that opcode tables for different cores can reuse without duplication.
//! Each core (8‑bit or 16‑bit) can wrap these with its own exec function pointers later.

use crate::{
    bus::{Bus, WaitStates},
    isa::{
        Latch, MemoryAction,
        microcycle::{AluOp, MicroCycle, UcycQueue},
    },
};

#[derive(Clone, Copy)]
pub enum DataDest {
    Discard,
    EffectiveAddressLow,
    EffectiveAddressHigh,
    DataLatch,
}

#[derive(Default, Clone)]
pub struct MicroExecutor {
    pub data_latch: u8,
    pub ea: u32,
    pub ptr: u16,
    pub op0: u8,
    pub signed_offset8: i8,
    pub memory_action: MemoryAction,
}

pub enum StepResult {
    Pending,
    InstructionFinished,
}

pub trait MicroContext {
    fn pc(&self) -> u16;
    fn set_pc(&mut self, value: u16);
    fn set_pc_hi(&mut self, value: u8);
    fn set_pc_lo(&mut self, value: u8);
    fn inc_pc(&mut self);

    fn sp(&self) -> u8;
    fn set_sp(&mut self, value: u8);

    fn status(&self) -> u8;
    fn set_status(&mut self, value: u8);

    fn reg_x(&self) -> u16;
    fn reg_y(&self) -> u16;

    fn direct_page_base(&self) -> u16 {
        0
    }
    fn data_bank(&self) -> u8 {
        0
    }
    fn stack_base(&self) -> u16 {
        0x0100
    }

    /// Advance cycle accounting for the current micro-op (1) plus any bus-inserted wait states.
    fn tick(&mut self, wait_states: WaitStates);

    /// Whether this context should perform the legacy NMOS dummy write during RMW sequences.
    fn rmw_dummy_write(&self) -> bool;

    /// Whether JMP (indirect) should emulate the original NMOS page-wrap bug.
    #[inline(always)]
    fn jmp_indirect_wrap_bug(&self) -> bool {
        false
    }

    /// Prepare the value that should be written to memory for store-style instructions.
    fn alu_prepare_store(&mut self, scratch: &mut MicroExecutor) -> u8;

    /// Execute the read-modify portion of a Read-Modify-Write instruction.
    fn alu_modify(&mut self, scratch: &mut MicroExecutor);

    /// Evaluate a branch, returning whether the branch was taken
    fn alu_branch(&mut self, scratch: &mut MicroExecutor, queue: &mut UcycQueue) -> bool;

    /// Produce the value to be pushed onto the stack for stack-write instructions.
    fn alu_push_value(&mut self, scratch: &mut MicroExecutor) -> u8;
}

impl MicroExecutor {
    #[inline(always)]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline(always)]
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn execute_next<C: MicroContext, B: Bus>(
        &mut self,
        ctx: &mut C,
        bus: &mut B,
        queue: &mut UcycQueue,
    ) -> StepResult {
        let Some(ucyc) = queue.pop() else {
            return StepResult::InstructionFinished;
        };
        // pre memory access alu stuff:
        let mut fixed_addr = None;
        match ucyc.alu {
            AluOp::NextOpcode => {
                // we can ignore everything else i guess, PC guaranteed ok for next opcode fetch
                return StepResult::InstructionFinished;
            }
            AluOp::OffsetWithExtraCycle(offset_type) => {
                // before the memory access, we add offset to the low byte of EA.
                // technically, this happened last cycle after memory access
                // but codewise it was easier to kinda jank it in right here, and
                // who cares what the internal latches are doing!
                let ea_lo = self.ea as u8;
                let offset = match offset_type {
                    crate::isa::OffsetType::None => 0,
                    crate::isa::OffsetType::X => ctx.reg_x(),
                    crate::isa::OffsetType::Y => ctx.reg_y(),
                };
                let target_ea = self.ea.wrapping_add(offset as u32);
                self.ea = (self.ea & !0xFF) | (ea_lo.wrapping_add(offset as u8) as u32);
                if self.ea != target_ea {
                    println!("ea {} != target ea {target_ea}", self.ea);
                    fixed_addr = Some(target_ea)
                }
            }
            _ => {}
        }

        // perform the memory access portion of this cycle:
        let addr = self.address_of(ucyc.bus.addr, ctx);
        if ucyc.bus.read {
            let (data, wait) = bus.read(addr, ucyc.bus.vda, ucyc.bus.vpa);
            self.data_latch = data;
            self.write_latch(ctx, ucyc.copy_to, data);
            ctx.tick(wait);
        } else {
            let value = ctx.alu_prepare_store(self);
            let wait = bus.write(addr, value, ucyc.bus.vda, ucyc.bus.vpa);
            ctx.tick(wait);
        }
        if ucyc.inc_src {
            self.add_offset(ctx, ucyc.bus.addr, 1);
        }

        // post memory alu stuff:
        match ucyc.alu {
            AluOp::None => {}
            AluOp::NextOpcode => unreachable!(),
            AluOp::AddOffset { latch, offset } => {
                let o = match offset {
                    super::OffsetType::None => 0,
                    super::OffsetType::X => ctx.reg_x(),
                    super::OffsetType::Y => ctx.reg_y(),
                };
                self.add_offset(ctx, latch, o);
            }
            AluOp::OffsetWithExtraCycle(_) => {
                if let Some(addr) = fixed_addr {
                    // we need to fix the address! the queue already has a read and an opcode fetch
                    // so we're done!
                    self.ea = addr;
                } else if self.memory_action == MemoryAction::Read {
                    // we can short circuit here! next cycle is opcode fetch, which ends the op
                    // and clears everything after it
                    queue.insert(MicroCycle::opcode_fetch());
                    // on writes and RMW's, we do the extra cycle even when addr is correct
                }
            }
            AluOp::IncLatch(latch) => self.add_offset(ctx, latch, 1),
            AluOp::Push => {
                ctx.set_sp(ctx.sp().wrapping_sub(1));
            }
            AluOp::Modify => {
                ctx.alu_modify(self);
            }
            AluOp::JumpToEa => {
                ctx.set_pc(self.ea as u16);
            }
            AluOp::Branch => {
                if ctx.alu_branch(self, queue) {
                    // branch taken!
                    // we need to figure out if we are branching to the same page or a different one
                    let new_pc = ctx.pc().wrapping_add_signed(self.signed_offset8 as i16);
                    let maybe_invalid = (ctx.pc() & 0xFF00) | (new_pc & 0xFF);
                    // regardless of whether the address is valid or not, we set the PC to it on this cycle:
                    queue.push(MicroCycle::read_pc_set_pc(maybe_invalid));
                    if new_pc != maybe_invalid {
                        // the address was invalid! luckily we know what the valid address is, let's spend another cycle
                        // fixing it!
                        queue.push(MicroCycle::read_pc_set_pc(new_pc));
                    }
                    // then, stick a fork in us, we're done
                    queue.push(MicroCycle::opcode_fetch());
                } else {
                    // branch not taken! next cycle is opcode fetch
                    queue.push(MicroCycle::opcode_fetch())
                }
            }
            AluOp::SetPc(new_pc) => ctx.set_pc(new_pc),
        }
        StepResult::Pending
    }

    fn address_of<C: MicroContext>(&self, latch: Latch, ctx: &C) -> u32 {
        match latch {
            Latch::Pc => ctx.pc() as u32,
            Latch::Sp => ctx.stack_base() as u32 + ctx.sp() as u32,
            Latch::Ea => self.ea,
            Latch::EaLo => self.ea & 0xFF,
            Latch::Ptr => self.ptr as u32,
            Latch::PtrLo => (self.ptr & 0xFF) as u32,
            _ => unimplemented!("{latch:?}"),
        }
    }

    fn add_offset<C: MicroContext>(&mut self, ctx: &mut C, latch: Latch, offset: u16) {
        let add_lo =
            |v: u16, o: u16| -> u16 { (v & 0xFF00) | (v as u8).wrapping_add(o as u8) as u16 };
        let add_hi = |v: u16, o: u16| -> u16 {
            (((v >> 8) as u8).wrapping_add(o as u8) as u16) << 8 | (ctx.pc() & 0xFF)
        };
        let pc = ctx.pc();
        let ea_bank = self.ea & 0xFF0000;
        let ea = self.ea as u16;
        match latch {
            Latch::Pc => ctx.set_pc(pc.wrapping_add(offset)),
            Latch::PcLo => ctx.set_pc(add_lo(pc, offset)),
            Latch::PcHi => ctx.set_pc(add_hi(pc, offset)),
            Latch::Ea => self.ea = self.ea.wrapping_add(offset as u32),
            Latch::EaLo => self.ea = ea_bank | add_lo(ea, offset) as u32,
            Latch::EaHi => self.ea = ea_bank | add_hi(ea, offset) as u32,
            Latch::Ptr => self.ptr = self.ptr.wrapping_add(offset),
            Latch::PtrLo => self.ptr = add_lo(self.ptr, offset),
            Latch::PtrHi => self.ptr = add_hi(self.ptr, offset),
            Latch::Sp => ctx.set_sp(ctx.sp().wrapping_add(offset as u8)),
            _ => unimplemented!("{latch:?}"),
        }
    }

    fn write_latch<C: MicroContext>(&mut self, ctx: &mut C, latch: super::Latch, value: u8) {
        use super::Latch::*;
        match latch {
            None => {}
            Ea => {
                self.ea = (self.ea & !0xFFFF) | value as u32;
            }
            EaLo => {
                self.ea = (self.ea & !0xFF) | value as u32;
            }
            EaHi => {
                self.ea = (self.ea & !0xFF00) | ((value as u32) << 8);
            }
            Op0 => self.op0 = value,
            Ptr => self.ptr = value as u16,
            PtrLo => {
                self.ptr = (self.ptr & 0xFF00) | value as u16;
            }
            PtrHi => {
                self.ptr = (self.ptr & 0x00FF) | ((value as u16) << 8);
            }
            Pc => {
                let current = ctx.pc();
                let new = (current & 0xFF00) | value as u16;
                ctx.set_pc(new);
            }
            PcLo => ctx.set_pc_lo(value),
            PcHi => ctx.set_pc_hi(value),
            Sp => ctx.set_sp(value),
            Status => ctx.set_status(value),
            SignedOffset8 => self.signed_offset8 = value as i8,
        }
    }
}
