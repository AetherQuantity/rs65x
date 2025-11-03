//! CPU "flavor" definitions for 8‑bit 65x cores.
//!
//! This module declares a zero‑cost trait `Flavor` implemented by marker types for the
//! supported variants. The goal is to allow the hot loop to be fully monomorphized with
//! **no runtime branching** on flavor, while keeping names self‑documenting.

#![allow(dead_code)]

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
    /// Human‑readable name for logs and asserts.
    const NAME: &'static str;

    /// Decimal (BCD) behavior when the D flag is set.
    const DECIMAL: DecimalSemantics;

    /// NMOS quirk: JMP (indirect) wraps within the page (e.g., $xxFF reads high from $xx00).
    const JMP_INDIRECT_WRAP_BUG: bool;

    /// Whether classic NMOS‑style dummy writes occur during RMW sequences on zero page/abs.
    /// apparently according to Rockwell documentation CMOS chips replace the dummy write with a read instead
    const RMW_DUMMY_WRITE: bool;

    /// Additional opcodes present in CMOS 65C02 and/or WDC variants.
    const HAS_BRA: bool; // Branch Always (relative)
    const HAS_STZ: bool; // Store Zero
    const HAS_WAI_STP: bool; // Wait‑for‑Interrupt / Stop

    /// Rockwell bit manipulation extensions.
    const HAS_ROCKWELL_OPS: bool; // RMB, SMB, BBR, BBS: setting, resetting, and testing bits in zp

    /// Branch penalty model: +1 cycle if branch taken, +1 if page boundary crossed.
    /// Many real cores follow this, but flavors can override if needed.
    #[inline(always)]
    fn branch_penalty(taken: bool, page_cross: bool) -> u8 {
        (taken as u8) + (page_cross as u8)
    }
}

/// Marker for the original NMOS 6502.
pub enum NMOS6502 {}
/// Marker for the baseline CMOS 65C02 (without Rockwell extensions).
pub enum CMOS65C02 {}
/// Marker for a 65C02 with Rockwell bit‑ops enabled.
pub enum Rockwell65C02 {}
/// Marker for NES CPU Ricoh 2A03/2A07, with BCD nonsense removed
pub enum NES {}

impl Flavor for NMOS6502 {
    const NAME: &'static str = "NMOS6502";
    const DECIMAL: DecimalSemantics = DecimalSemantics::Nmos6502;
    const JMP_INDIRECT_WRAP_BUG: bool = true;
    const RMW_DUMMY_WRITE: bool = true;
    const HAS_BRA: bool = false;
    const HAS_STZ: bool = false;
    const HAS_WAI_STP: bool = false;
    const HAS_ROCKWELL_OPS: bool = false;
}

impl Flavor for CMOS65C02 {
    const NAME: &'static str = "CMOS65C02";
    const DECIMAL: DecimalSemantics = DecimalSemantics::Cmos65C02;
    const JMP_INDIRECT_WRAP_BUG: bool = false; // fixed on CMOS
    const RMW_DUMMY_WRITE: bool = false;
    const HAS_BRA: bool = true;
    const HAS_STZ: bool = true;
    const HAS_WAI_STP: bool = true;
    const HAS_ROCKWELL_OPS: bool = false;
}

impl Flavor for Rockwell65C02 {
    const NAME: &'static str = "Rockwell65C02";
    const DECIMAL: DecimalSemantics = DecimalSemantics::Cmos65C02;
    const JMP_INDIRECT_WRAP_BUG: bool = false;
    const RMW_DUMMY_WRITE: bool = false;
    const HAS_BRA: bool = true;
    const HAS_STZ: bool = true;
    const HAS_WAI_STP: bool = true;
    const HAS_ROCKWELL_OPS: bool = false;
}

impl Flavor for NES {
    const NAME: &'static str = "Ricoh 2A03/2A07";
    const DECIMAL: DecimalSemantics = DecimalSemantics::None;
    const JMP_INDIRECT_WRAP_BUG: bool = true;
    const RMW_DUMMY_WRITE: bool = true;
    const HAS_BRA: bool = false;
    const HAS_STZ: bool = false;
    const HAS_WAI_STP: bool = false;
    const HAS_ROCKWELL_OPS: bool = false;
}
