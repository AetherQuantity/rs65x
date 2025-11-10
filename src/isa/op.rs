//! Shared ISA (Instruction Set Architecture) metadata used by 6502/65C02/Rockwell/65C816 cores.
//!
//! This module intentionally contains **no core-specific state** and **no function pointers**.
//! It defines *metadata* that opcode tables for different cores can reuse without duplication.
//! Each core (8‑bit or 16‑bit) can wrap these with its own exec function pointers later.

use crate::{
    bus::{Bus, WaitStates},
    isa::memory::{MemoryAction, OffsetType},
};

const MAX_UOPS: usize = 14;

/// Micro-Operations
///
/// Operations are comprised of many different Uops, which perform sub-op tasks like
/// memory access or individual ALU operations. Each Uop is one cycle USUALLY, though
/// can be zero cycles in cases where optional address-fixing stuff takes place
#[derive(Clone, Copy)]
pub enum Uop {
    // MEMORY ACCESS UOPS
    /// Reads a value from memory into a latch
    Read {
        src: super::memory::MemLoc,
        dest: super::memory::Latch,
    },
    /// Push the data from a latch to the memory location denoted by Stack Pointer. If dec,
    /// decrement the Stack Pointer.
    Push {
        src: super::memory::Latch,
        dec: bool,
    },
    /// Pull the data from the stack into a given latch. If inc, increment the Stack Pointer
    Pull {
        dest: super::memory::Latch,
        inc: bool,
    },
    /// Read from the address in latch, pre-offset. Then, add offset to that latch.
    AddOffset8 {
        latch: super::memory::Latch,
        offset_type: super::memory::address_mode_subtypes::OffsetType,
    },
    /// Fetch Effective Address High, then set PC to entire EA
    FetchEaHiAndJump,
    /// Set Program Counter to Effective Address, dummy read PC, then increment PC
    ReturnToEa,
    /// Request ALU to determine whether we branch or not, and populate uop queue further. This uop takes:
    /// - Zero cycles if branch not taken--the read we do on this cycle is the opcode for the next cycle
    /// - One cycle if branch taken to same page, a read is done regardless and is the next opcode if valid
    /// - Two cycles if branch taken to different page and opcode is read
    AluBranch,
    /// Finished: a zero-cycle uop that should function as the fetch of the next opcode
    Finished,

    // ALU stuff
    /// Write the contents of Op0 to EA
    AluWrite,
    /// Tell the ALU to modify Op0 and store the result in Op0
    ///
    /// TODO: the Rockwell documentation for the R and C variants of the 6502 claims that this uop
    ///       performs a dummy read of the EA for RMW, as opposed to the CMOS variant's dummy write of the
    ///       pre-modified value. I'll have to do some research about this.
    AluModify,
    /// Push register onto stack
    ///
    /// This Uop does two things in order. First, we push the requested register to the stack. Second,
    /// we decrement the Stack Pointer. These two things happen in the same cycle.
    AluPush,
    /// Dummy read the old PC and fix the PC
    ///
    /// Used for branches, where they occur across a page boundary
    FixPc(u16),
    /// On this cycle, if the action is Read
    ReadOrFix(super::memory::OffsetType, super::memory::MemoryAction),
}

#[derive(Clone, Copy)]
pub enum DataDest {
    Discard,
    EffectiveAddressLow,
    EffectiveAddressHigh,
    DataLatch,
}

/// A completely stack-based queue that i'm trying to make super duper
/// lightning fast since it's in the hot path
pub struct UopQueue {
    buf: [Uop; MAX_UOPS],
    head: u8, // next to execute (pop front)
    len: u8,  // number of valid entries
}

impl Default for UopQueue {
    fn default() -> Self {
        Self {
            buf: [Uop::Finished; MAX_UOPS],
            head: 0,
            len: 0,
        }
    }
}

impl UopQueue {
    #[inline(always)]
    pub fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.head == self.len
    }
    #[inline(always)]
    pub fn push(&mut self, u: Uop) {
        debug_assert!((self.len as usize) < MAX_UOPS);
        unsafe {
            *self.buf.get_unchecked_mut(self.len as usize) = u;
        }
        self.len += 1;
    }
    #[inline(always)]
    pub fn front(&self) -> Option<Uop> {
        if self.head >= self.len {
            return None;
        }
        debug_assert!(self.head < MAX_UOPS as u8);
        unsafe { Some(*self.buf.get_unchecked(self.head as usize)) }
    }
    #[inline(always)]
    pub fn pop(&mut self) -> Option<Uop> {
        let front = self.front()?;
        self.head += 1;
        if self.head >= self.len {
            self.clear();
        }
        Some(front)
    }
}

#[derive(Default, Clone)]
pub struct MicroExecutor {
    pub data_latch: u8,
    pub ea: u32,
    pub ptr: u16,
    pub op0: u8,
    pub signed_offset8: i8,
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

    /// Prepare the value that should be written to memory for store-style instructions.
    fn alu_prepare_store(&mut self, scratch: &mut MicroExecutor) -> u8;

    /// Execute the read-modify portion of a Read-Modify-Write instruction.
    fn alu_modify(&mut self, scratch: &mut MicroExecutor);

    /// Evaluate a branch, updating program counter / queue and returning penalty info.
    fn alu_branch(&mut self, scratch: &mut MicroExecutor, queue: &mut UopQueue) -> bool;

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
        queue: &mut UopQueue,
    ) -> StepResult {
        let Some(uop) = queue.pop() else {
            return StepResult::InstructionFinished;
        };

        match uop {
            Uop::Read { src, dest } => {
                let (addr, vda, vpa) = self.resolve_memloc(ctx, src);
                let (data, wait) = bus.read(addr, vda, vpa);
                self.data_latch = data;
                self.write_latch(ctx, dest, data);
                ctx.tick(wait);
                StepResult::Pending
            }
            Uop::Push { src, dec } => {
                let value = self.read_latch(ctx, src);
                let addr = (ctx.stack_base() as u32) | (ctx.sp() as u32);
                let wait = bus.write(addr, value, true, false);
                ctx.tick(wait);
                if dec {
                    ctx.set_sp(ctx.sp().wrapping_sub(1));
                }
                StepResult::Pending
            }
            Uop::Pull { dest, inc } => {
                let addr = (ctx.stack_base() as u32) | (ctx.sp() as u32);
                let (data, wait) = bus.read(addr, true, false);
                self.data_latch = data;
                self.write_latch(ctx, dest, data);
                ctx.tick(wait);
                if inc {
                    ctx.set_sp(ctx.sp().wrapping_add(1));
                }
                StepResult::Pending
            }
            Uop::AddOffset8 { latch, offset_type } => {
                let addr = match latch {
                    crate::isa::memory::Latch::Ptr => self.ptr as u32,
                    crate::isa::memory::Latch::Ea => self.ea,
                    _ => unreachable!("only ptr and ea should be adding offset"),
                };
                let offset = match offset_type {
                    OffsetType::X => ctx.reg_x(),
                    OffsetType::Y => ctx.reg_y(),
                    OffsetType::None => unreachable!(),
                };
                // dummy read at pre-offset location pointed to by latch
                let (byte, wait) = bus.read(addr, true, false);
                self.data_latch = byte;
                self.write_latch(ctx, latch, (addr as u8).wrapping_add(offset as u8));
                ctx.tick(wait);
                StepResult::Pending
            }
            Uop::ReadOrFix(offset_type, action) => {
                let offset = match offset_type {
                    OffsetType::X => ctx.reg_x(),
                    OffsetType::Y => ctx.reg_y(),
                    OffsetType::None => unreachable!(),
                };
                let new_addr = self.ea + offset as u32;
                let maybe_valid = (new_addr & 0xFF) | (self.ea & !0xFF);
                let (byte, wait) = bus.read(maybe_valid, true, false);
                self.ea = new_addr;
                self.data_latch = byte;
                ctx.tick(wait);
                if action == MemoryAction::Read && (new_addr & 0xFFFF == maybe_valid & 0xFFFF) {
                    // on a read cycle, if there's no overflow, we're done
                    self.op0 = byte;
                    StepResult::InstructionFinished
                } else {
                    // otherwise, after the read, let's fix the address and we can read/write/whatever next cycle
                    StepResult::Pending
                }
            }
            Uop::FetchEaHiAndJump => {
                let (addr, vda, vpa) = self.resolve_memloc(ctx, super::memory::MemLoc::Pc);
                let (ea_high, wait) = bus.read(addr, vda, vpa);
                let ea = ((ea_high as u16) << 8) | (self.ea as u16 & 0xFFFF);
                ctx.set_pc(ea);
                ctx.tick(wait);
                StepResult::Pending // we can't fetch opcode on this cycle unfortunately
            }
            Uop::ReturnToEa => {
                // dummy read ea before we add 1 to it
                let (addr, vda, vpa) = self.resolve_memloc(ctx, super::memory::MemLoc::Ea);
                let (_, wait) = bus.read(addr, vda, vpa);
                let ea = self.ea.wrapping_add(1) as u16;
                ctx.set_pc(ea);
                ctx.tick(wait);
                StepResult::Pending
            }
            Uop::AluModify => {
                let wait: WaitStates = if ctx.rmw_dummy_write() {
                    let addr = ((ctx.data_bank() as u32) << 16) | (self.ea & 0xFFFF);
                    bus.write(addr, self.op0, true, false)
                } else {
                    0
                };
                ctx.alu_modify(self);
                ctx.tick(wait);
                StepResult::Pending
            }
            Uop::AluWrite => {
                let value = ctx.alu_prepare_store(self);
                let addr = ((ctx.data_bank() as u32) << 16) | (self.ea & 0xFFFF);
                let wait = bus.write(addr, value, true, false);
                ctx.tick(wait);
                StepResult::Pending
            }
            Uop::AluBranch => {
                let result = ctx.alu_branch(self, queue);
                if result {
                    // at least one more cycle, alu_branch added to the
                    StepResult::Pending
                } else {
                    // branch not taken, fetch next opcode on this cycle
                    StepResult::InstructionFinished
                }
            }
            Uop::AluPush => {
                let value = ctx.alu_push_value(self);
                let addr = (ctx.stack_base() as u32) | (ctx.sp() as u32);
                let wait = bus.write(addr, value, true, false);
                ctx.tick(wait);
                ctx.set_sp(ctx.sp().wrapping_sub(1));
                StepResult::Pending
            }
            Uop::FixPc(target) => {
                let (addr, vda, vpa) = self.resolve_memloc(ctx, super::memory::MemLoc::Pc);
                let (_, wait) = bus.read(addr, vda, vpa);
                ctx.set_pc(target);
                ctx.tick(wait);
                StepResult::Pending
            }
            Uop::Finished => StepResult::InstructionFinished,
        }
    }

    fn resolve_memloc<C: MicroContext>(
        &mut self,
        ctx: &mut C,
        loc: super::memory::MemLoc,
    ) -> (u32, bool, bool) {
        use super::memory::MemLoc;
        match loc {
            MemLoc::Pc => (ctx.pc() as u32, false, true),
            MemLoc::PcInc => {
                let addr = ctx.pc();
                ctx.inc_pc();
                (addr as u32, false, true)
            }
            MemLoc::Ea => {
                let bank = (ctx.data_bank() as u32) << 16;
                (bank | (self.ea as u32 & 0xFFFF), true, false)
            }
            MemLoc::Ptr => {
                let base = ctx.direct_page_base();
                let addr = base.wrapping_add((self.ptr & 0x00FF) as u16);
                (addr as u32, true, false)
            }
            MemLoc::PtrPlusOne => {
                let base = ctx.direct_page_base();
                let addr = base.wrapping_add((self.ptr.wrapping_add(1) & 0x00FF) as u16);
                (addr as u32, true, false)
            }
            MemLoc::Sp => {
                let addr = (ctx.stack_base() as u32) | (ctx.sp() as u32);
                (addr, true, false)
            }
            MemLoc::Const(loc) => (loc as u32, true, false),
        }
    }

    fn write_latch<C: MicroContext>(
        &mut self,
        ctx: &mut C,
        latch: super::memory::Latch,
        value: u8,
    ) {
        use super::memory::Latch::*;
        match latch {
            None => {}
            Ea => {
                self.ea = (self.ea & !0xFFFF) | value as u32;
            }
            EaLo => {
                self.ea = (self.ea & 0xFF00) | value as u32;
            }
            EaHi => {
                self.ea = (self.ea & 0x00FF) | ((value as u32) << 8);
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
            Status => ctx.set_status(value),
            SignedOffset8 => self.signed_offset8 = value as i8,
        }
    }

    fn read_latch<C: MicroContext>(&self, ctx: &C, latch: super::memory::Latch) -> u8 {
        use super::memory::Latch::*;
        match latch {
            None => 0,
            Ea => (self.ea & 0xFF) as u8,
            EaLo => (self.ea & 0xFF) as u8,
            EaHi => ((self.ea >> 8) & 0xFF) as u8,
            Op0 => self.op0,
            Ptr => self.ptr as u8,
            PtrLo => (self.ptr & 0xFF) as u8,
            PtrHi => (self.ptr >> 8) as u8,
            Pc => ctx.pc() as u8,
            PcLo => ctx.pc() as u8,
            PcHi => (ctx.pc() >> 8) as u8,
            Status => ctx.status(),
            SignedOffset8 => self.signed_offset8 as u8,
        }
    }
}
