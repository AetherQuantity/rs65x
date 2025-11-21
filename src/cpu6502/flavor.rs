//! CPU "flavor" definitions for 8‑bit 65x cores.
//!
//! This module declares a zero‑cost trait `Flavor` implemented by marker types for the
//! supported variants. The goal is to allow the hot loop to be fully monomorphized with
//! **no runtime branching** on flavor, while keeping names self‑documenting.

#![allow(dead_code)]

use crate::isa::{
    Latch, OffsetType,
    microcycle::{AluOp, BusCycle, DecodeContext, MicroCode, MicroCycle, UcycQueue},
    table::OpcodeTable,
};

/// Decimal (BCD) arithmetic semantics used by ADC/SBC when the D flag is set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecimalSemantics {
    /// NES Ricoh chip without any DEC behavior at all
    None,
    /// NMOS 6502 behavior (original 6502 rules)
    Nmos6502,
    /// CMOS behavior (65C02, WDC, etc.)
    Cmos65C02,
}

/// Compile‑time flavor contract for a 6502‑family core.
///
/// All associated constants are used to **select code paths at compile time**, not at
/// runtime. The compiler will inline and DCE (dead‑code‑eliminate) unused branches.
pub trait Flavor {
    type Micro: MicroCode;
    fn microcode(&self) -> &Self::Micro;

    /// Human‑readable name for logs and asserts.
    const NAME: &'static str;

    /// Decimal (BCD) behavior when the D flag is set.
    const DECIMAL: DecimalSemantics;

    /// NMOS quirk: JMP (indirect) wraps within the page (e.g., $xxFF reads high from $xx00).
    const JMP_INDIRECT_WRAP_BUG: bool;

    /// Whether classic NMOS‑style dummy writes occur during RMW sequences on zero page/abs.
    /// Aapparently according to Rockwell documentation CMOS chips replace the dummy write with a read instead
    const RMW_DUMMY_WRITE: bool;

    /// Additional opcodes present in CMOS 65C02 and/or WDC variants.
    const HAS_BRA: bool; // Branch Always (relative)
    const HAS_STZ: bool; // Store Zero
    const HAS_WAI_STP: bool; // Wait‑for‑Interrupt / Stop

    /// Rockwell bit manipulation extensions.
    const HAS_ROCKWELL_OPS: bool; // RMB, SMB, BBR, BBS: setting, resetting, and testing bits in zp

    /// Opcode decode table used by this flavor.
    const OPCODE_TABLE: OpcodeTable;
}

/// While the cycle count between the 816 and 6502 remain the same (6), the cycles actually happen in a
/// different order!
fn emit_jsr_8bit(queue: &mut UcycQueue, _ctx: DecodeContext) {
    queue.push(MicroCycle::read(Latch::Pc, Latch::EaLo, true));
    // internal op:
    queue.push(MicroCycle::read(Latch::Sp, Latch::None, false));
    queue.push(MicroCycle::push(Latch::PcHi));
    queue.push(MicroCycle::push(Latch::PcLo));
    queue.push(MicroCycle {
        bus: BusCycle {
            addr: Latch::Pc,
            vda: false,
            vpa: true,
            read: true,
        },
        inc_src: false,
        local_latch: Latch::EaHi,
        alu: AluOp::JumpToEa, // ready for next opcode fetch!
    });
    // then, PC is already rarin to go
    queue.push(MicroCycle::opcode_fetch());
}

/// Slightly different dummy reads
fn emit_rts_8bit(queue: &mut UcycQueue, _ctx: DecodeContext) {
    queue.push(MicroCycle::dummy_read(Latch::Pc));
    queue.push(MicroCycle::read(Latch::Sp, Latch::None, true));
    queue.push(MicroCycle::read(Latch::Sp, Latch::PcLo, true));
    queue.push(MicroCycle::read(Latch::Sp, Latch::PcHi, false));
    queue.push(MicroCycle::read(Latch::Pc, Latch::None, true));
    queue.push(MicroCycle::opcode_fetch());
}

pub struct Micro6502;
impl MicroCode for Micro6502 {
    /// The infamous NMOS6502 JMP (IND) wraparound bug!
    ///
    /// When the low byte of the jmp pointer is on a page boundary, i.e. xxFF, the high byte
    /// is fetched from xx00, i.e., the same page as the low byte, rather than xy00.
    ///
    /// note: we don't need to deal with OffsetType as it is always None: there is no
    /// such thing as JMP (IND,X) on the NMOS6502. It was introduced on the CMOS variants
    fn emit_jmpind(queue: &mut UcycQueue, _ctx: DecodeContext, _offset: OffsetType) {
        queue.push(MicroCycle::read(Latch::Pc, Latch::PtrLo, true));
        queue.push(MicroCycle::read(Latch::Pc, Latch::PtrHi, false));
        // read Ptr, but instead of automatically incrementing it (which would result in the
        // correct high byte address), we use the custom AluOp to just increment PtrLo instead:
        queue.push(MicroCycle {
            bus: BusCycle {
                addr: Latch::Ptr,
                vda: true, // 6502 doesnt have vda/vpa who cares
                vpa: false,
                read: true,
            },
            inc_src: false, // would be 16-bit inc, because src is 16-bit Latch::Ptr
            local_latch: Latch::PcLo,
            alu: AluOp::IncLatch(Latch::PtrLo), // wraps around on Lo byte, does not affect Hi
        });
        queue.push(MicroCycle::read(Latch::Ptr, Latch::PcHi, false));
        queue.push(MicroCycle::opcode_fetch());
    }

    fn emit_jsr(queue: &mut UcycQueue, _ctx: DecodeContext) {
        emit_jsr_8bit(queue, _ctx);
    }

    fn emit_rts(queue: &mut UcycQueue, _ctx: DecodeContext) {
        emit_rts_8bit(queue, _ctx);
    }
}

pub struct Micro65C02;
impl MicroCode for Micro65C02 {
    /// The CMOS65C02 JMP (IND) bugfix!
    ///
    /// The bugfix results in correct page for the pointer high byte, at the cost of
    /// one extra cycle: both JMP (IND) and JMP (IND,X) take six cycles
    fn emit_jmpind(queue: &mut UcycQueue, _ctx: DecodeContext, offset: OffsetType) {
        queue.push(MicroCycle::read(Latch::Pc, Latch::PtrLo, true));
        queue.push(MicroCycle::read(Latch::Pc, Latch::PtrHi, false));
        // dummy read while adding offset:
        queue.push(MicroCycle {
            bus: BusCycle {
                addr: Latch::Pc,
                vda: false,
                vpa: true,
                read: true,
            },
            inc_src: false,
            local_latch: Latch::None,
            alu: AluOp::AddOffset {
                latch: Latch::Ptr,
                offset,
            },
        });
        queue.push(MicroCycle::read(Latch::Ptr, Latch::PcLo, true));
        queue.push(MicroCycle::read(Latch::Ptr, Latch::PcHi, false));
        queue.push(MicroCycle::opcode_fetch());
    }

    fn emit_jsr(queue: &mut UcycQueue, _ctx: DecodeContext) {
        emit_jsr_8bit(queue, _ctx);
    }

    fn emit_rts(queue: &mut UcycQueue, _ctx: DecodeContext) {
        emit_rts_8bit(queue, _ctx);
    }
}
pub static MICRO_6502: Micro6502 = Micro6502;
pub static MICRO_65C02: Micro65C02 = Micro65C02;

/// Marker for the original NMOS 6502.
pub enum NMOS6502 {}
/// Marker for the baseline CMOS 65C02 (without Rockwell extensions).
pub enum CMOS65C02 {}
/// Marker for NES CPU Ricoh 2A03/2A07, with BCD nonsense removed
pub enum NES {}

impl Flavor for NMOS6502 {
    type Micro = Micro6502;
    fn microcode(&self) -> &'static Self::Micro {
        &MICRO_6502
    }
    const NAME: &'static str = "NMOS6502";
    const DECIMAL: DecimalSemantics = DecimalSemantics::Nmos6502;
    const JMP_INDIRECT_WRAP_BUG: bool = true;
    const RMW_DUMMY_WRITE: bool = true;
    const HAS_BRA: bool = false;
    const HAS_STZ: bool = false;
    const HAS_WAI_STP: bool = false;
    const HAS_ROCKWELL_OPS: bool = false;
    const OPCODE_TABLE: OpcodeTable = OpcodeTable::Nmos;
}

impl Flavor for CMOS65C02 {
    type Micro = Micro65C02;
    fn microcode(&self) -> &'static Self::Micro {
        &MICRO_65C02
    }
    const NAME: &'static str = "CMOS65C02";
    const DECIMAL: DecimalSemantics = DecimalSemantics::Cmos65C02;
    const JMP_INDIRECT_WRAP_BUG: bool = false; // fixed on CMOS
    const RMW_DUMMY_WRITE: bool = false;
    const HAS_BRA: bool = true;
    const HAS_STZ: bool = true;
    const HAS_WAI_STP: bool = true;
    const HAS_ROCKWELL_OPS: bool = true;
    const OPCODE_TABLE: OpcodeTable = OpcodeTable::Cmos;
}

impl Flavor for NES {
    type Micro = Micro6502;
    fn microcode(&self) -> &'static Self::Micro {
        &MICRO_6502
    }
    const NAME: &'static str = "Ricoh 2A03/2A07";
    const DECIMAL: DecimalSemantics = DecimalSemantics::None;
    const JMP_INDIRECT_WRAP_BUG: bool = true;
    const RMW_DUMMY_WRITE: bool = true;
    const HAS_BRA: bool = false;
    const HAS_STZ: bool = false;
    const HAS_WAI_STP: bool = false;
    const HAS_ROCKWELL_OPS: bool = false;
    const OPCODE_TABLE: OpcodeTable = OpcodeTable::Nmos;
}
