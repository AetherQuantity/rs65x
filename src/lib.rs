pub mod bus;
pub mod cpu6502;
pub mod isa;

#[allow(dead_code)]
// Processor Status flag bits
// ============================
// These constants are shared across variants. Note that bit assignments for
// M/X vs B differ between 65C816 native and 8-bit parts. We expose both sets.

/// Common flags (same bit positions across families where applicable)
pub(crate) mod psr {
    /// N: Negative
    pub const N: u8 = 0b1000_0000; // Negative
    /// V: Overflow
    pub const V: u8 = 0b0100_0000; // Overflow
    // 0b0010_0000 and 0b0001_0000 are context-dependent (see below)
    /// D: Decimal
    pub const D: u8 = 0b0000_1000; // Decimal
    /// I: IRQ Disable
    pub const I: u8 = 0b0000_0100; // IRQ disable
    /// Z: Zero
    pub const Z: u8 = 0b0000_0010; // Zero
    /// C: Carry
    pub const C: u8 = 0b0000_0001; // Carry

    /// B: Break: set when pushing flags in interrupts
    pub const B_6502: u8 = 0b0001_0000;
    /// U: Unused, but always set in pushes
    pub const U_6502: u8 = 0b0010_0000; // Unused/always set in pushes on some parts

    /// M: Accumulator/Memory width
    pub const M_816: u8 = 0b0010_0000;
    /// X: Index width
    pub const X_816: u8 = 0b0001_0000;
}
