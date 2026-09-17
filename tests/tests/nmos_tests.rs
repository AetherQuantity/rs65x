use rs65x::cpu6502::flavor::NMOS6502;

use crate::common::single_step;

#[test]
fn run_all() {
    single_step::run_suite::<NMOS6502>("6502", 0x00..=0xFF);
}

#[test]
fn run_single() {
    single_step::run_single::<NMOS6502>("6502", 0xE1);
}
