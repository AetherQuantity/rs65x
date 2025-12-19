use std::{fs::File, io::Read, time::Instant};

use rs65x::{
    FastMapBus,
    cpu6502::{
        Cpu6502,
        flavor::{CMOS65C02, NMOS6502},
    },
};

#[test]
fn functional_tests() {
    let mut bus = FastMapBus::new();
    let mut cpu: Cpu6502<NMOS6502, FastMapBus<16, 12>> = Cpu6502::new();

    let mut f = File::open("tests/func_tests/6502_functional_test.bin").unwrap();
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).unwrap();
    let mut page = 0;
    for page_data in buf.as_chunks_mut::<0x1000>().0 {
        bus.map_ram_page_idx(page, page_data);
        page += 1;
    }

    const PROGRAM_START: u16 = 0x400;
    const SUCCESS_ADDR: u16 = 0x336D;

    cpu.reset(&mut bus);
    cpu.pc = PROGRAM_START;
    let mut cycles = 0;
    let start = Instant::now();
    while cpu.pc != SUCCESS_ADDR {
        cpu.step(&mut bus);
        cycles += 1;
    }
    let second = (Instant::now() - start).as_secs_f64();
    let mhz = cycles as f64 / second / 1_000_000.;
    println!("Speed: {mhz:.4}MHz")
}

#[test]
fn extended_opcode_tests() {
    let mut bus = FastMapBus::new();
    let mut cpu: Cpu6502<CMOS65C02, FastMapBus<16, 12>> = Cpu6502::new();

    let mut f = File::open("tests/func_tests/65C02_extended_opcodes_test.bin").unwrap();
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).unwrap();
    let mut page = 0;
    for page_data in buf.as_chunks_mut::<0x1000>().0 {
        bus.map_ram_page_idx(page, page_data);
        page += 1;
    }

    const PROGRAM_START: u16 = 0x400;
    const SUCCESS_ADDR: u16 = 0x24F1;

    cpu.reset(&mut bus);
    cpu.pc = PROGRAM_START;
    let mut cycles: usize = 0;
    let start = Instant::now();
    while cpu.pc != SUCCESS_ADDR {
        cpu.step(&mut bus);
        cycles += 1;
    }
    let second = (Instant::now() - start).as_secs_f64();
    let mhz = cycles as f64 / second / 1_000_000.;
    println!("Speed: {mhz:.4}MHz")
}
