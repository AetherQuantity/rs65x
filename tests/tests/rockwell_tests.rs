use rs65x::cpu6502::flavor::Rockwell65C02;

use crate::common::single_step;

#[test]
fn run_all() {
    single_step::run_suite::<Rockwell65C02>("rockwell65c02", 0x00..=0xFF);
}

#[test]
fn run_single() {
    single_step::run_single::<Rockwell65C02>("rockwell65c02", 0xEB);
}
