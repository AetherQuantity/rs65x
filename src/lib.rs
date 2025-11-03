pub mod bus;
pub mod cpu6502;
pub mod isa;

// Processor Status flag bits
// ============================
// These constants are shared across variants. Note that bit assignments for
// M/X vs B differ between 65C816 native and 8-bit parts. We expose both sets.

/// Common flags (same bit positions across families where applicable)
pub(crate) mod psr {
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
