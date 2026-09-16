//! Value-based arithmetic shared by CPU cores. Operand types select the arithmetic width;
//! register writeback, mode selection, and bus timing belong to the caller.

use crate::psr;

/// Decimal arithmetic behavior, independent of operand width and bus timing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecimalSemantics {
    /// Decimal mode is ignored (Ricoh 2A03/2A07).
    None,
    /// Original NMOS decimal adjustment and flag behavior.
    Nmos6502,
    /// 65C02 decimal adjustment and flag behavior.
    Cmos65C02,
    /// 65816 decimal subtraction uses digit-wise borrows and intermediate overflow.
    Wdc65C816,
}

/// A result from the ALU and appropriate flags to set. `apply()` applies the flags and
/// returns the value.
///  
/// Only flags selected by `flag_mask` are replaced when the result is applied.
#[must_use]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AluResult<T> {
    pub value: T,
    pub flags: u8,
    pub flag_mask: u8,
}

impl<T> AluResult<T> {
    pub(crate) fn apply(self, p: &mut u8) -> T {
        *p = (*p & !self.flag_mask) | (self.flags & self.flag_mask); // set requested flags
        self.value // return the actual value
    }
}

pub(crate) fn set_flag(p: &mut u8, mask: u8, value: bool) {
    if value { *p |= mask } else { *p &= !mask }
}

/// The supported operand widths. Intermediates use `u32` to retain carry;
/// subtraction uses wrapping arithmetic or `i32` when a signed borrow is needed.
/// CPU mode bits select the operand type before calling the ALU.
pub(crate) trait AluWord: Copy {
    const MASK: u32;
    const SIGN: u32;

    fn widen(self) -> u32;
    /// Keep only the low bits belonging to this operand width.
    fn truncate(value: u32) -> Self;
}

impl AluWord for u8 {
    const MASK: u32 = 0xff;
    const SIGN: u32 = 0x80;

    fn widen(self) -> u32 {
        u32::from(self)
    }
    fn truncate(value: u32) -> Self {
        value as Self
    }
}

impl AluWord for u16 {
    const MASK: u32 = 0xffff;
    const SIGN: u32 = 0x8000;

    fn widen(self) -> u32 {
        u32::from(self)
    }
    fn truncate(value: u32) -> Self {
        value as Self
    }
}

pub(crate) fn zn<T: AluWord>(value: T) -> AluResult<T> {
    AluResult {
        value,
        flags: if value.widen() & T::SIGN != 0 {
            psr::N
        } else {
            0
        } | if value.widen() == 0 { psr::Z } else { 0 },
        flag_mask: psr::Z | psr::N,
    }
}

fn with_shift_flags<T: AluWord>(value: T, carry: bool) -> AluResult<T> {
    let mut result = zn(value);
    result.flag_mask |= psr::C;
    set_flag(&mut result.flags, psr::C, carry);
    result
}

pub(crate) fn asl<T: AluWord>(value: T) -> AluResult<T> {
    with_shift_flags(
        T::truncate(value.widen() << 1),
        value.widen() & T::SIGN != 0,
    )
}

pub(crate) fn lsr<T: AluWord>(value: T) -> AluResult<T> {
    with_shift_flags(T::truncate(value.widen() >> 1), value.widen() & 1 != 0)
}

pub(crate) fn rol<T: AluWord>(value: T, carry: bool) -> AluResult<T> {
    with_shift_flags(
        T::truncate((value.widen() << 1) | u32::from(carry)),
        value.widen() & T::SIGN != 0,
    )
}

pub(crate) fn ror<T: AluWord>(value: T, carry: bool) -> AluResult<T> {
    with_shift_flags(
        T::truncate((value.widen() >> 1) | if carry { T::SIGN } else { 0 }),
        value.widen() & 1 != 0,
    )
}

pub(crate) fn inc<T: AluWord>(value: T) -> AluResult<T> {
    zn(T::truncate(value.widen() + 1))
}

pub(crate) fn dec<T: AluWord>(value: T) -> AluResult<T> {
    zn(T::truncate(value.widen().wrapping_sub(1)))
}

pub(crate) fn tsb<T: AluWord>(a: T, value: T) -> AluResult<T> {
    AluResult {
        value: T::truncate(value.widen() | a.widen()),
        flags: if a.widen() & value.widen() == 0 {
            psr::Z
        } else {
            0
        },
        flag_mask: psr::Z,
    }
}

pub(crate) fn trb<T: AluWord>(a: T, value: T) -> AluResult<T> {
    AluResult {
        value: T::truncate(value.widen() & !a.widen()),
        flags: if a.widen() & value.widen() == 0 {
            psr::Z
        } else {
            0
        },
        flag_mask: psr::Z,
    }
}

fn with_arith_flags<T: AluWord>(value: T, carry: bool, overflow: bool) -> AluResult<T> {
    let mut result = with_shift_flags(value, carry);
    result.flag_mask |= psr::V;
    set_flag(&mut result.flags, psr::V, overflow);
    result
}

pub(crate) fn adc<T: AluWord>(
    acc: T,
    operand: T,
    p: u8,
    semantics: DecimalSemantics,
) -> AluResult<T> {
    let a = acc.widen();
    let b = operand.widen();
    let mut carry = u32::from(p & psr::C != 0);
    let sum = a + b + carry;
    let overflow = |value| (!(a ^ b) & (a ^ value) & T::SIGN) != 0;
    if p & psr::D == 0 || semantics == DecimalSemantics::None {
        // Either can't or don't want decimal ADC, return binary result
        return with_arith_flags(T::truncate(sum), sum > T::MASK, overflow(sum));
    }

    let mut value = 0;
    let mut pre_high = 0;
    let nibbles = if T::MASK == 0xFF { 2 } else { 4 };
    for digit in 0..nibbles {
        let shift = digit * 4;
        let mut nibble = ((a >> shift) & 0xf) + ((b >> shift) & 0xf) + carry;
        // Overflow uses the top digit before its decimal correction.
        if digit == nibbles - 1 {
            pre_high = value | (nibble << shift);
        }
        carry = u32::from(nibble > 9);
        if carry != 0 {
            nibble += 6;
        }
        value |= (nibble & 0xf) << shift;
    }
    let mut result = with_arith_flags(T::truncate(value), carry != 0, overflow(pre_high));
    if semantics == DecimalSemantics::Nmos6502 {
        // NMOS Z comes from the binary sum; N precedes the top-digit correction.
        set_flag(&mut result.flags, psr::Z, sum & T::MASK == 0);
        set_flag(&mut result.flags, psr::N, pre_high & T::SIGN != 0);
    }
    result
}

pub(crate) fn sbc<T: AluWord>(
    acc: T,
    operand: T,
    p: u8,
    semantics: DecimalSemantics,
) -> AluResult<T> {
    let a = acc.widen();
    let b = operand.widen();
    let borrow_in = i32::from(p & psr::C == 0);
    let sum = a + (b ^ T::MASK) + u32::from(p & psr::C != 0);
    let binary = sum & T::MASK;
    let carry = sum > T::MASK;
    let overflow = |value| ((a ^ b) & (a ^ value) & T::SIGN) != 0;
    if p & psr::D == 0 || semantics == DecimalSemantics::None {
        // Either can't or don't want decimal SBC, return binary result
        return with_arith_flags(T::truncate(binary), carry, overflow(binary));
    }
    let nibbles = if T::MASK == 0xFF { 2 } else { 4 };
    if semantics == DecimalSemantics::Cmos65C02 {
        // Preserve the 65C02's binary-borrow correction, including invalid BCD
        // Each prefix determines whether its most significant digit borrows
        let mut value = binary;
        for digit in 0..nibbles {
            let shift = digit * 4;
            let prefix_mask = (1 << (shift + 4)) - 1;
            if (a & prefix_mask) as i32 - (b & prefix_mask) as i32 - borrow_in < 0 {
                value = value.wrapping_sub(6 << shift);
            }
        }
        return with_arith_flags(T::truncate(value), carry, overflow(binary));
    }

    // NMOS and 816 propagate a borrow between corrected digits. Keeping each
    // corrected digit to four bits also preserves behavior on non-BCD operands
    let mut value = 0;
    let mut borrow = borrow_in;
    let mut pre_high = 0;
    for digit in 0..nibbles {
        let shift = digit * 4;
        let mut nibble = ((a >> shift) & 0xf) as i32 - ((b >> shift) & 0xf) as i32 - borrow;
        if digit == nibbles - 1 {
            pre_high = value | ((nibble as u32) << shift);
        }
        borrow = i32::from(nibble < 0);
        if borrow != 0 {
            nibble -= 6;
        }
        value |= (nibble as u32 & 0xf) << shift;
    }
    let mut result = with_arith_flags(T::truncate(value), borrow == 0, overflow(pre_high));
    if semantics == DecimalSemantics::Nmos6502 {
        // NMOS keeps all arithmetic flags from the binary subtraction
        result.flags = with_arith_flags(T::truncate(binary), carry, overflow(binary)).flags;
    }
    result
}
