//! Ergonomic public surface for the 6502-family cores.
//!
//! The crate re-exports ready-to-use CPU variants and a simple in-memory bus so
//! you don't have to wire up flavors manually:
//!
//! ```
//! use rs65x::{Nmos6502, SimpleBus};
//!
//! // Zeroed RAM with a tiny program loaded at $8000.
//! let mut bus = SimpleBus::with_program(0x8000, &[0xEA, 0xEA]); // NOP; NOP
//! bus.set_reset_vector(0x8000);
//!
//! // Pick a CPU variant without touching internal flavor types.
//! let mut cpu: Nmos6502<SimpleBus> = Nmos6502::new();
//! cpu.reset(&mut bus);
//! ```

mod alu;
pub mod bus;
pub mod cpu6502;
pub mod isa;

pub use bus::fastmap;
pub use bus::{Bus, FastMapBus, Lines, SimpleBus};

/// NMOS 6502 core (original MOS/Rockwell parts).
pub type Nmos6502<B> = cpu6502::Cpu6502<cpu6502::flavor::NMOS6502, B>;
/// NES Ricoh 2A03/2A07 variant (decimal mode disabled).
pub type Nes6502<B> = cpu6502::Cpu6502<cpu6502::flavor::NES, B>;
/// CMOS 65C02 core with Rockwell extensions enabled.
pub type Cmos65c02<B> = cpu6502::Cpu6502<cpu6502::flavor::CMOS65C02, B>;

/// Grab the most common types without digging through modules.
pub mod prelude {
    pub use crate::{
        Cmos65c02, Nes6502, Nmos6502, SimpleBus,
        bus::{Bus, Lines},
    };
}

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
