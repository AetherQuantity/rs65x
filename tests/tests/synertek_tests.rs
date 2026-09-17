use rs65x::cpu6502::flavor::Synertek65C02;

use crate::common::single_step;

#[test]
fn run_all() {
    single_step::run_suite::<Synertek65C02>("synertek65c02", 0x00..=0xFF);
}

#[test]
fn run_single() {
    single_step::run_single::<Synertek65C02>("synertek65c02", 0xEB);
}
