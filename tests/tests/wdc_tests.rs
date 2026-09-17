use rs65x::cpu6502::flavor::WDC65C02;

use crate::common::single_step;

#[test]
fn run_all() {
    // Upstream omits WAI/STP. Their execution and dedicated tests remain TODO.
    single_step::run_suite::<WDC65C02>(
        "wdc65c02",
        (0x00..=0xFF).filter(|op| !matches!(op, 0xCB | 0xDB)),
    );
}

#[test]
fn run_single() {
    single_step::run_single::<WDC65C02>("wdc65c02", 0xEB);
}
