//! Shared ISA (Instruction Set Architecture) metadata used by 6502/65C02/Rockwell/65C816 cores.
//!
//! This module intentionally contains **no core-specific state** and **no function pointers**.
//! It defines *metadata* that opcode tables for different cores can reuse without duplication.
//! Each core (8‑bit or 16‑bit) can wrap these with its own exec function pointers later.

#![allow(dead_code)]

/// Addressing modes across the 65x family (superset; specific cores use a subset).
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddrMode {
    /// Implied/Accumulator (no operand fetch; may use A directly)
    Imp, // e.g., CLC, INX, ASL A
    /// Immediate 8‑bit (operand is next byte)
    Imm8, // e.g., LDA #imm
    /// Zero page (direct) addressing
    Zp,
    /// Zero page,X indexed
    ZpX,
    /// Zero page,Y indexed (some ops)
    ZpY,
    /// Absolute 16‑bit
    Abs,
    /// Absolute,X indexed
    AbsX,
    /// Absolute,Y indexed
    AbsY,
    /// (indirect) using 16‑bit pointer (JMP (abs) et al.)
    Ind,
    /// (zp,X) pre‑indexed indirect
    IndX,
    /// (zp),Y post‑indexed indirect
    IndY,
    /// Relative (branch displacement)
    Rel,
    /// Zero page + relative (Rockwell: BBR/BBS)
    ZpRel,
    // ===== 65C816 extensions (native mode) =====
    /// 24‑bit absolute (bank:address)
    Long,
    /// 24‑bit absolute,X
    LongX,
    /// Stack‑relative (S + disp)
    Sr,
    /// (Stack‑relative, indirect, indexed by Y)
    SrIndY,
    /// Direct page (DP) a.k.a. zero page with relocatable base
    Dir,
    /// Direct page,X
    DirX,
    /// (Direct), indirect (DP)
    DirInd,
    /// (Direct), indirect, indexed by Y
    DirIndY,
    /// (Absolute), long indirect
    AbsLongInd,
}

const MAX_UOPS: usize = 14;

/// Micro-Operations
///
/// Operations are comprised of many different Uops, which perform sub-op tasks like
/// memory access or individual ALU operations. Each Uop is one cycle USUALLY, though
/// can be zero cycles in cases where optional address-fixing stuff takes place
#[derive(Clone, Copy)]
pub enum Uop {
    // MEMORY ACCESS UOPS
    Read {
        src: super::memory::MemLoc,
        dest: super::memory::Latch,
    },
    ReadNext {
        src: super::memory::MemLoc,
        dest: super::memory::Latch,
        wrap_mode: super::memory::WrapMode,
    },
    Push {
        src: super::memory::Latch,
        dec: bool,
    },
    Pull {
        dest: super::memory::Latch,
        inc: bool,
    },
    AddOffset {
        latch: super::memory::Latch,
        offset_type: super::memory::address_mode_subtypes::OffsetType,
        wrap_mode: super::memory::WrapMode,
    },
    /// Fetch Effective Address High, then set PC to entire EA
    FetchEaHiAndJump,
    /// Set Program Counter to Effective Address, then increment PC
    ReturnToEa,
    /// Read 0xFFFE directly into PC Low
    IntFFFE,
    /// Read 0xFFFF directly into PC High
    IntFFFF,
    /// Request ALU to determine whether we branch or not, and populate uop queue further
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
}

#[derive(Clone, Copy)]
pub enum DataDest {
    Discard,
    EffectiveAddressLow,
    EffectiveAddressHigh,
    DataLatch,
}

pub struct UopQueue {
    buf: [Uop; MAX_UOPS],
    head: u8, // next to execute (pop front)
    len: u8,  // number of valid entries
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
    pub fn front(&self) -> Uop {
        debug_assert!(self.head < MAX_UOPS as u8 && self.head < self.len);
        unsafe { *self.buf.get_unchecked(self.head as usize) }
    }
    #[inline(always)]
    pub fn advance(&mut self) {
        debug_assert!(self.head < self.len - 1);
        self.head += 1;
    }
}
