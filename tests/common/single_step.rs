use rs65x::{
    cpu6502::{Cpu6502, flavor::Flavor},
    isa::table::Instruction,
};

use super::{
    data::TestData,
    init_logger,
    setup::{Access, Harness, State},
};

pub fn run_suite<F: Flavor>(suite: &str, opcodes: impl IntoIterator<Item = u8>) {
    init_logger(log::LevelFilter::Warn);
    let mut data = TestData::open(suite).unwrap_or_else(|err| panic!("{err}"));
    for op in opcodes {
        print_opcode::<F>(op);
        run_opcode::<F>(&mut data, op, false).unwrap_or_else(|err| panic!("{err}"));
    }
}

pub fn run_single<F: Flavor>(suite: &str, op: u8) {
    init_logger(log::LevelFilter::Warn);
    let mut data = TestData::open(suite).unwrap_or_else(|err| panic!("{err}"));
    print_opcode::<F>(op);
    run_opcode::<F>(&mut data, op, true).unwrap_or_else(|err| panic!("{err}"));
}

fn print_opcode<F: Flavor>(op: u8) {
    let inst = Instruction::from_byte(op, F::OPCODE_TABLE);
    println!(
        "Starting {} [{op:02X}] {}, {}",
        F::NAME,
        inst.mnemonic,
        inst.address_mode
    );
}

fn run_opcode<F: Flavor>(data: &mut TestData, op: u8, debug: bool) -> Result<(), String> {
    let tests = data.load(op)?;
    for test in &tests {
        if debug {
            println!("TEST CASE BEGIN: \"{}\"", test.name);
        }
        let (mut cpu, mut bus) = setup_initial::<F>(&test.initial);
        bus.debug_print = debug;
        for cycle in &test.cycles {
            cpu.step(&mut bus);
            assert_cycle(cycle, &bus.last_cycle());
        }
        if debug {
            println!("All bus activity correct! Checking for final state");
        }
        assert_final(&cpu, &bus, &test.r#final);
    }
    Ok(())
}

pub fn setup_initial<F: Flavor>(initial: &State) -> (Cpu6502<F, Harness>, Harness) {
    let mut cpu = Cpu6502::<F, Harness>::new();
    cpu.pc = initial.pc;
    cpu.a = initial.a;
    cpu.s = initial.s;
    cpu.x = initial.x;
    cpu.y = initial.y;
    cpu.p = initial.p;
    let mut bus = Harness::default();
    for byte in &initial.ram {
        bus.mem[byte.0 as usize] = byte.1;
    }
    (cpu, bus)
}

pub fn assert_cycle(expected: &Access, actual: &Access) {
    if expected == actual {
        return;
    }
    println!();
    println!("Expected: {expected:?}");
    println!("Actual:   {actual:?}");
    // we failed an assert here
    assert!(
        expected.access_type == actual.access_type,
        "Expected: {:?}, Actual: {:?}",
        expected.access_type,
        actual.access_type
    );
    assert!(
        expected.addr == actual.addr,
        "Expected addr: {:04X}, Actual addr: {:04X}",
        expected.addr,
        actual.addr
    );
    assert!(
        expected.data == actual.data,
        "Expected data: {:02X}, Actual data: {:02X}",
        expected.data,
        actual.data
    );
}

pub fn assert_final<F: Flavor>(cpu: &Cpu6502<F, Harness>, bus: &Harness, fin: &State) {
    assert!(
        cpu.pc == fin.pc,
        "PC mismatch: Expected {:04X}, Actual {:04X}",
        fin.pc,
        cpu.pc
    );
    assert!(
        cpu.a == fin.a,
        "Register A mismatch: Expected {:02X}, Actual {:02X}",
        fin.a,
        cpu.a
    );
    assert!(
        cpu.s == fin.s,
        "Register S mismatch: Expected {:02X}, Actual {:02X}",
        fin.s,
        cpu.s
    );
    assert!(
        cpu.x == fin.x,
        "Register X mismatch: Expected {:02X}, Actual {:02X}",
        fin.x,
        cpu.x
    );
    assert!(
        cpu.y == fin.y,
        "Register Y mismatch: Expected {:02X}, Actual {:02X}",
        fin.y,
        cpu.y
    );
    assert!(
        cpu.p == fin.p,
        "Register P mismatch: Expected {:02X}{}, Actual {:02X}{}",
        fin.p,
        status(fin.p),
        cpu.p,
        status(cpu.p)
    );
    for byte in &fin.ram {
        assert!(
            bus.mem[byte.0 as usize] == byte.1,
            "Mem location {:#06X} should contain {:#06X} but contains {:#06X} instead",
            byte.0,
            byte.1,
            bus.mem[byte.0 as usize]
        )
    }
}

fn status(p: u8) -> String {
    const FLAGS: [(char, u8); 8] = [
        ('N', 7),
        ('V', 6),
        ('U', 5),
        ('B', 4),
        ('D', 3),
        ('I', 2),
        ('Z', 1),
        ('C', 0),
    ];

    let mut status = String::with_capacity(11);
    status.push('|');
    for (idx, &(flag, bit)) in FLAGS.iter().enumerate() {
        if idx == 4 {
            status.push(' ');
        }
        let active = (p >> bit) & 1 == 1;
        status.push(if active {
            flag
        } else {
            flag.to_ascii_lowercase()
        });
    }
    status.push('|');
    status
}
