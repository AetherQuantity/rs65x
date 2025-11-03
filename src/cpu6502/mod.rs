//! Minimal 6502 core scaffold (per-instruction stepping) wired to the shared Bus.
//!
//! This file intentionally starts tiny so we can iterate file-by-file under the
//! editor constraint. We'll later move opcode tables and addressing helpers into
//! `src/isa/` and expand to per-cycle micro-ops. For now: NOP and LDA #imm so we
//! can smoke-test the Bus and reset vector logic.

pub mod flavor;

use core::marker::PhantomData;

use crate::bus::Bus;
use crate::isa::memory::AddressMode;
use crate::isa::table::{Instruction, Mnemonic};
use crate::psr;

pub use flavor::Flavor; // re-export for convenience

/// CPU registers/state for a plain 6502-like core (8-bit A/X/Y, 16-bit PC).
pub struct Cpu6502<F: Flavor, B: Bus> {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub s: u8,   // stack pointer
    pub p: u8,   // processor status
    pub pc: u16, // program counter
    pub cycles: u64,
    _f: PhantomData<(F, B)>,
}

impl<F: Flavor, B: Bus> Cpu6502<F, B> {
    #[inline(always)]
    pub fn new() -> Self {
        Self {
            a: 0,
            x: 0,
            y: 0,
            s: 0xFD,                 // reset default
            p: psr::I | psr::U_6502, // I set, U set in pushes on many parts
            pc: 0,
            cycles: 0,
            _f: PhantomData,
        }
    }

    /// Read little-endian 16-bit vector from bank 0.
    /// TODO this is just a helper for something that's gonna get replaced with real cycle-accurate
    /// stuff later
    #[inline(always)]
    fn read16(bus: &mut B, addr: u16) -> u16 {
        let (lo, w0) = bus.read(addr as u32, /*vda=*/ true, /*vpa=*/ false);
        let (hi, w1) = bus.read(
            addr.wrapping_add(1) as u32,
            /*vda=*/ true,
            /*vpa=*/ false,
        );
        let _ = (w0, w1); // wait-states are accumulated by the caller as needed later
        u16::from_le_bytes([lo, hi])
    }

    /// Reset sequence: fetch PC from $FFFC/$FFFD. (IRQ/NMI vectors not handled here.)
    /// TODO this is not cycle accurate but whatever, i'll fix it later
    #[inline(always)]
    pub fn reset(&mut self, bus: &mut B) {
        self.p |= psr::I; // mask IRQ
        self.pc = Self::read16(bus, 0xFFFC); // TODO: implement the real cycle-accurate reset sequence
        self.cycles = 0;
    }
}
