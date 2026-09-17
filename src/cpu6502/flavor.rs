//! CPU "flavor" definitions for 8‑bit 65x cores.
//!
//! This module declares a zero‑cost trait `Flavor` implemented by marker types for the
//! supported variants. The goal is to allow the hot loop to be fully monomorphized with
//! **no runtime branching** on flavor, while keeping names self‑documenting.

#![allow(dead_code)]

use crate::isa::{
    Latch,
    microop::{AluOp, BusCycle, DecodeContext, MicroCode, MicroOp, UcycQueue},
    table::OpcodeTable,
};

pub use crate::alu::DecimalSemantics;

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

    /// Extra decimal ADC-immediate cycle address observed in the SingleStepTests fixtures.
    /// This models the tested chip variants; it is not an architectural address guarantee.
    const DECIMAL_ADC_IMMEDIATE_READ: u32 = 0x00;

    /// NMOS quirk: JMP (indirect) wraps within the page (e.g., $xxFF reads high from $xx00).
    const JMP_INDIRECT_WRAP_BUG: bool;

    /// On IndirectX and IndirectY memory accesses, when the offset causes a page cross, the NMOS
    /// does a dummy read from the invalid address (i.e. old high byte + new low byte), then correct
    /// address on the next cycle. On CMOS, we dummy read from the PC instead of the invalid address.
    const INVALID_ADDR_READ: bool;

    /// Whether classic NMOS‑style dummy writes occur during RMW sequences on zero page/abs.
    /// Aapparently according to Rockwell documentation CMOS chips replace the dummy write with a read instead
    const RMW_DUMMY_WRITE: bool;

    /// Opcode decode table used by this flavor.
    const OPCODE_TABLE: OpcodeTable;
}

/// While the cycle count between the 816 and 6502 remain the same (6), the cycles actually happen in a
/// different order!
fn emit_jsr_8bit(queue: &mut UcycQueue, _ctx: DecodeContext) {
    queue.push(MicroOp::read(Latch::Pc, Latch::EaLo, true));
    // internal op:
    queue.push(MicroOp::read(Latch::Sp, Latch::None, false));
    queue.push(MicroOp::push(Latch::PcHi));
    queue.push(MicroOp::push(Latch::PcLo));
    queue.push(MicroOp {
        bus: BusCycle {
            addr: Latch::Pc,
            vda: false,
            vpa: true,
            read: true,
        },
        inc_src: false,
        local_latch: Latch::EaHi,
        alu: AluOp::SwapEaPc, // ready for next opcode fetch!
    });
}

/// Slightly different dummy reads
fn emit_rts_8bit(queue: &mut UcycQueue, _ctx: DecodeContext) {
    queue.push(MicroOp::dummy_read(Latch::Pc));
    queue.push(MicroOp::read(Latch::Sp, Latch::None, true));
    queue.push(MicroOp::read(Latch::Sp, Latch::PcLo, true));
    queue.push(MicroOp::read(Latch::Sp, Latch::PcHi, false));
    queue.push(MicroOp::read(Latch::Pc, Latch::None, true));
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
    fn emit_jmpind(queue: &mut UcycQueue, _ctx: DecodeContext) {
        queue.push(MicroOp::read(Latch::Pc, Latch::PtrLo, true));
        queue.push(MicroOp::read(Latch::Pc, Latch::PtrHi, false));
        // read Ptr, but instead of automatically incrementing it (which would result in the
        // correct high byte address), we use the custom AluOp to just increment PtrLo instead:
        queue.push(MicroOp {
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
        queue.push(MicroOp::read(Latch::Ptr, Latch::PcHi, false));
    }

    fn emit_jsr(queue: &mut UcycQueue, _ctx: DecodeContext) {
        emit_jsr_8bit(queue, _ctx);
    }

    fn emit_rts(queue: &mut UcycQueue, _ctx: DecodeContext) {
        emit_rts_8bit(queue, _ctx);
    }

    fn emit_jam(queue: &mut UcycQueue, _ctx: DecodeContext) {
        queue.push(MicroOp::read(Latch::Pc, Latch::None, false));
        queue.push(MicroOp::read(Latch::Constant(0xFFFF), Latch::None, false));
        queue.push(MicroOp::read(Latch::Constant(0xFFFE), Latch::None, false));
        queue.push(MicroOp::read(Latch::Constant(0xFFFE), Latch::None, false));
        queue.push(MicroOp {
            bus: BusCycle {
                addr: Latch::Constant(0xFFFF),
                vda: false,
                vpa: false,
                read: true,
            },
            inc_src: false,
            local_latch: Latch::None,
            alu: AluOp::Jam,
        });
        queue.push(MicroOp::dummy_read(Latch::None)); // this will never be reached, we just dont want the queue to be empty
    }
}

pub struct Micro65C02;
impl MicroCode for Micro65C02 {
    /// The CMOS65C02 JMP (IND) bugfix!
    ///
    /// The bugfix results in correct page for the pointer high byte, at the cost of
    /// one extra cycle: both JMP (IND) and JMP (IND,X) take six cycles
    fn emit_jmpind(queue: &mut UcycQueue, _ctx: DecodeContext) {
        queue.push(MicroOp::read(Latch::Pc, Latch::PtrLo, true));
        queue.push(MicroOp::read(Latch::Pc, Latch::PtrHi, false));
        queue.push(MicroOp::read_ptr1(Latch::PcLo));
        queue.push(MicroOp::read_inv_ptr2(Latch::PcHi));
        queue.push(MicroOp::read(Latch::Ptr, Latch::PcHi, false));
    }

    fn emit_jsr(queue: &mut UcycQueue, _ctx: DecodeContext) {
        emit_jsr_8bit(queue, _ctx);
    }

    fn emit_rts(queue: &mut UcycQueue, _ctx: DecodeContext) {
        emit_rts_8bit(queue, _ctx);
    }

    fn emit_cmos_nop(queue: &mut UcycQueue, _ctx: DecodeContext, bytes: u8, mut cycles: u8) {
        // Synertek's three-cycle NOP consumes both operand bytes without a dummy read
        if bytes == 3 && cycles == 3 {
            queue.push(MicroOp::read(Latch::Pc, Latch::None, true));
            queue.push(MicroOp::read(Latch::Pc, Latch::None, true));
            return;
        }
        // Otherwise, WDC and Rockwell (and non CB/DB Synertek NOPs) only inc Pc every other cycle
        let mut i = 0;
        while cycles > 1 {
            cycles -= 1;
            queue.push(MicroOp::read(Latch::Pc, Latch::None, i == 0 || i == 2));
            i += 1;
        }
        // TODO: we should probably do this better
    }
}
pub static MICRO_6502: Micro6502 = Micro6502;
pub static MICRO_65C02: Micro65C02 = Micro65C02;

/// Marker for the original NMOS 6502.
pub enum NMOS6502 {}
/// Marker for the WDC 65C02 instruction set, including Rockwell bit operations and WAI/STP.
pub enum WDC65C02 {}
/// Marker for the Synertek 65C02. No WAI/STP, no Rockwell bit operations.
pub enum Synertek65C02 {}
/// Marker for the Rockwell 65C02. No WAI/STP.
pub enum Rockwell65C02 {}
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
    const INVALID_ADDR_READ: bool = true;
    const RMW_DUMMY_WRITE: bool = true;
    const OPCODE_TABLE: OpcodeTable = OpcodeTable::Nmos;
}

impl Flavor for WDC65C02 {
    type Micro = Micro65C02;
    fn microcode(&self) -> &'static Self::Micro {
        &MICRO_65C02
    }
    const NAME: &'static str = "WDC65C02";
    const DECIMAL: DecimalSemantics = DecimalSemantics::Cmos65C02;
    const DECIMAL_ADC_IMMEDIATE_READ: u32 = 0x007F;
    const JMP_INDIRECT_WRAP_BUG: bool = false; // fixed on CMOS
    const INVALID_ADDR_READ: bool = false;
    const RMW_DUMMY_WRITE: bool = false;
    const OPCODE_TABLE: OpcodeTable = OpcodeTable::Wdc65c02;
}

impl Flavor for Synertek65C02 {
    type Micro = Micro65C02;
    fn microcode(&self) -> &'static Self::Micro {
        &MICRO_65C02
    }
    const NAME: &'static str = "Synertek65C02";
    const DECIMAL: DecimalSemantics = DecimalSemantics::Cmos65C02;
    const DECIMAL_ADC_IMMEDIATE_READ: u32 = 0x0056;
    const JMP_INDIRECT_WRAP_BUG: bool = false; // fixed on CMOS
    const INVALID_ADDR_READ: bool = false;
    const RMW_DUMMY_WRITE: bool = false;
    const OPCODE_TABLE: OpcodeTable = OpcodeTable::Synertek;
}

impl Flavor for Rockwell65C02 {
    type Micro = Micro65C02;
    fn microcode(&self) -> &'static Self::Micro {
        &MICRO_65C02
    }
    const NAME: &'static str = "Rockwell65C02";
    const DECIMAL: DecimalSemantics = DecimalSemantics::Cmos65C02;
    const DECIMAL_ADC_IMMEDIATE_READ: u32 = 0x0059;
    const JMP_INDIRECT_WRAP_BUG: bool = false; // fixed on CMOS
    const INVALID_ADDR_READ: bool = false;
    const RMW_DUMMY_WRITE: bool = false;
    const OPCODE_TABLE: OpcodeTable = OpcodeTable::Rockwell;
}

impl Flavor for NES {
    type Micro = Micro6502;
    fn microcode(&self) -> &'static Self::Micro {
        &MICRO_6502
    }
    const NAME: &'static str = "Ricoh 2A03/2A07";
    const DECIMAL: DecimalSemantics = DecimalSemantics::None;
    const JMP_INDIRECT_WRAP_BUG: bool = true;
    const INVALID_ADDR_READ: bool = true;
    const RMW_DUMMY_WRITE: bool = true;
    const OPCODE_TABLE: OpcodeTable = OpcodeTable::Nmos;
}
