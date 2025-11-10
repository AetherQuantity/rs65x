#![allow(dead_code)]
use crate::{
    bus::{Bus, Lines, WaitStates},
    cpu6502::{Cpu6502, Flavor},
    isa::op::StepResult,
};

/// Here's an 8-bit test harness that ignores all the vpa/vda stuff which is ideal as they are 16-bit only
struct Harness {
    mem: [u8; 0x10000],
    log: Vec<Access>,
    wait_map: std::collections::HashMap<u16, WaitStates>,
    lines: Lines,
    cycle: u64,
}

impl Default for Harness {
    fn default() -> Self {
        Self {
            mem: [0; 0x10000],
            log: Default::default(),
            wait_map: Default::default(),
            lines: Default::default(),
            cycle: Default::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum AccessType {
    Read,
    Write,
}

#[derive(Debug, Clone, PartialEq)]
struct Access {
    cycle: u64,
    addr: u16,
    data: u8,
    wait: WaitStates,
    access_type: AccessType,
}

impl Harness {
    fn with_program(reset_addr: u16, bytes: &[u8]) -> Self {
        let mut h = Harness::default();
        h.mem[reset_addr as usize..reset_addr as usize + bytes.len()].copy_from_slice(bytes);
        h.mem[0xFFFC] = (reset_addr & 0xFF) as u8;
        h.mem[0xFFFD] = (reset_addr >> 8) as u8;
        h
    }
    fn clear_log(&mut self) {
        self.log.clear();
        self.cycle = 0;
    }
}

impl Bus for Harness {
    fn read(&mut self, addr: u32, _vda: bool, _vpa: bool) -> (u8, WaitStates) {
        //println!("read access at {addr:X}");
        let access_type = AccessType::Read;
        let addr16 = addr as u16;
        let data = self.mem[addr16 as usize];
        let wait = *self.wait_map.get(&addr16).unwrap_or(&0);
        self.log.push(Access {
            cycle: self.cycle,
            addr: addr16,
            data,
            wait,
            access_type,
        });
        self.cycle += 1 + wait as u64;
        (data, wait)
    }

    fn write(&mut self, addr: u32, data: u8, _vda: bool, _vpa: bool) -> WaitStates {
        let access_type = AccessType::Write;
        let addr16 = addr as u16;
        self.mem[addr16 as usize] = data;
        let wait = *self.wait_map.get(&addr16).unwrap_or(&0);
        self.log.push(Access {
            cycle: self.cycle,
            addr: addr16,
            data,
            wait,
            access_type,
        });
        self.cycle += 1 + wait as u64;
        wait
    }

    fn sample_lines(&mut self) -> Lines {
        self.lines
    }
}

impl Access {
    fn cycle_cost(&self) -> u64 {
        1 + self.wait as u64
    }

    fn basic_read(addr: u16, data: u8) -> Self {
        Access {
            addr,
            cycle: 0, // overwrite this if needed
            data,
            wait: 0,
            access_type: AccessType::Read,
        }
    }

    fn basic_write(addr: u16, data: u8) -> Self {
        Access {
            addr,
            cycle: 0, // overwrite this if needed
            data,
            wait: 0,
            access_type: AccessType::Write,
        }
    }
}

struct InstructionTrace {
    cycles: u64,
    accesses: Vec<Access>,
    prefetch: Access,
}

impl InstructionTrace {
    fn assert_accesses(&self, expected: Vec<Access>) {
        let mut cycle = 0;
        let expected_cycles = expected.len() as u64;
        for mut e in expected {
            e.cycle = cycle;
            assert_eq!(
                e, self.accesses[cycle as usize],
                "Access: Cycle {cycle} mismatch!"
            );
            cycle += 1;
        }
        assert!(
            expected_cycles == self.cycles,
            "All expected cycles matched, but {} extra cycles found.",
            self.cycles - expected_cycles
        );
    }

    fn assert_prefetch(&self, expected: Access) {
        let cycle = self.accesses.len() as u64;
        let mut e = expected;
        e.cycle = cycle;
        assert_eq!(e, self.prefetch, "Prefetch mismatch!");
    }
}

fn run_instruction<F: Flavor>(
    cpu: &mut Cpu6502<F, Harness>,
    bus: &mut Harness,
) -> InstructionTrace {
    let start_cycles = cpu.cycles;
    let start_len = bus.log.len();
    loop {
        match cpu.step(bus) {
            StepResult::Pending => {}
            StepResult::InstructionFinished => break,
        }
    }
    let total_cycles = cpu.cycles - start_cycles;
    let mut entries = bus.log.split_off(start_len);
    let prefetch = entries
        .pop()
        .expect("instruction should end with opcode prefetch");
    let prefetch_cost = prefetch.cycle_cost();
    let cycles = total_cycles - prefetch_cost;
    debug_assert_eq!(
        cycles,
        entries.iter().map(Access::cycle_cost).sum::<u64>(),
        "body cycles should match log entries"
    );
    InstructionTrace {
        cycles,
        accesses: entries,
        prefetch,
    }
}

#[cfg(test)]
mod mem_cycle_accuracy {
    use super::*;
    use crate::cpu6502::Cpu6502;
    use crate::cpu6502::flavor::NMOS6502;
    use crate::psr;

    fn setup_6502(addr: u16, program: &[u8]) -> (Cpu6502<NMOS6502, Harness>, Harness) {
        let mut bus = Harness::with_program(addr, program); // LDA #$42; NOP
        let mut cpu = Cpu6502::<NMOS6502, Harness>::new();

        cpu.reset(&mut bus);
        bus.clear_log(); // ignore reset-vector reads
        assert_eq!(cpu.pc, 0x8000);
        (cpu, bus)
    }

    #[test]
    fn asl_implied() {
        // A/S/L?????
        let (mut cpu, mut bus) = setup_6502(0x8000, &[0x0A, 0xEA]); // ASL A; NOP
        cpu.a = 0b1001_1100; // some random bits to see if they rotate
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(trace.cycles, 2, "ASL IMM should be 2 cycles");
        assert_eq!(cpu.a, 0b0011_1000);
        assert_eq!(cpu.p & psr::C, psr::C); // C set because most sig bit rotated into it
        assert_eq!(cpu.p & (psr::Z | psr::N), 0); // result != 0, positive

        let accesses = vec![
            Access::basic_read(0x8000, 0x0A),
            Access::basic_read(0x8001, 0xEA), // discarded!
        ];
        let prefetch = Access::basic_read(0x8001, 0xEA); // re-read as opcode

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn ldy_immediate() {
        let (mut cpu, mut bus) = setup_6502(0x8000, &[0xA0, 0x42, 0xEA]); // LDY #$42; NOP
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(trace.cycles, 2, "LDY # should be 2 cycles");
        assert_eq!(cpu.y, 0x42);
        assert_eq!(cpu.p & (psr::Z | psr::N), 0); // result != 0, positive

        let accesses = vec![
            Access::basic_read(0x8000, 0xA0),
            Access::basic_read(0x8001, 0x42),
        ];
        let prefetch = Access::basic_read(0x8002, 0xEA);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn sty_zero_page() {
        let (mut cpu, mut bus) = setup_6502(0x8000, &[0x84, 0xAB, 0xEA]); // STY $AB; NOP
        cpu.y = 0x77;
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(trace.cycles, 3, "STY ZP should take 3 cycles");
        assert_eq!(bus.mem[0xAB], 0x77);
        assert_eq!(cpu.p & (psr::Z | psr::N), 0); // result != 0, positive

        let accesses = vec![
            Access::basic_read(0x8000, 0x84),
            Access::basic_read(0x8001, 0xAB),
            Access::basic_write(0x00AB, 0x77),
        ];
        let prefetch = Access::basic_read(0x8002, 0xEA);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn cmp_zp_x() {
        let (mut cpu, mut bus) = setup_6502(0x8000, &[0xD5, 0x62, 0xEA]); // CMP $62,X; NOP
        bus.mem[0x62] = 0x66; // not the byte we are addressing
        bus.mem[0x65] = 0x88; // this is the correct byte
        cpu.x = 0x03; // add 0x03 to 0x62 to get our effective address
        cpu.a = 0xF0; // will be cmp'd to 0x88
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(trace.cycles, 4, "CMP ZPX should take 4 cycles");
        assert_eq!(cpu.p & (psr::Z | psr::N), 0); // not zero, F0-88=68, not negative
        let expected_set = psr::C;
        assert_eq!(cpu.p & expected_set, expected_set); // carry (0xF0>0x88) -- sub's carry rules are backwards

        let accesses = vec![
            Access::basic_read(0x8000, 0xD5),
            Access::basic_read(0x8001, 0x62),
            Access::basic_read(0x0062, 0x66), // dummy read at pre-offset address
            Access::basic_read(0x0065, 0x88),
        ];
        let prefetch = Access::basic_read(0x8002, 0xEA);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn eor_indirect_x() {
        let (mut cpu, mut bus) = setup_6502(0x8000, &[0x41, 0x62, 0xEA]); // EOR ($62, X); NOP
        bus.mem[0x62] = 0xFF; // not the byte we are addressing
        bus.mem[0x65] = 0xEF; // low byte
        bus.mem[0x66] = 0xBE; // high byte
        bus.mem[0xBEEF] = 0x42; // final byte we want
        cpu.x = 0x03; // add 0x03 to 0x62 to get our effective address
        cpu.a = 0x42; // when EOR'd to 0x42, produces 0x00
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(trace.cycles, 6, "EOR IND,X should take 6 cycles");
        assert_eq!(cpu.a, 0);
        assert_eq!(cpu.p & psr::N, 0); // zero is not negative
        assert_eq!(cpu.p & psr::Z, psr::Z); // zero is, weirdly, zero

        let accesses = vec![
            Access::basic_read(0x8000, 0x41),
            Access::basic_read(0x8001, 0x62),
            Access::basic_read(0x0062, 0xFF), // dummy read at pre-offset address
            Access::basic_read(0x0065, 0xEF),
            Access::basic_read(0x0066, 0xBE),
            Access::basic_read(0xBEEF, 0x42),
        ];
        let prefetch = Access::basic_read(0x8002, 0xEA);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn ora_indirect_x_zp_wrap() {
        let (mut cpu, mut bus) = setup_6502(0x8000, &[0x01, 0x62, 0xEA]); // ORA ($62, X); NOP
        bus.mem[0x62] = 0xFF; // not the byte we are addressing
        bus.mem[0x60] = 0xEF; // low byte
        bus.mem[0x61] = 0xBE; // high byte
        bus.mem[0xBEEF] = 0x0F; // final byte we want
        cpu.x = 0xFE; // add 0xFE to 0x62, wrap around to 0x60
        cpu.a = 0x42; // when EOR'd to 0x42, produces 0x00
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(trace.cycles, 6, "ORA IND,X should take 6 cycles");
        assert_eq!(cpu.a, 0x4F);
        assert_eq!(cpu.p & (psr::N | psr::Z), 0);

        let accesses = vec![
            Access::basic_read(0x8000, 0x01),
            Access::basic_read(0x8001, 0x62),
            Access::basic_read(0x0062, 0xFF), // dummy read at pre-offset address
            Access::basic_read(0x0060, 0xEF),
            Access::basic_read(0x0061, 0xBE),
            Access::basic_read(0xBEEF, 0x0F),
        ];
        let prefetch = Access::basic_read(0x8002, 0xEA);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn lda_indirect_y_no_wrap() {
        let (mut cpu, mut bus) = setup_6502(0x8000, &[0xB1, 0x62, 0xEA]); // LDA ($62),Y; NOP
        bus.mem[0x62] = 0xEA; // low byte of address
        bus.mem[0x63] = 0xBE; // high byte of address
        bus.mem[0xBEEF] = 0x42; // final byte we want
        cpu.y = 0x05; // 0xBEEA + 0x05 = 0xBEEF
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(
            trace.cycles, 5,
            "LDA IND,Y should take 5 cycles in best case"
        );
        assert_eq!(cpu.a, 0x42);
        assert_eq!(cpu.p & (psr::Z | psr::N), 0); // 0x42 not negative, not zero

        let accesses = vec![
            Access::basic_read(0x8000, 0xB1),
            Access::basic_read(0x8001, 0x62),
            Access::basic_read(0x0062, 0xEA),
            Access::basic_read(0x0063, 0xBE),
            Access::basic_read(0xBEEF, 0x42),
        ];
        let prefetch = Access::basic_read(0x8002, 0xEA);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn lda_indirect_y_wrap() {
        let (mut cpu, mut bus) = setup_6502(0x8000, &[0xB1, 0x62, 0xEA]); // LDA ($62),Y; NOP
        bus.mem[0x62] = 0xF0; // low byte of address
        bus.mem[0x63] = 0xBD; // high byte of address
        bus.mem[0xBDEF] = 0xAA; // bad read at invalid address
        bus.mem[0xBEEF] = 0x42; // final byte we want
        cpu.y = 0xFF; // 0xBDF0 + 0xFF = 0xBEEF
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(
            trace.cycles, 6,
            "LDA IND,Y should take 6 cycles in worst case"
        );
        assert_eq!(cpu.a, 0x42);
        assert_eq!(cpu.p & (psr::Z | psr::N), 0); // 0x42 not negative, not zero

        let accesses = vec![
            Access::basic_read(0x8000, 0xB1),
            Access::basic_read(0x8001, 0x62),
            Access::basic_read(0x0062, 0xF0),
            Access::basic_read(0x0063, 0xBD),
            Access::basic_read(0xBDEF, 0xAA), // dummy read at invalid address
            Access::basic_read(0xBEEF, 0x42),
        ];
        let prefetch = Access::basic_read(0x8002, 0xEA);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn sta_indirect_y_no_wrap() {
        let (mut cpu, mut bus) = setup_6502(0x8000, &[0x91, 0x62, 0xEA]); // STA ($62),Y; NOP
        bus.mem[0x62] = 0xEA; // low byte of address
        bus.mem[0x63] = 0xBE; // high byte of address
        bus.mem[0xBEEF] = 0x42; // final byte we want
        cpu.y = 0x05; // 0xBEEA + 0x05 = 0xBEEF
        cpu.a = 0x69; // store this at 0xBEEF
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(
            trace.cycles, 6,
            "STA IND,Y should take 6 cycles, always, because write"
        );

        let accesses = vec![
            Access::basic_read(0x8000, 0x91),
            Access::basic_read(0x8001, 0x62),
            Access::basic_read(0x0062, 0xEA),
            Access::basic_read(0x0063, 0xBE),
            Access::basic_read(0xBEEF, 0x42), // read here because we are writing and this cycle must exist
            Access::basic_write(0xBEEF, 0x69),
        ];
        let prefetch = Access::basic_read(0x8002, 0xEA);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn and_absolute() {
        let (mut cpu, mut bus) = setup_6502(0x8000, &[0x2D, 0xEF, 0xBE, 0xEA]); // AND $BEEF; NOP
        bus.mem[0xBEEF] = 0b1100_0011; // bit pattern to be anded to accumulator
        cpu.a = 0b0101_0101;
        let expected = 0b0100_0001;
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(trace.cycles, 4, "AND ABS should take 4 cycles");
        assert_eq!(cpu.a, expected);
        assert_eq!(cpu.p & (psr::Z | psr::N), 0); // result != 0, positive

        let accesses = vec![
            Access::basic_read(0x8000, 0x2D),
            Access::basic_read(0x8001, 0xEF),
            Access::basic_read(0x8002, 0xBE),
            Access::basic_read(0xBEEF, 0xC3), // the bit pattern but in hex
        ];
        let prefetch = Access::basic_read(0x8003, 0xEA);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn ldx_absolute_y() {
        let (mut cpu, mut bus) = setup_6502(0x8000, &[0xBE, 0xEE, 0xBE, 0xEA]); // LDX $BEEF,Y; NOP
        bus.mem[0xBEEE] = 0x13; // not the correct address
        bus.mem[0xBEEF] = 0x26; // here's the correct address
        cpu.y = 0x01; // add to address
        cpu.x = 0x00; // will be replaced with 0x26
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(trace.cycles, 4, "LDX ABS,Y should take 4 cycles, best case");
        assert_eq!(cpu.x, 0x26);
        assert_eq!(cpu.p & (psr::Z | psr::N), 0); // result != 0, positive

        let accesses = vec![
            Access::basic_read(0x8000, 0xBE),
            Access::basic_read(0x8001, 0xEE),
            Access::basic_read(0x8002, 0xBE),
            Access::basic_read(0xBEEF, 0x26),
        ];
        let prefetch = Access::basic_read(0x8003, 0xEA);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn pla() {
        let (mut cpu, mut bus) = setup_6502(0x8000, &[0x68, 0xEA]); // PHA; NOP
        cpu.s = 0xFE;
        bus.mem[0x01FE] = 0x65;
        bus.mem[0x01FF] = 0x78;
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(trace.cycles, 4, "PLA should take 4 cycles");
        assert_eq!(cpu.s, 0xFF);
        assert_eq!(cpu.a, 0x78);

        let accesses = vec![
            Access::basic_read(0x8000, 0x68),
            Access::basic_read(0x8001, 0xEA),
            Access::basic_read(0x01FE, 0x65),
            Access::basic_read(0x01FF, 0x78),
        ];
        let prefetch = Access::basic_read(0x8001, 0xEA);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn pha() {
        let (mut cpu, mut bus) = setup_6502(0x8000, &[0x48, 0xEA]); // PHA; NOP
        cpu.s = 0xFF;
        cpu.a = 0x78;
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(trace.cycles, 3, "PHA should take 3 cycles");
        assert_eq!(cpu.s, 0xFE);
        assert_eq!(bus.mem[0x01FF], 0x78);

        let accesses = vec![
            Access::basic_read(0x8000, 0x48),
            Access::basic_read(0x8001, 0xEA),
            Access::basic_write(0x01FF, 0x78),
        ];
        let prefetch = Access::basic_read(0x8001, 0xEA);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }
}
