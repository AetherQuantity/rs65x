//! Shared ISA (Instruction Set Architecture) metadata used by 6502/65C02/Rockwell/65C816 cores.
//!
//! This module intentionally contains **no core-specific state** and **no function pointers**.
//! It defines *metadata* that opcode tables for different cores can reuse without duplication.
//! Each core (8‑bit or 16‑bit) can wrap these with its own exec function pointers later.

use log::{debug, trace};

use crate::{
    bus::Bus,
    isa::{
        Latch, MemoryAction,
        microop::{AluOp, MicroOp, UcycQueue},
    },
    psr,
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

#[derive(Debug)]
pub enum StepResult {
    Pending,
    DoOpcodeFetch,
}

pub trait MicroContext {
    fn pc(&self) -> u16;
    fn set_pc(&mut self, value: u16);
    fn set_pc_hi(&mut self, value: u8);
    fn set_pc_lo(&mut self, value: u8);
    fn inc_pc(&mut self);

    fn opcode(&self) -> u8;

    fn sp(&self) -> u8;
    fn set_sp(&mut self, value: u8);

    fn status(&self) -> u8;
    fn set_status(&mut self, value: u8);

    fn reg_x(&self) -> u16;
    fn reg_y(&self) -> u16;
    fn reg_a(&self) -> u16;

    fn direct_page_base(&self) -> u16 {
        0
    }
    fn data_bank(&self) -> u8 {
        0
    }
    fn stack_base(&self) -> u16 {
        0x0100
    }

    /// Whether this context should perform the legacy NMOS dummy write during RMW sequences.
    fn rmw_dummy_write(&self) -> bool;

    /// Whether JMP (indirect) should emulate the original NMOS page-wrap bug.
    #[inline(always)]
    fn jmp_indirect_wrap_bug(&self) -> bool {
        false
    }

    #[inline(always)]
    fn read_invalid_on_page_cross(&self) -> bool {
        false
    }

    /// Prepare the value that should be written to memory for store-style instructions, and store in data latch.
    fn alu_prepare_store(&mut self, scratch: &mut MicroExecutor);

    /// Execute the read-modify portion of a Read-Modify-Write instruction.
    fn alu_modify(&mut self, scratch: &mut MicroExecutor);

    /// Evaluate a branch, returning whether the branch was taken
    fn alu_branch(&mut self, scratch: &mut MicroExecutor, queue: &mut UcycQueue) -> bool;
}

impl MicroExecutor {
    #[inline(always)]
    pub fn new() -> Self {
        Self {
            ea: 0x7F,
            ..Default::default()
        }
    }

    #[inline(always)]
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn execute_next<C: MicroContext, B: Bus>(
        &mut self,
        ctx: &mut C,
        bus: &mut B,
        queue: &mut UcycQueue,
    ) -> StepResult {
        let Some(ucyc) = queue.pop() else {
            return StepResult::DoOpcodeFetch;
        };
        // pre memory access alu stuff:
        let mut fixed_addr = None;
        let mut subtract_one = false;
        if !ucyc.bus.read {
            // prepare data_bus with the data to write, if any
            ctx.alu_prepare_store(self);
        }
        match ucyc.alu {
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

                // "hardware" way: only change low byte, keep high as-is
                self.ea = (self.ea & !0xFF) | (ea_lo.wrapping_add(offset as u8) as u32);
                let page_crossed = self.ea != target_ea;

                // 65C02 quirk: INC/DEC abs,X always do the extra dummy cycle at PC+2
                let opcode = ctx.opcode();
                let is_incdec_abs_x = matches!(opcode, 0xDE | 0xFE);

                let need_dummy_this_cycle = page_crossed
                    || self.memory_action == MemoryAction::Write
                    || (self.memory_action == MemoryAction::ReadModifyWrite && is_incdec_abs_x);

                if need_dummy_this_cycle {
                    debug!("ea {:#06X} != target ea {target_ea:#06X}", self.ea);
                    if !ctx.read_invalid_on_page_cross() {
                        // on CMOS chips we don't do a read of the invalid location!
                        // instead, we read the current pc again (PC+2)
                        self.ea = (ctx.pc() as u32).wrapping_sub(1);
                    }
                    fixed_addr = Some(target_ea);
                }
            }
            AluOp::SpecialAddOffset => {
                // oh my goodness.
                // so, JMP ABS,X (CMOS only), apparently needs to make PC go BACKWARDS for a cycle
                // for some dummy read it needs to do. wowzer.
                subtract_one = true;
            }
            _ => {}
        }

        // perform the memory access portion of this cycle:
        let addr = self.address_of(ucyc.bus.addr, ctx);
        let addr = if subtract_one {
            addr.wrapping_sub(1)
        } else {
            addr
        };
        if ucyc.bus.read {
            let data = bus.read(addr, ucyc.bus.vda, ucyc.bus.vpa);
            self.data_latch = data;
            self.write_latch(ctx, ucyc.local_latch, data);
        } else {
            if !matches!(ucyc.local_latch, Latch::None) {
                self.data_latch = self.read_latch(ctx, ucyc.local_latch);
            }
            bus.write(addr, self.data_latch, ucyc.bus.vda, ucyc.bus.vpa);
        }
        if ucyc.inc_src {
            self.add_offset(ctx, ucyc.bus.addr, 1);
        }

        // post memory alu stuff:
        match ucyc.alu {
            AluOp::None => {}
            AluOp::AddOffset { latch, offset } => {
                let o = match offset {
                    super::OffsetType::None => 0,
                    super::OffsetType::X => ctx.reg_x(),
                    super::OffsetType::Y => ctx.reg_y(),
                };
                self.add_offset(ctx, latch, o);
            }
            AluOp::SpecialAddOffset => self.add_offset(ctx, Latch::Ea, ctx.reg_x()),
            AluOp::OffsetWithExtraCycle(_) => {
                let opcode = ctx.opcode();
                let is_incdec_abs_x = matches!(opcode, 0xDE | 0xFE);

                if let Some(addr) = fixed_addr {
                    // we had a dummy this cycle (page cross / write / special INC/DEC case)
                    // now restore the "real" EA for the following cycles
                    self.ea = addr;
                } else if self.memory_action == MemoryAction::Read {
                    // we can short circuit here! next cycle is opcode fetch, which ends the op
                    // and clears everything after it
                    queue.clear();
                } else if self.memory_action == MemoryAction::ReadModifyWrite
                    && !ctx.read_invalid_on_page_cross()
                    && !is_incdec_abs_x
                {
                    // on CMOS, for *normal* RMW (ASL/LSR/ROL/ROR abs,X) with no page-cross:
                    // we just read the correct EA, so we can skip the extra read phase.
                    trace!("popping first of queue (len {} pre-pop", queue.len());
                    queue.pop();
                }
            }
            AluOp::IncLatch(latch) => self.add_offset(ctx, latch, 1),
            AluOp::DecSp => {
                ctx.set_sp(ctx.sp().wrapping_sub(1));
            }
            AluOp::Modify => {
                ctx.alu_modify(self);
            }
            AluOp::SwapEaPc => {
                let temp = ctx.pc();
                ctx.set_pc(self.ea as u16);
                self.ea = temp as u32;
            }
            AluOp::Branch => {
                if ctx.alu_branch(self, queue) {
                    // branch taken!
                    trace!("branch taken!");
                    // we need to figure out if we are branching to the same page or a different one
                    let new_pc = ctx.pc().wrapping_add_signed(self.signed_offset8 as i16);
                    let maybe_invalid = (ctx.pc() & 0xFF00) | (new_pc & 0xFF);
                    // regardless of whether the address is valid or not, we set the PC to it on this cycle:

                    if new_pc != maybe_invalid {
                        // the address was invalid! luckily we know what the valid address is, let's spend another cycle
                        // fixing it!
                        if ctx.opcode() & 0x0F == 0x0F {
                            // on CMOS BBR/BBS instructions we read the PC again as we fix the Ea internally
                            queue.push(MicroOp::read(Latch::Pc, Latch::None, false));
                        } else {
                            // otherwise, we read the invalid locations
                            queue.push(MicroOp::read_pc_set_pc(maybe_invalid));
                        }
                        queue.push(MicroOp::read_pc_set_pc(new_pc));
                    } else {
                        queue.push(MicroOp::read_pc_set_pc(new_pc));
                    }
                    // then, stick a fork in us, we're done
                } else {
                    trace!("branch not taken!");
                    // branch not taken! next cycle is opcode fetch
                    queue.clear();
                }
            }
            AluOp::SetPc(new_pc) => ctx.set_pc(new_pc),
            AluOp::Jam => {
                // decreasing the cursor in the queue guarantees that this same uCycle will be
                // executed next cycle
                queue.dec_head();
            }
            AluOp::FixPtr => {
                if self.ptr & 0xFF == 0 {
                    // we overflowed when we inc'd!! we need to add 0x100 to ptr to get to
                    // the right addr
                    self.ptr += 0x100;
                } else if ctx.opcode() == 0x7C {
                    // only on JMP ABS,X: this cycle only executes if needed
                    queue.clear();
                }
            }
        }
        StepResult::Pending
    }

    fn address_of<C: MicroContext>(&self, latch: Latch, ctx: &C) -> u32 {
        match latch {
            Latch::Constant(c) => c as u32,
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
            None | Constant(_) => {}
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
            BrkStatus => ctx.set_status(value | psr::B_6502 | psr::U_6502),
            SignedOffset8 => self.signed_offset8 = value as i8,
        }
    }

    fn read_latch<C: MicroContext>(&mut self, ctx: &mut C, latch: super::Latch) -> u8 {
        use super::Latch::*;
        match latch {
            None => 0,
            Constant(c) => c as u8,
            Ea | EaLo => self.ea as u8,
            EaHi => (self.ea >> 8) as u8,
            Op0 => self.op0,
            Ptr | PtrLo => self.ptr as u8,
            PtrHi => (self.ptr >> 8) as u8,
            Pc | PcLo => ctx.pc() as u8,
            PcHi => (ctx.pc() >> 8) as u8,
            Sp => ctx.sp(),
            Status => ctx.status(),
            BrkStatus => ctx.status() | psr::B_6502 | psr::U_6502,
            SignedOffset8 => u8::from_le_bytes(self.signed_offset8.to_le_bytes()),
        }
    }
}
