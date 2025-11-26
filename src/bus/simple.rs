//! Minimal 64 KiB in-memory bus for 6502-family cores.
//!
//! This is intended for quick bring-up, tests, and examples. It wraps a single
//! 64 KiB RAM array, mirrors writes back for reads, and exposes helpers to load
//! programs and set the interrupt vectors.

use super::{Bus, Lines};

/// Simple zero-wait-state 64 KiB bus backed by RAM.
pub struct SimpleBus {
    pub ram: [u8; 0x10000],
    lines: Lines,
}

impl SimpleBus {
    /// Create a zeroed bus with all lines inactive.
    #[inline]
    pub fn new() -> Self {
        Self {
            ram: [0; 0x10000],
            lines: Lines::none(),
        }
    }

    /// Load bytes into RAM starting at `origin`, wrapping within the 16-bit space.
    #[inline]
    pub fn load(&mut self, origin: u16, bytes: &[u8]) {
        for (i, b) in bytes.iter().enumerate() {
            let addr = origin.wrapping_add(i as u16) as usize;
            self.ram[addr] = *b;
        }
    }

    /// Create a bus with `bytes` preloaded at `origin`.
    #[inline]
    pub fn with_program(origin: u16, bytes: &[u8]) -> Self {
        let mut bus = Self::new();
        bus.load(origin, bytes);
        bus
    }

    /// Set the reset vector ($FFFC/$FFFD) to `addr`.
    #[inline]
    pub fn set_reset_vector(&mut self, addr: u16) {
        self.write_vector(0xFFFC, addr);
    }

    /// Set the IRQ/BRK vector ($FFFE/$FFFF) to `addr`.
    #[inline]
    pub fn set_irq_vector(&mut self, addr: u16) {
        self.write_vector(0xFFFE, addr);
    }

    /// Set the NMI vector ($FFFA/$FFFB) to `addr`.
    #[inline]
    pub fn set_nmi_vector(&mut self, addr: u16) {
        self.write_vector(0xFFFA, addr);
    }

    /// Access the mutable lines configuration.
    #[inline]
    pub fn lines_mut(&mut self) -> &mut Lines {
        &mut self.lines
    }

    #[inline]
    fn write_vector(&mut self, base: u16, addr: u16) {
        let [lo, hi] = addr.to_le_bytes();
        self.ram[base as usize] = lo;
        self.ram[base as usize + 1] = hi;
    }
}

impl Default for SimpleBus {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus for SimpleBus {
    #[inline(always)]
    fn read(&mut self, addr: u32, _vda: bool, _vpa: bool) -> u8 {
        let addr16 = addr as u16 as usize;
        self.ram[addr16]
    }

    #[inline(always)]
    fn write(&mut self, addr: u32, data: u8, _vda: bool, _vpa: bool) {
        let addr16 = addr as u16 as usize;
        self.ram[addr16] = data;
    }

    #[inline(always)]
    fn sample_lines(&mut self) -> Lines {
        self.lines
    }
}
