use rs65x::cpu6502::flavor::NES;

use crate::common::single_step;

#[test]
fn run_all() {
    single_step::run_suite::<NES>("nes6502", 0x00..=0xFF);
}

#[test]
fn run_single() {
    single_step::run_single::<NES>("nes6502", 0x00);
}
