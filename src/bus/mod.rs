pub mod fastmap;

/// Level-sensitive input lines the CPU samples once per cycle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lines {
    /// Interrupt Request (maskable on 65x via I flag)
    pub irq: bool,
    /// Non-Maskable Interrupt
    pub nmi: bool,
    /// ABORT (65C816 native). Ignored by 6502/65C02 cores.
    pub abort_: bool,
    /// Ready: when low, the CPU inserts wait states (stretches the current cycle).
    pub rdy: bool,
    /// Bus Enable (65C816). When low, the CPU tri-states its buses. Mostly for co-processors.
    pub be: bool,
}

impl Lines {
    /// Convenience constructor: all lines low (inactive).
    #[inline(always)]
    pub const fn none() -> Self {
        Self {
            irq: false,
            nmi: false,
            abort_: false,
            rdy: true,
            be: true,
        }
    }
}

/// Extra wait states returned by the bus for a given access.
///
/// A value of 0 means the access completes within the current cycle budget.
/// Non-zero values stretch the cycle by that many additional master cycles.
pub type WaitStates = u8;

/// Shared bus trait for all cores.
///
/// The core performs **one call per bus cycle** in the default configuration.
/// `vda`/`vpa` mirror the 65C816 pins and are meaningful for hosts even when
/// running 8-bit cores (6502/65C02) — those cores simply set them as:
/// - instruction fetch: `vpa = true, vda = false`
/// - data access:       `vda = true,  vpa = false`
/// - internal/no access: both false (rare)
pub trait Bus {
    /// Read a byte from the 24-bit address space.
    ///
    /// Returns `(data, extra_wait_states)`.
    fn read(&mut self, addr: u32, vda: bool, vpa: bool) -> (u8, WaitStates);

    /// Write a byte to the 24-bit address space.
    ///
    /// Returns `extra_wait_states`.
    fn write(&mut self, addr: u32, data: u8, vda: bool, vpa: bool) -> WaitStates;

    /// Sample asynchronous level-sensitive lines once per cycle.
    fn sample_lines(&mut self) -> Lines;
}

/// Simple open-bus tracker for hosts that want deterministic reads when the
/// bus is not driven (optional usage by Bus implementors).
#[derive(Clone, Copy, Debug, Default)]
pub struct OpenBus {
    last: u8,
}
impl OpenBus {
    #[inline(always)]
    pub fn sample(&self) -> u8 {
        self.last
    }
    #[inline(always)]
    pub fn drive(&mut self, v: u8) {
        self.last = v;
    }
}
