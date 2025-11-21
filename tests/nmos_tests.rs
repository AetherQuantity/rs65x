use rs65x::{
    cpu6502::{Cpu6502, flavor::NMOS6502},
    isa::table::{Instruction, OpcodeTable},
};
use serde::Deserialize;
use serde_json;
use std::fs;

use crate::setup::{Access, Harness, SingleStepCase, State};

mod setup;

#[test]
fn run_all() {
    for op in 0x00..=0xFF {
        run_opcode(op).unwrap_or_default();
    }
}

fn run_opcode(op: u8) -> Result<(), String> {
    let path = format!("tests/nmos/{op:02X}.json");
    let json = fs::read_to_string(&path).map_err(|_| "opcode not found")?;
    // The top-level JSON is an array of test cases
    let tests: Vec<SingleStepCase> =
        serde_json::from_str(&json).map_err(|_| "error reading json")?;
    assert!(!tests.is_empty(), "no tests found in JSON");
    let inst = Instruction::from_byte(op, OpcodeTable::Nmos);

    println!("Starting {}, {}", inst.mnemonic, inst.address_mode);
    for test in &tests {
        let (mut cpu, mut bus) = setup_initial(&test.initial);
        for cycle in &test.cycles {
            cpu.step(&mut bus);
            assert_eq!(cycle, &bus.last_cycle());
        }
        // we kinda have to run an extra cycle for opcode prefetch
        // because that's where the previous cycle gets finalized
        cpu.step(&mut bus);
        cpu.pc -= 1; // but pc is expected to be the next opcode, not after
        assert_final(&cpu, &bus, &test.r#final);
    }
    Ok(())
}

pub fn setup_initial(initial: &State) -> (Cpu6502<NMOS6502, Harness>, Harness) {
    let mut cpu = Cpu6502::<NMOS6502, Harness>::new();
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

pub fn assert_final(cpu: &Cpu6502<NMOS6502, Harness>, bus: &Harness, fin: &State) {
    assert_eq!(cpu.pc, fin.pc, "PC mismatch");
    assert_eq!(cpu.a, fin.a, "Register A mismatch");
    assert_eq!(cpu.s, fin.s, "Register S mismatch");
    assert_eq!(cpu.x, fin.x, "Register X mismatch");
    assert_eq!(cpu.y, fin.y, "Register Y mismatch");
    assert_eq!(cpu.p, fin.p, "Register P mismatch");
    for byte in &fin.ram {
        assert_eq!(
            bus.mem[byte.0 as usize], byte.1,
            "Mem location {:#06X} should contain {:#06X} but contains {:#06X} instead",
            byte.0, byte.1, bus.mem[byte.0 as usize]
        )
    }
}
