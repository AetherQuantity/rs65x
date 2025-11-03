//! Common, zero-cost primitives shared by 6502/65C02/65C816 cores.
//!
//! Keep this module *very* small and stable. It defines:
//! - The bus interface used by all CPU cores (8- and 16-bit).
//! - Side-band line signals sampled once per cycle.
//! - Shared flag bit constants and tiny helpers that inline in hot paths.
//!
//! Design goals:
//! - No heap allocations.
//! - No dynamic dispatch in the hot path (traits are monomorphized).
//! - All helpers `#[inline(always)]` and portable (no `unsafe` here).

#![allow(dead_code)]

// ============================
// Processor Status flag bits
// ============================
// These constants are shared across variants. Note that bit assignments for
// M/X vs B differ between 65C816 native and 8-bit parts. We expose both sets.

/// Common flags (same bit positions across families where applicable)
pub mod psr {
    pub const N: u8 = 0b1000_0000; // Negative
    pub const V: u8 = 0b0100_0000; // Overflow
    // 0b0010_0000 and 0b0001_0000 are context-dependent (see below)
    pub const D: u8 = 0b0000_1000; // Decimal
    pub const I: u8 = 0b0000_0100; // IRQ disable
    pub const Z: u8 = 0b0000_0010; // Zero
    pub const C: u8 = 0b0000_0001; // Carry

    /// 6502/65C02 meaning (when a copy of P is pushed/pulled):
    /// Bit 4 is BRK (B), bit 5 is typically set in pushes (historical)
    pub const B_6502: u8 = 0b0001_0000;
    pub const U_6502: u8 = 0b0010_0000; // Unused/always set in pushes on some parts

    /// 65C816 native meaning: bit 5 = M (accumulator/memory width), bit 4 = X (index width)
    pub const M_816: u8 = 0b0010_0000;
    pub const X_816: u8 = 0b0001_0000;
}

// ============================
// Tiny helpers (inline always)
// ============================

/// Update Z and N based on an 8-bit result.
#[inline(always)]
pub fn set_zn8(p: &mut u8, value: u8) {
    use psr::{N, Z};
    // Clear N and Z, then set from value.
    let z = ((value == 0) as u8) << 1; // Z is bit 1
    *p = (*p & !(N | Z)) | (value & N) | z;
}

/// Update Z and N based on a 16-bit result (for 65C816 when M/X=0).
#[inline(always)]
pub fn set_zn16(p: &mut u8, value: u16) {
    use psr::{N, Z};
    let n = ((value & 0x8000) != 0) as u8 * N; // replicate bit 15 into N
    let z = ((value == 0) as u8) << 1; // Z is bit 1
    *p = (*p & !(N | Z)) | n | z;
}

/// Compute branch penalty (+1 if taken, +1 if page crossed). Shared default.
#[inline(always)]
pub fn default_branch_penalty(taken: bool, page_cross: bool) -> u8 {
    (taken as u8) + (page_cross as u8)
}
