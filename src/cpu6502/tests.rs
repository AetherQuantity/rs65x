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
        let access_type = AccessType::Read;
        let addr16 = addr as u16;
        let data = self.mem[addr16 as usize];
        println!("read access at {addr:#04X} | data = {data:#04X}");
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
        println!("write access at {addr:#04X} | data = {data:#04X}");
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
    use crate::cpu6502::flavor::{CMOS65C02, NMOS6502};
    use crate::psr;

    fn setup_nmos(addr: u16, program: &[u8]) -> (Cpu6502<NMOS6502, Harness>, Harness) {
        let mut bus = Harness::with_program(addr, program);
        let mut cpu = Cpu6502::<NMOS6502, Harness>::new();

        cpu.reset(&mut bus);
        bus.clear_log(); // ignore reset-vector reads
        assert_eq!(cpu.pc, addr);
        (cpu, bus)
    }

    fn setup_cmos(addr: u16, program: &[u8]) -> (Cpu6502<CMOS65C02, Harness>, Harness) {
        let mut bus = Harness::with_program(addr, program);
        let mut cpu = Cpu6502::<CMOS65C02, Harness>::new();

        cpu.reset(&mut bus);
        bus.clear_log(); // ignore reset-vector reads
        assert_eq!(cpu.pc, addr);
        (cpu, bus)
    }

    #[test]
    fn imp_asl() {
        // A/S/L?????
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x0A, 0xEA]); // ASL A; NOP
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
    fn imm_ldy() {
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0xA0, 0x42, 0xEA]); // LDY #$42; NOP
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
    fn zp_sty() {
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x84, 0xAB, 0xEA]); // STY $AB; NOP
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
    fn zp_x_cmp() {
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0xD5, 0x62, 0xEA]); // CMP $62,X; NOP
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
    fn ind_x_eor() {
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x41, 0x62, 0xEA]); // EOR ($62, X); NOP
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
    fn ind_x_zp_wrap_ora() {
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x01, 0x62, 0xEA]); // ORA ($62, X); NOP
        bus.mem[0x62] = 0xFF; // not the byte we are addressing
        bus.mem[0x60] = 0xEF; // low byte
        bus.mem[0x61] = 0xBE; // high byte
        bus.mem[0xBEEF] = 0x0F; // final byte we want
        cpu.x = 0xFE; // add 0xFE to 0x62, wrap around to 0x60
        cpu.a = 0x42; // when OR'd to 0x0F, produces 0x4F
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
    fn ind_y_no_wrap_ldy() {
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0xB1, 0x62, 0xEA]); // LDA ($62),Y; NOP
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
    fn ind_y_wrap_lda() {
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0xB1, 0x62, 0xEA]); // LDA ($62),Y; NOP
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
    fn ind_y_no_wrap_sta() {
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x91, 0x62, 0xEA]); // STA ($62),Y; NOP
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
    fn abs_and() {
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x2D, 0xEF, 0xBE, 0xEA]); // AND $BEEF; NOP
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
    fn abs_y_ldx() {
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0xBE, 0xEE, 0xBE, 0xEA]); // LDX $BEEF,Y; NOP
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
    fn stack_pla() {
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x68, 0xEA]); // PHA; NOP
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
    fn stack_pha() {
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x48, 0xEA]); // PHA; NOP
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

    #[test]
    fn zp_rmw_inc() {
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0xE6, 0x08, 0xEA]); // INC $08; NOP
        bus.mem[0x08] = 0xA0;
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(trace.cycles, 5, "SMB0 (rmw) should take 5 cycles");
        assert_eq!(bus.mem[0x08], 0xA1);

        let accesses = vec![
            Access::basic_read(0x8000, 0xE6),
            Access::basic_read(0x8001, 0x08),
            Access::basic_read(0x0008, 0xA0),
            Access::basic_write(0x0008, 0xA0),
            Access::basic_write(0x0008, 0xA1),
        ];
        let prefetch = Access::basic_read(0x8002, 0xEA);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn smb4_rmw() {
        let (mut cpu, mut bus) = setup_cmos(0x8000, &[0xC7, 0x08, 0xEA]); // SMB4 $08; NOP
        bus.mem[0x08] = 0xA0;
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(trace.cycles, 5, "SMB0 (rmw) should take 5 cycles");
        assert_eq!(bus.mem[0x08], 0xB0);

        let accesses = vec![
            Access::basic_read(0x8000, 0xC7),
            Access::basic_read(0x8001, 0x08),
            Access::basic_read(0x0008, 0xA0),
            Access::basic_read(0x0008, 0xA0), // on a 6502 rmw this would be a write!!
            Access::basic_write(0x0008, 0xB0),
        ];
        let prefetch = Access::basic_read(0x8002, 0xEA);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn rel_take_beq() {
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0xF0, 0x10, 0xEA]); // BEQ #$10; NOP
        cpu.p |= psr::Z; // make sure Z is set
        bus.mem[0x8012] = 0xE8; // INX
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(
            trace.cycles, 3,
            "BEQ should take 3 cycles on branch taken same page"
        );
        // i mean, technically 4 according to 6502_cpu.txt, but the 4th is next opcode fetch which we classify
        // as prefetch
        assert_eq!(cpu.pc, 0x8013);

        let accesses = vec![
            Access::basic_read(0x8000, 0xF0),
            Access::basic_read(0x8001, 0x10),
            Access::basic_read(0x8002, 0xEA), // dummy read
        ];
        let prefetch = Access::basic_read(0x8012, 0xE8);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn rel_take_back_bmi() {
        let (mut cpu, mut bus) = setup_nmos(0x8060, &[0x30, 0xF0, 0xEA]); // BMI #$-10; NOP
        cpu.p |= psr::N; // make sure Z is set
        bus.mem[0x8052] = 0xE8; // INX
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(
            trace.cycles, 3,
            "BEQ should take 3 cycles on branch taken same page"
        );
        // i mean, technically 4 according to 6502_cpu.txt, but the 4th is next opcode fetch which we classify
        // as prefetch
        assert_eq!(cpu.pc, 0x8053);

        let accesses = vec![
            Access::basic_read(0x8060, 0x30),
            Access::basic_read(0x8061, 0xF0),
            Access::basic_read(0x8062, 0xEA), // dummy read
        ];
        let prefetch = Access::basic_read(0x8052, 0xE8);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn rel_take_cross_page_bvc() {
        let (mut cpu, mut bus) = setup_nmos(0x80F0, &[0x50, 0x10, 0xEA]); // BVC #$10; NOP
        cpu.p &= !psr::V; // make sure V is clear
        bus.mem[0x8002] = 0x78; // junk data at wrong address
        bus.mem[0x8102] = 0xE8; // INX
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(
            trace.cycles, 4,
            "BEQ should take 4 cycles on branch taken different page"
        );
        // i mean, technically 5 according to 6502_cpu.txt, but the 5th is next opcode fetch which we classify
        // as prefetch
        assert_eq!(cpu.pc, 0x8103);

        let accesses = vec![
            Access::basic_read(0x80F0, 0x50),
            Access::basic_read(0x80F1, 0x10),
            Access::basic_read(0x80F2, 0xEA),
            Access::basic_read(0x8002, 0x78),
        ];
        let prefetch = Access::basic_read(0x8102, 0xE8);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn rel_take_back_cross_page_bcc() {
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x90, 0xF0, 0xEA]); // BCC #$-10; NOP
        cpu.p &= !psr::C; // make sure C is clear
        bus.mem[0x80F2] = 0x78; // junk data at wrong address
        bus.mem[0x7FF2] = 0xE8; // INX
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(
            trace.cycles, 4,
            "BEQ should take 4 cycles on branch taken different page"
        );
        // i mean, technically 5 according to 6502_cpu.txt, but the 5th is next opcode fetch which we classify
        // as prefetch
        assert_eq!(cpu.pc, 0x7FF3);

        let accesses = vec![
            Access::basic_read(0x8000, 0x90),
            Access::basic_read(0x8001, 0xF0),
            Access::basic_read(0x8002, 0xEA), // dummy read
            Access::basic_read(0x80F2, 0x78), // bad read at invalid address
        ];
        let prefetch = Access::basic_read(0x7FF2, 0xE8);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn rel_no_take_bcs() {
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0xB0, 0xF0, 0xEA]); // BCC #$-10; NOP
        cpu.p &= !psr::C; // make sure C is clear
        bus.mem[0x80F2] = 0x78; // junk data at wrong address
        bus.mem[0x7FF2] = 0xE8; // INX
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(
            trace.cycles, 2,
            "BCS should take 2 cycles on branch not taken"
        );
        assert_eq!(cpu.pc, 0x8003);

        let accesses = vec![
            Access::basic_read(0x8000, 0xB0),
            Access::basic_read(0x8001, 0xF0),
        ];
        let prefetch = Access::basic_read(0x8002, 0xEA);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn zprel_take_bbs2() {
        let (mut cpu, mut bus) = setup_cmos(0x8000, &[0xAF, 0x42, 0x10, 0xEA]); // BBS2 #$10; NOP
        bus.mem[0x0042] = 0x3C; // bit 2 of this is set
        bus.mem[0x8013] = 0xE8; // INX
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(
            trace.cycles, 5,
            "BBS should take 5 cycles on branch taken same page"
        );
        assert_eq!(cpu.pc, 0x8014);

        let accesses = vec![
            Access::basic_read(0x8000, 0xAF),
            Access::basic_read(0x8001, 0x42),
            Access::basic_read(0x0042, 0x3C),
            Access::basic_read(0x8002, 0x10),
            Access::basic_read(0x8003, 0xEA), //dummy
        ];
        let prefetch = Access::basic_read(0x8013, 0xE8);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn jmpabs() {
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x4C, 0x69, 0x80]); // JMP $8069
        bus.mem[0x8069] = 0xEA; // NOP
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(trace.cycles, 3, "JMP ABS should take 3 cycles");
        assert_eq!(cpu.pc, 0x806A);

        let accesses = vec![
            Access::basic_read(0x8000, 0x4C),
            Access::basic_read(0x8001, 0x69),
            Access::basic_read(0x8002, 0x80),
        ];
        let prefetch = Access::basic_read(0x8069, 0xEA);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn jmpind_normal() {
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x6C, 0x69, 0x80]); // JMP ($8069)
        bus.mem[0x8069] = 0xEF;
        bus.mem[0x806A] = 0xBE;
        bus.mem[0xBEEF] = 0xEA;
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(trace.cycles, 5, "JMP IND should take 5 cycles on NMOS");
        assert_eq!(cpu.pc, 0xBEF0);

        let accesses = vec![
            Access::basic_read(0x8000, 0x6C),
            Access::basic_read(0x8001, 0x69),
            Access::basic_read(0x8002, 0x80),
            Access::basic_read(0x8069, 0xEF),
            Access::basic_read(0x806A, 0xBE),
        ];
        let prefetch = Access::basic_read(0xBEEF, 0xEA);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn jmpind_wrap_bug() {
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x6C, 0xFF, 0x80]); // JMP ($80FF)
        bus.mem[0x80FF] = 0xEF;
        bus.mem[0x8100] = 0xBE;
        bus.mem[0xBEEF] = 0xEA;
        bus.mem[0x6CEF] = 0x01; // whatever junk data
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(trace.cycles, 5, "JMP IND should take 5 cycles");
        assert_eq!(cpu.pc, 0x6CF0);

        let accesses = vec![
            Access::basic_read(0x8000, 0x6C),
            Access::basic_read(0x8001, 0xFF),
            Access::basic_read(0x8002, 0x80),
            Access::basic_read(0x80FF, 0xEF),
            Access::basic_read(0x8000, 0x6C), // uh oh, bad address! should have been 0x8100
        ];
        let prefetch = Access::basic_read(0x6CEF, 0x01);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn jmpind_wrap_fix() {
        let (mut cpu, mut bus) = setup_cmos(0x8000, &[0x6C, 0xFF, 0x80]); // JMP ($80FF)
        bus.mem[0x80FF] = 0xEF;
        bus.mem[0x8100] = 0xBE;
        bus.mem[0xBEEF] = 0xEA;
        bus.mem[0x6CEF] = 0x01; // whatever junk data
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(trace.cycles, 6, "JMP IND should take 6 cycles on CMOS");
        assert_eq!(cpu.pc, 0xBEF0);

        // the CMOS JMP IND dummy reads the high byte of operand an extra time, NOT the incorrect
        // address, as per https://github.com/CompuSAR/sar6502/blob/master/sar6502.srcs/sim_1/new/test_plan.mem

        let accesses = vec![
            Access::basic_read(0x8000, 0x6C),
            Access::basic_read(0x8001, 0xFF),
            Access::basic_read(0x8002, 0x80),
            Access::basic_read(0x8002, 0x80), // dummy read here for some reason
            Access::basic_read(0x80FF, 0xEF),
            Access::basic_read(0x8100, 0xBE), // fix, hooray!
        ];
        let prefetch = Access::basic_read(0xBEEF, 0xEA);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn jmpind_normal_cmos() {
        let (mut cpu, mut bus) = setup_cmos(0x8000, &[0x6C, 0x69, 0x80]); // JMP ($8069)
        bus.mem[0x8069] = 0xEF;
        bus.mem[0x806A] = 0xBE;
        bus.mem[0xBEEF] = 0xEA;
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(trace.cycles, 6, "JMP IND should take 6 cycles on CMOS");
        assert_eq!(cpu.pc, 0xBEF0);

        let accesses = vec![
            Access::basic_read(0x8000, 0x6C),
            Access::basic_read(0x8001, 0x69),
            Access::basic_read(0x8002, 0x80),
            Access::basic_read(0x8002, 0x80),
            Access::basic_read(0x8069, 0xEF),
            Access::basic_read(0x806A, 0xBE),
        ];
        let prefetch = Access::basic_read(0xBEEF, 0xEA);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn jmpindx() {
        let (mut cpu, mut bus) = setup_cmos(0x8000, &[0x7C, 0x69, 0x80]); // JMP ($8069)
        bus.mem[0x806E] = 0xEF;
        bus.mem[0x806F] = 0xBE;
        bus.mem[0xBEEF] = 0xEA;
        cpu.x = 0x5;
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(trace.cycles, 6, "JMP IND should take 6 cycles on CMOS");
        assert_eq!(cpu.pc, 0xBEF0);

        let accesses = vec![
            Access::basic_read(0x8000, 0x7C),
            Access::basic_read(0x8001, 0x69),
            Access::basic_read(0x8002, 0x80),
            Access::basic_read(0x8002, 0x80),
            Access::basic_read(0x806E, 0xEF),
            Access::basic_read(0x806F, 0xBE),
        ];
        let prefetch = Access::basic_read(0xBEEF, 0xEA);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn jsr() {
        // nmos and cmos are the same
        let (mut cpu, mut bus) = setup_cmos(0x8000, &[0x20, 0x69, 0x80]); // JSR $8069
        cpu.s = 0xFF;
        bus.mem[0x8069] = 0xEA;
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(trace.cycles, 6, "JSR is always 6 cycles");
        assert_eq!(cpu.pc, 0x806A);
        assert_eq!(cpu.s, 0xFD);
        assert_eq!(bus.mem[0x1FF], 0x80);
        assert_eq!(bus.mem[0x1FE], 0x02);
        let accesses = vec![
            Access::basic_read(0x8000, 0x20),
            Access::basic_read(0x8001, 0x69),
            Access::basic_read(0x01FF, 0x00),
            Access::basic_write(0x01FF, 0x80),
            Access::basic_write(0x01FE, 0x02),
            Access::basic_read(0x8002, 0x80),
        ];
        let prefetch = Access::basic_read(0x8069, 0xEA);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn rts() {
        let (mut cpu, mut bus) = setup_cmos(0x806A, &[0x60, 0x69]); // RTS; junk
        cpu.s = 0xFD;
        bus.mem[0x1FE] = 0x02;
        bus.mem[0x1FF] = 0x80; // address 0x8002 on the stack
        bus.mem[0x8002] = 0xAB; // junk
        bus.mem[0x8003] = 0xEA; // NOP
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(trace.cycles, 6, "RTS is always 6 cycles");
        assert_eq!(cpu.pc, 0x8004);
        assert_eq!(cpu.s, 0xFF);
        assert_eq!(bus.mem[0x1FF], 0x80);
        assert_eq!(bus.mem[0x1FE], 0x02);
        let accesses = vec![
            Access::basic_read(0x806A, 0x60),
            Access::basic_read(0x806B, 0x69),
            Access::basic_read(0x01FD, 0x00),
            Access::basic_read(0x01FE, 0x02),
            Access::basic_read(0x01FF, 0x80),
            Access::basic_read(0x8002, 0xAB),
        ];
        let prefetch = Access::basic_read(0x8003, 0xEA);
        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn brk() {
        let (mut cpu, mut bus) = setup_cmos(0x8000, &[0x00, 0x69]); // BRK; junk
        cpu.s = 0xFF;
        cpu.p = 0;
        bus.mem[0xFFFE] = 0x45;
        bus.mem[0xFFFF] = 0x23;
        bus.mem[0x2345] = 0xEA;
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(trace.cycles, 7, "BRK is always 7 cycles");
        assert_eq!(cpu.pc, 0x2346);
        let accesses = vec![
            Access::basic_read(0x8000, 0x00), // BRK
            Access::basic_read(0x8001, 0x69), // (pc is incremented here)
            Access::basic_write(0x01FF, 0x80),
            Access::basic_write(0x01FE, 0x02),
            Access::basic_write(0x01FD, psr::B_6502 | psr::U_6502),
            Access::basic_read(0xFFFE, 0x45),
            Access::basic_read(0xFFFF, 0x23),
        ];
        let prefetch = Access::basic_read(0x2345, 0xEA);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }

    #[test]
    fn rti() {
        let (mut cpu, mut bus) = setup_cmos(0x9000, &[0x40, 0x69]); // RTI; junk
        cpu.s = 0xFC;
        bus.mem[0x1FD] = psr::I;
        bus.mem[0x1FE] = 0x02;
        bus.mem[0x1FF] = 0x80; // address 0x8002 on the stack
        bus.mem[0x8002] = 0xEA; // NOP
        let trace = run_instruction(&mut cpu, &mut bus);
        assert_eq!(trace.cycles, 6, "RTI is always 6 cycles");
        assert_eq!(cpu.pc, 0x8003);
        let accesses = vec![
            Access::basic_read(0x9000, 0x40), // RTI
            Access::basic_read(0x9001, 0x69),
            Access::basic_read(0x01FC, 0x00), // dummy
            Access::basic_read(0x01FD, psr::I),
            Access::basic_read(0x01FE, 0x02),
            Access::basic_read(0x01FF, 0x80),
        ];
        let prefetch = Access::basic_read(0x8002, 0xEA);

        trace.assert_accesses(accesses);
        trace.assert_prefetch(prefetch);
    }
}

#[cfg(test)]
mod alu_accuracy {
    use crate::{cpu6502::flavor::*, psr::*};

    use super::*;

    fn setup_nmos(addr: u16, program: &[u8]) -> (Cpu6502<NMOS6502, Harness>, Harness) {
        let mut bus = Harness::with_program(addr, program);
        let mut cpu = Cpu6502::<NMOS6502, Harness>::new();

        cpu.reset(&mut bus);
        bus.clear_log(); // ignore reset-vector reads
        assert_eq!(cpu.pc, addr);
        (cpu, bus)
    }

    fn setup_cmos(addr: u16, program: &[u8]) -> (Cpu6502<CMOS65C02, Harness>, Harness) {
        let mut bus = Harness::with_program(addr, program);
        let mut cpu = Cpu6502::<CMOS65C02, Harness>::new();

        cpu.reset(&mut bus);
        bus.clear_log(); // ignore reset-vector reads
        assert_eq!(cpu.pc, addr);
        (cpu, bus)
    }

    fn setup_nes(addr: u16, program: &[u8]) -> (Cpu6502<NES, Harness>, Harness) {
        let mut bus = Harness::with_program(addr, program);
        let mut cpu = Cpu6502::<NES, Harness>::new();

        cpu.reset(&mut bus);
        bus.clear_log(); // ignore reset-vector reads
        assert_eq!(cpu.pc, addr);
        (cpu, bus)
    }

    /// A test case for ADC or SBC
    struct ArithCase {
        desc: &'static str,
        a: u8,
        operand: u8,
        carry_in: bool,
        expected_a: u8,
        expected_flags: u8,
    }

    #[test]
    fn adc_status_flags() {
        const FLAG_MASK: u8 = N | Z | C | V;

        let cases = [
            ArithCase {
                desc: "simple addition keeps flags clear",
                a: 0x0C,
                operand: 0x10,
                carry_in: false,
                expected_a: 0x1C,
                expected_flags: 0,
            },
            ArithCase {
                desc: "carry-in increments without setting carry out",
                a: 0x00,
                operand: 0x00,
                carry_in: true,
                expected_a: 0x01,
                expected_flags: 0,
            },
            ArithCase {
                desc: "carry-in produces zero result and carry out",
                a: 0xFF,
                operand: 0x00,
                carry_in: true,
                expected_a: 0x00,
                expected_flags: Z | C,
            },
            ArithCase {
                desc: "negative result without overflow",
                a: 0x80,
                operand: 0x00,
                carry_in: false,
                expected_a: 0x80,
                expected_flags: N,
            },
            ArithCase {
                desc: "overflow without carry",
                a: 0x50,
                operand: 0x50,
                carry_in: false,
                expected_a: 0xA0,
                expected_flags: N | V,
            },
            ArithCase {
                desc: "carry and overflow both asserted",
                a: 0x80,
                operand: 0x80,
                carry_in: false,
                expected_a: 0x00,
                expected_flags: Z | C | V,
            },
        ];

        for case in cases {
            let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x69, case.operand, 0xEA]);
            cpu.a = case.a;
            cpu.p = U_6502;
            if case.carry_in {
                cpu.p |= C;
            } else {
                cpu.p &= !C;
            }
            cpu.p &= !D;

            run_instruction(&mut cpu, &mut bus);

            assert_eq!(cpu.a, case.expected_a, "{}", case.desc);
            assert_eq!(
                cpu.p & FLAG_MASK,
                case.expected_flags,
                "{} set incorrect flags",
                case.desc
            );
        }
    }

    #[test]
    fn adc_decimal_mode() {
        const FLAG_MASK: u8 = N | Z | C | V;

        // 0x50 + 0x50 => 100 decimal, which produces a 0x00 result with decimal carry.
        let (mut nmos_cpu, mut nmos_bus) = setup_nmos(0x8000, &[0x69, 0x50, 0xEA]);
        nmos_cpu.a = 0x50;
        nmos_cpu.p = U_6502 | D;
        run_instruction(&mut nmos_cpu, &mut nmos_bus);

        assert_eq!(nmos_cpu.a, 0x00, "NMOS decimal result should wrap to 00");
        let nmos_flags = nmos_cpu.p & FLAG_MASK;
        assert_eq!(
            nmos_flags & (C | V),
            C | V,
            "NMOS should set carry and overflow"
        );
        assert_eq!(nmos_flags & N, N, "NMOS uses binary pre-adjust for N flag");
        assert_eq!(nmos_flags & Z, 0, "NMOS zero flag follows binary result");

        let (mut cmos_cpu, mut cmos_bus) = setup_cmos(0x8000, &[0x69, 0x50, 0xEA]);
        cmos_cpu.a = 0x50;
        cmos_cpu.p = U_6502 | D;
        run_instruction(&mut cmos_cpu, &mut cmos_bus);

        assert_eq!(cmos_cpu.a, 0x00, "CMOS decimal result should match NMOS");
        let cmos_flags = cmos_cpu.p & FLAG_MASK;
        assert_eq!(
            cmos_flags & (C | V),
            C | V,
            "CMOS should set carry and overflow"
        );
        assert_eq!(cmos_flags & N, 0, "CMOS N flag follows adjusted result");
        assert_eq!(cmos_flags & Z, Z, "CMOS zero flag follows adjusted result");

        let (mut nes_cpu, mut nes_bus) = setup_nes(0x8000, &[0x69, 0x50, 0xEA]);
        nes_cpu.a = 0x50;
        nes_cpu.p = U_6502 | D; // D flag ignored entirely
        run_instruction(&mut nes_cpu, &mut nes_bus);

        assert_eq!(
            nes_cpu.a, 0xA0,
            "NES should ignore decimal mode and behave like binary ADC"
        );
        let nes_flags = nes_cpu.p & FLAG_MASK;
        assert_eq!(nes_flags & (N | V), N | V, "NES should set binary N/V");
        assert_eq!(nes_flags & (Z | C), 0, "NES should leave Z/C clear");
    }

    #[test]
    fn sbc_status_flags() {
        const FLAG_MASK: u8 = N | Z | C | V;

        let cases = [
            ArithCase {
                desc: "basic subtraction keeps carry set",
                a: 0x10,
                operand: 0x01,
                carry_in: true,
                expected_a: 0x0F,
                expected_flags: C,
            },
            ArithCase {
                desc: "borrow clears carry and sets negative",
                a: 0x00,
                operand: 0x01,
                carry_in: true,
                expected_a: 0xFF,
                expected_flags: N,
            },
            ArithCase {
                desc: "cleared carry subtracts extra one",
                a: 0x10,
                operand: 0x01,
                carry_in: false,
                expected_a: 0x0E,
                expected_flags: C,
            },
            ArithCase {
                desc: "overflow occurs when subtracting negative",
                a: 0x80,
                operand: 0x7F,
                carry_in: true,
                expected_a: 0x01,
                expected_flags: C | V,
            },
            ArithCase {
                desc: "zero result sets Z while preserving carry",
                a: 0x34,
                operand: 0x34,
                carry_in: true,
                expected_a: 0x00,
                expected_flags: Z | C,
            },
        ];

        for case in cases {
            let (mut cpu, mut bus) = setup_nmos(0x8000, &[0xE9, case.operand, 0xEA]);
            cpu.a = case.a;
            cpu.p = U_6502;
            if case.carry_in {
                cpu.p |= C;
            } else {
                cpu.p &= !C;
            }
            cpu.p &= !D;

            run_instruction(&mut cpu, &mut bus);

            assert_eq!(cpu.a, case.expected_a, "{}", case.desc);
            assert_eq!(
                cpu.p & FLAG_MASK,
                case.expected_flags,
                "{} set incorrect flags",
                case.desc
            );
        }
    }

    #[test]
    fn sbc_decimal_mode() {
        const FLAG_MASK: u8 = N | Z | C | V;

        let (mut nmos_cpu, mut nmos_bus) = setup_nmos(0x8000, &[0xE9, 0x90, 0xEA]);
        nmos_cpu.a = 0x10;
        nmos_cpu.p = U_6502 | D | C;
        run_instruction(&mut nmos_cpu, &mut nmos_bus);

        assert_eq!(
            nmos_cpu.a, 0x20,
            "NMOS decimal subtract should store BCD result"
        );
        let nmos_flags = nmos_cpu.p & FLAG_MASK;
        assert_eq!(nmos_flags & (C | N), N, "NMOS N flag follows binary result");
        assert_eq!(
            nmos_flags & (Z | C),
            0,
            "NMOS zero flag follows binary result"
        );

        let (mut cmos_cpu, mut cmos_bus) = setup_cmos(0x8000, &[0xE9, 0x90, 0xEA]);
        cmos_cpu.a = 0x10;
        cmos_cpu.p = U_6502 | D | C;
        run_instruction(&mut cmos_cpu, &mut cmos_bus);

        assert_eq!(
            cmos_cpu.a, 0x20,
            "CMOS decimal subtract should match NMOS result"
        );
        let cmos_flags = cmos_cpu.p & FLAG_MASK;
        assert_eq!(cmos_flags & N, 0, "CMOS N flag follows decimal result");
        assert_eq!(
            cmos_flags & (Z | C),
            0,
            "CMOS zero flag tracks decimal result"
        );

        let (mut nes_cpu, mut nes_bus) = setup_nes(0x8000, &[0xE9, 0x15, 0xEA]);
        nes_cpu.a = 0x50;
        nes_cpu.p = U_6502 | D | C;
        run_instruction(&mut nes_cpu, &mut nes_bus);

        assert_eq!(nes_cpu.a, 0x3B, "NES should ignore decimal flag for SBC");
        let nes_flags = nes_cpu.p & FLAG_MASK;
        assert_eq!(nes_flags & C, C, "NES carry follows binary subtraction");
        assert_eq!(nes_flags & Z, 0, "NES zero follows binary result");
    }

    #[test]
    fn and() {
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x29, 0x0F, 0xEA]); // AND #$0F; NOP
        cpu.a = 0xF0;
        cpu.p = U_6502 | N;
        run_instruction(&mut cpu, &mut bus);

        assert_eq!(cpu.a, 0x00);
        assert_eq!(cpu.p & (N | Z), Z);
    }

    #[test]
    fn eor() {
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x49, 0x3F, 0xEA]); // EOR #$3F; NOP
        cpu.a = 0xF0;
        cpu.p = U_6502 | Z;
        run_instruction(&mut cpu, &mut bus);

        assert_eq!(cpu.a, 0xCF);
        assert_eq!(cpu.p & (N | Z), N);
    }

    #[test]
    fn ora() {
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x09, 0x3F, 0xEA]); // ORA #$0F; NOP
        cpu.a = 0x41;
        cpu.p = U_6502 | N | Z;
        run_instruction(&mut cpu, &mut bus);

        assert_eq!(cpu.a, 0x7F);
        assert_eq!(cpu.p & (N | Z), 0);
    }

    #[test]
    fn asl() {
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x0A, 0xEA]); // ASL A; NOP
        cpu.a = 0x81;
        cpu.p = U_6502;
        run_instruction(&mut cpu, &mut bus);

        assert_eq!(cpu.a, 0x02);
        assert_eq!(cpu.p & C, C);
    }

    #[test]
    fn bcc() {
        // take branch:
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x90, 0x05]);
        bus.mem[0x8006] = 0xEA;
        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.pc, 0x8008);

        // don't take branch:
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x90, 0x05, 0xEA]);
        cpu.p = C;
        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.pc, 0x8003);
    }

    #[test]
    fn bcs() {
        // take branch:
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0xB0, 0x05]);
        bus.mem[0x8006] = 0xEA;
        cpu.p = C;
        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.pc, 0x8008);

        // don't take branch:
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0xB0, 0x05, 0xEA]);
        cpu.p = 0;
        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.pc, 0x8003);
    }

    #[test]
    fn beq() {
        // take branch:
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0xF0, 0x05]);
        bus.mem[0x8006] = 0xEA;
        cpu.p = Z;
        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.pc, 0x8008);

        // don't take branch:
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0xF0, 0x05, 0xEA]);
        cpu.p = 0;
        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.pc, 0x8003);
    }

    #[test]
    fn bit() {
        const FLAG_MASK: u8 = N | Z | C | V;

        // NMOS BIT zp: Z from A&M, N/V from memory, C unaffected.
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x24, 0x10, 0xEA]); // BIT $10; NOP
        bus.mem[0x0010] = 0b1100_0000; // N=1, V=1
        cpu.a = 0b0000_1111; // AND with operand yields 0 -> Z set
        cpu.p = U_6502 | C; // carry set so we can see it's preserved

        run_instruction(&mut cpu, &mut bus);

        assert_eq!(cpu.a, 0b0000_1111, "BIT must not modify A");
        let flags = cpu.p & FLAG_MASK;
        assert_eq!(flags & Z, Z, "BIT should set Z when A&M == 0");
        assert_eq!(flags & (N | V), N | V, "BIT should copy N/V from memory");
        assert_eq!(flags & C, C, "BIT should preserve carry");

        // NMOS BIT zp with non-zero result and clear N/V in memory.
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x24, 0x10, 0xEA]); // BIT $10; NOP
        bus.mem[0x0010] = 0b0000_0011; // N=0, V=0
        cpu.a = 0b0000_0001; // AND with operand yields non-zero -> Z clear
        cpu.p = U_6502 | C | N | V | Z; // start with all these bits set

        run_instruction(&mut cpu, &mut bus);

        assert_eq!(cpu.a, 0x01, "BIT must not modify A");
        let flags = cpu.p & FLAG_MASK;
        assert_eq!(flags & Z, 0, "BIT clears Z when A&M != 0");
        assert_eq!(flags & (N | V), 0, "BIT copies N/V from memory");
        assert_eq!(flags & C, C, "BIT preserves carry in all cases");

        // 65C02 BIT #imm only affects Z; N/V remain unchanged.
        let (mut cpu, mut bus) = setup_cmos(0x8000, &[0x89, 0x80, 0xEA]); // BIT #$80; NOP
        cpu.a = 0x80;
        cpu.p = U_6502 | N | V; // start with N/V set so we can see that they persist

        run_instruction(&mut cpu, &mut bus);

        assert_eq!(cpu.a, 0x80, "BIT #imm must not modify A");
        let flags = cpu.p & FLAG_MASK;
        // A & 0x80 != 0 -> Z cleared; N/V should remain as they were.
        assert_eq!(flags & Z, 0, "BIT #imm sets Z from A & operand");
        assert_eq!(flags & (N | V), N, "BIT #imm copies N/V from operand");
    }

    #[test]
    fn bmi() {
        // take branch:
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x30, 0x05]);
        cpu.p = N;
        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.pc, 0x8008);

        // don't take branch:
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x30, 0x05]);
        cpu.p = 0;
        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.pc, 0x8003);
    }

    #[test]
    fn bne() {
        // take branch:
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0xD0, 0x05]);
        cpu.p = 0;
        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.pc, 0x8008);

        // don't take branch:
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0xD0, 0x05]);
        cpu.p = Z;
        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.pc, 0x8003);
    }

    #[test]
    fn bpl() {
        // take branch:
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x10, 0x05]);
        cpu.p = 0;
        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.pc, 0x8008);

        // don't take branch:
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x10, 0x05]);
        cpu.p = N;
        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.pc, 0x8003);
    }

    #[test]
    fn jsr_rts() {
        // JSR/RTS should round-trip PC and stack without touching A/X/Y or flags.
        // We only care about ALU-visible state here, not exact cycle behavior.
        //
        // Layout:
        //  $8000: 20 00 90   ; JSR $9000
        //  $8003: EA         ; NOP (code we "return" to)
        //
        //  $9000:            ; RTS
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x20, 0x00, 0x90, 0xEA]); // JSR $9000; NOP

        // Subroutine body: first word is just a NOP (prefetch target), second byte is RTS.
        bus.mem[0x9000] = 0x60; // RTS

        // Set up known register and flag state.
        cpu.s = 0xFF;
        cpu.a = 0x12;
        cpu.x = 0x34;
        cpu.y = 0x56;
        cpu.p = U_6502 | C | Z;

        // Execute JSR.
        run_instruction(&mut cpu, &mut bus);

        // JSR must push return address ($8002) and land in the subroutine.
        assert_eq!(cpu.s, 0xFD, "JSR must push two bytes on the stack");
        assert_eq!(
            bus.mem[0x01FF], 0x80,
            "JSR should push high byte of return PC"
        );
        assert_eq!(
            bus.mem[0x01FE], 0x02,
            "JSR should push low byte of return PC"
        );
        assert_eq!(
            cpu.pc, 0x9001,
            "JSR should transfer control to subroutine (post-prefetch)"
        );

        // Registers and flags should be unchanged (JSR is not supposed to touch them).
        assert_eq!(cpu.a, 0x12, "JSR must not modify A");
        assert_eq!(cpu.x, 0x34, "JSR must not modify X");
        assert_eq!(cpu.y, 0x56, "JSR must not modify Y");
        assert_eq!(cpu.p, U_6502 | C | Z, "JSR must not modify flags");

        // Now execute RTS from the subroutine.
        run_instruction(&mut cpu, &mut bus);

        // RTS should pull the same return address and restore S.
        assert_eq!(cpu.s, 0xFF, "RTS must restore the stack pointer");
        assert_eq!(
            cpu.pc, 0x8004,
            "RTS should return to the instruction following the JSR"
        );

        // JSR/RTS must leave registers and flags exactly as they were.
        assert_eq!(cpu.a, 0x12, "RTS must not modify A");
        assert_eq!(cpu.x, 0x34, "RTS must not modify X");
        assert_eq!(cpu.y, 0x56, "RTS must not modify Y");
        assert_eq!(cpu.p, U_6502 | C | Z, "RTS must not modify flags");
    }

    #[test]
    fn brk_rti() {
        // Use NMOS core here; BRK semantics are the baseline 6502 behavior.
        // We focus on stack contents and final flags/PC, not cycle timing (covered elsewhere).
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x00, 0xEA]); // BRK; NOP (as padding)

        // Set stack pointer to a known value, and PSR to a known pattern.
        cpu.s = 0xFF;
        cpu.p = U_6502 | C | Z; // some arbitrary flags set; B and I will be modified by BRK

        // Set the IRQ/BRK vector to a known address.
        bus.mem[0xFFFE] = 0x34;
        bus.mem[0xFFFF] = 0x12; // vector = $1234
        bus.mem[0x1234] = 0x40; // RTI opcode at interrupt vector
        bus.mem[0x1235] = 0xEA; // NOP after RTI for a safe prefetch

        run_instruction(&mut cpu, &mut bus);

        // After BRK, PC should point to the vector target.
        assert_eq!(
            cpu.pc, 0x1235,
            "BRK should jump via the IRQ/BRK vector and increment PC"
        );

        // Stack pointer should have decremented three times (PCH, PCL, P pushed).
        assert_eq!(cpu.s, 0xFC, "BRK must push three bytes on the stack");

        // Check the pushed PC: BRK pushes PC+2 (the return address *after* the BRK's operand).
        // Starting PC was 0x8000, so pushed address should be 0x8002.
        assert_eq!(
            bus.mem[0x01FF], 0x80,
            "High byte of return PC should be 0x80"
        );
        assert_eq!(
            bus.mem[0x01FE], 0x02,
            "Low byte of return PC should be 0x02"
        );

        // Check the pushed status byte:
        // - Bit 4 (B flag) must be set in the pushed value.
        // - Bit 5 is always set.
        // - I should be set in the *live* status register after BRK, but we don't rely on its previous value.
        let pushed_p = bus.mem[0x01FD];
        assert_ne!(
            pushed_p & B_6502,
            0,
            "Pushed status must have Break flag set"
        );
        assert_ne!(
            pushed_p & U_6502,
            0,
            "Pushed status must have unused bit 5 set"
        );

        // Live flags after BRK:
        // - I should be set
        // - B in the *live* P is typically cleared in emulators, but the pushed copy retains it.
        assert_ne!(cpu.p & I, 0, "Interrupt Disable flag must be set after BRK");

        // Now execute RTI from the interrupt vector and verify that CPU state is restored.
        run_instruction(&mut cpu, &mut bus);

        // RTI should pull the status and return address from the stack.
        // BRK pushed $8002, so RTI should set PC to $8003.
        assert_eq!(
            cpu.pc, 0x8003,
            "RTI should return to the instruction after BRK's operand"
        );
        assert_eq!(cpu.s, 0xFF, "RTI must restore the stack pointer");

        // The I flag should be restored to its original value (cleared in this setup),
        // and the arithmetic flags we set (C and Z) should survive the BRK/RTI round trip.
        assert_eq!(
            cpu.p & I,
            0,
            "RTI should restore the original Interrupt Disable flag"
        );
        assert_eq!(
            cpu.p & (C | Z),
            (U_6502 | C | Z) & (C | Z),
            "RTI should restore carry and zero flags from the stack"
        );
    }

    #[test]
    fn bvc() {
        // take branch:
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x50, 0x05]);
        cpu.p = 0;
        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.pc, 0x8008);

        // don't take branch:
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x50, 0x05]);
        cpu.p = V;
        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.pc, 0x8003);
    }

    #[test]
    fn bvs() {
        // take branch:
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x70, 0x05]);
        cpu.p = V;
        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.pc, 0x8008);

        // don't take branch:
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x70, 0x05]);
        cpu.p = 0;
        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.pc, 0x8003);
    }

    #[test]
    fn bra() {
        // take branch:
        let (mut cpu, mut bus) = setup_cmos(0x8000, &[0x80, 0x05]);
        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.pc, 0x8008);

        // impossible not to branch on BRA
    }

    #[test]
    fn clc_cld_cli_clv() {
        let (mut cpu, mut bus) = setup_cmos(
            0x8000,
            &[
                0x18, // CLC
                0xD8, // CLD
                0x58, // CLI
                0xB8, // CLV
            ],
        );
        cpu.p = C | D | I | V;
        run_instruction(&mut cpu, &mut bus); // CLC
        assert_eq!(cpu.p, D | I | V);
        run_instruction(&mut cpu, &mut bus); // CLD
        assert_eq!(cpu.p, I | V);
        run_instruction(&mut cpu, &mut bus); // CLI
        assert_eq!(cpu.p, V);
        run_instruction(&mut cpu, &mut bus); // CLV
        assert_eq!(cpu.p, 0);
    }

    #[test]
    fn sec_sed_sei() {
        let (mut cpu, mut bus) = setup_cmos(
            0x8000,
            &[
                0x38, // SEC
                0xF8, // SED
                0x78, // SEI
            ],
        );
        cpu.p = 0;
        run_instruction(&mut cpu, &mut bus); // SEC
        assert_eq!(cpu.p, C);
        run_instruction(&mut cpu, &mut bus); // SED
        assert_eq!(cpu.p, C | D);
        run_instruction(&mut cpu, &mut bus); // SEI
        assert_eq!(cpu.p, C | D | I);
    }

    #[test]
    fn cmp() {
        // CMP performs A - M (without changing A) and sets:
        //  - C = 1 if A >= M, else 0
        //  - Z = 1 if A == M, else 0
        //  - N from bit 7 of the subtraction result
        //  - V is unaffected

        const FLAG_MASK: u8 = N | Z | C;

        struct CmpCase {
            desc: &'static str,
            a: u8,
            operand: u8,
            expected_flags: u8,
        }

        let cases = [
            CmpCase {
                desc: "A > M, positive result, carry set",
                a: 0x10,
                operand: 0x01,
                expected_flags: C, // 0x10 - 0x01 = 0x0F
            },
            CmpCase {
                desc: "A == M, zero result, carry set",
                a: 0x42,
                operand: 0x42,
                expected_flags: Z | C, // 0x42 - 0x42 = 0x00
            },
            CmpCase {
                desc: "A < M, negative result, carry clear",
                a: 0x01,
                operand: 0x02,
                expected_flags: N, // 0x01 - 0x02 = 0xFF
            },
            CmpCase {
                desc: "A >= M across sign boundary",
                a: 0x80,
                operand: 0x7F,
                expected_flags: C, // 0x80 - 0x7F = 0x01
            },
        ];

        for case in cases {
            let (mut cpu, mut bus) = setup_nmos(0x8000, &[0xC9, case.operand, 0xEA]); // CMP #imm; NOP
            cpu.a = case.a;
            cpu.p = U_6502 | V; // set V so we can verify it is preserved

            run_instruction(&mut cpu, &mut bus);

            // CMP must not modify A.
            assert_eq!(cpu.a, case.a, "{}: CMP should not change A", case.desc);

            // Check N/Z/C against expectations.
            let flags = cpu.p & FLAG_MASK;
            assert_eq!(
                flags, case.expected_flags,
                "{}: CMP produced incorrect flags",
                case.desc
            );

            // Overflow flag must be unaffected.
            assert_eq!(cpu.p & V, V, "{}: CMP should not modify V flag", case.desc);
        }
    }

    #[test]
    fn cpx_cpy() {
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0xE0, 0x01, 0xEA]); // CPX #$01; NOP
        cpu.x = 0x10;
        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.p & C, C);
        assert_eq!(cpu.x, 0x10);

        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0xC0, 0x42, 0xEA]); // CPY #$42; NOP
        cpu.y = 0x42;
        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.p & (C | Z), C | Z);
        assert_eq!(cpu.y, 0x42);
    }

    #[test]
    fn dec_dex_dey() {
        let (mut cpu, mut bus) = setup_cmos(
            0x8000,
            &[
                0xC6, 0x69, // DEC $69
                0xCA, // DEX
                0x88, // DEY
                0xEA, // NOP
            ],
        );
        bus.mem[0x0069] = 0xFF;
        cpu.x = 0x45;
        cpu.y = 0x8D;
        run_instruction(&mut cpu, &mut bus); // DEC $69
        assert_eq!(bus.mem[0x0069], 0xFE);
        run_instruction(&mut cpu, &mut bus); // DEX
        assert_eq!(cpu.x, 0x44);
        run_instruction(&mut cpu, &mut bus); // DEY
        assert_eq!(cpu.y, 0x8C);
    }

    #[test]
    fn inc_inx_iny() {
        let (mut cpu, mut bus) = setup_cmos(
            0x8000,
            &[
                0xE6, 0x69, // INC $69
                0xE8, // INX
                0xC8, // INY
                0xEA, // NOP
            ],
        );
        bus.mem[0x0069] = 0xFF;
        cpu.x = 0x45;
        cpu.y = 0x8D;
        run_instruction(&mut cpu, &mut bus); // INC $69
        assert_eq!(bus.mem[0x0069], 0x00);
        run_instruction(&mut cpu, &mut bus); // INX
        assert_eq!(cpu.x, 0x46);
        run_instruction(&mut cpu, &mut bus); // INY
        assert_eq!(cpu.y, 0x8E);
    }

    #[test]
    fn lda_ldx_ldy() {
        let (mut cpu, mut bus) = setup_nmos(
            0x8000,
            &[
                0xA9, 0x00, // LDA #$00
                0xA2, 0x80, // LDX #$80
                0xA0, 0x42, // LDY #$42
                0xEA, // NOP
            ],
        );
        cpu.a = 0xFF;
        cpu.x = 0xFF;
        cpu.y = 0xFF;
        cpu.p = U_6502 | C | V | N | Z;

        // LDA #$00
        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.a, 0x00, "LDA should load immediate into A");
        assert_eq!(cpu.x, 0xFF, "LDA must not modify X");
        assert_eq!(cpu.y, 0xFF, "LDA must not modify Y");
        assert_eq!(cpu.p & (N | Z), Z, "LDA should set Z and clear N for 0x00");
        assert_eq!(
            cpu.p & (C | V),
            C | V,
            "LDA should not modify carry or overflow flags"
        );

        // LDX #$80
        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.x, 0x80, "LDX should load immediate into X");
        assert_eq!(cpu.a, 0x00, "LDX must not modify A");
        assert_eq!(cpu.y, 0xFF, "LDX must not modify Y");
        assert_eq!(cpu.p & (N | Z), N, "LDX should set N and clear Z for 0x80");
        assert_eq!(
            cpu.p & (C | V),
            C | V,
            "LDX should not modify carry or overflow flags"
        );

        // LDY #$42
        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.y, 0x42, "LDY should load immediate into Y");
        assert_eq!(cpu.a, 0x00, "LDY must not modify A");
        assert_eq!(cpu.x, 0x80, "LDY must not modify X");
        assert_eq!(
            cpu.p & (N | Z),
            0,
            "LDY should clear N and Z for non-zero, positive value"
        );
        assert_eq!(
            cpu.p & (C | V),
            C | V,
            "LDY should not modify carry or overflow flags"
        );
    }

    #[test]
    fn lsr() {
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x4A, 0xEA]); // LSR A; NOP
        cpu.a = 0xF3;
        cpu.p = U_6502 | N | Z;
        run_instruction(&mut cpu, &mut bus);

        assert_eq!(cpu.a, 0x79);
        assert_eq!(cpu.p & (N | Z | C), C);
    }

    #[test]
    fn pha_php_pla_plp() {
        // --- PHA / PLA pair: accumulator round-trip and flag behavior ---
        let (mut cpu, mut bus) = setup_nmos(
            0x8000,
            &[
                0x48, // PHA
                0x68, // PLA
                0xEA, // NOP
            ],
        );

        cpu.s = 0xFF;
        cpu.a = 0x42;
        cpu.x = 0x11;
        cpu.y = 0x22;
        cpu.p = U_6502 | C | V; // keep C/V set to verify they survive PLA

        // Execute PHA.
        run_instruction(&mut cpu, &mut bus);

        // PHA pushes A to the current stack location and decrements S.
        assert_eq!(cpu.s, 0xFE, "PHA must decrement the stack pointer");
        assert_eq!(
            bus.mem[0x01FF], 0x42,
            "PHA must push the accumulator onto the stack"
        );
        assert_eq!(cpu.a, 0x42, "PHA must not modify A");
        assert_eq!(cpu.p, U_6502 | C | V, "PHA must not modify flags");
        assert_eq!(cpu.x, 0x11, "PHA must not modify X");
        assert_eq!(cpu.y, 0x22, "PHA must not modify Y");

        // Execute PLA.
        run_instruction(&mut cpu, &mut bus);

        // PLA pulls from stack (incrementing S first), stores into A, and updates N/Z only.
        assert_eq!(cpu.s, 0xFF, "PLA must restore the stack pointer");
        assert_eq!(cpu.a, 0x42, "PLA must restore the pushed accumulator value");
        assert_eq!(
            cpu.p & (N | Z),
            0,
            "PLA should clear N and Z for non-zero, positive value"
        );
        assert_eq!(
            cpu.p & (C | V),
            C | V,
            "PLA must not modify carry or overflow flags"
        );
        assert_eq!(cpu.x, 0x11, "PLA must not modify X");
        assert_eq!(cpu.y, 0x22, "PLA must not modify Y");

        // --- PHP / PLP pair: status round-trip and B/U behavior ---
        let (mut cpu, mut bus) = setup_nmos(
            0x9000,
            &[
                0x08, // PHP
                0x28, // PLP
                0xEA, // NOP
            ],
        );

        cpu.s = 0xFF;
        cpu.a = 0x99;
        cpu.x = 0x33;
        cpu.y = 0x44;
        cpu.p = U_6502 | C | Z; // known flags to track through PHP/PLP

        // Execute PHP.
        run_instruction(&mut cpu, &mut bus);

        // PHP pushes a copy of P with B and U bits set in the pushed value.
        assert_eq!(cpu.s, 0xFE, "PHP must decrement the stack pointer");
        let pushed_p = bus.mem[0x01FF];
        assert_eq!(
            pushed_p & (C | Z),
            C | Z,
            "PHP must push a copy of C/Z to the stack"
        );
        assert_ne!(
            pushed_p & B_6502,
            0,
            "PHP must set the Break flag bit in the pushed status"
        );
        assert_ne!(
            pushed_p & U_6502,
            0,
            "PHP must set the unused bit 5 in the pushed status"
        );
        // Live P must remain unchanged.
        assert_eq!(cpu.p & (C | Z), C | Z, "PHP must not modify live C/Z flags");

        // Execute PLP.
        run_instruction(&mut cpu, &mut bus);

        // PLP pulls status from the stack and replaces P.
        assert_eq!(cpu.s, 0xFF, "PLP must restore the stack pointer");
        assert_eq!(
            cpu.p & (C | Z),
            C | Z,
            "PLP must restore C/Z flags from the stack"
        );
        // A/X/Y should be untouched by PHP/PLP.
        assert_eq!(cpu.a, 0x99, "PHP/PLP must not modify A");
        assert_eq!(cpu.x, 0x33, "PHP/PLP must not modify X");
        assert_eq!(cpu.y, 0x44, "PHP/PLP must not modify Y");
    }

    #[test]
    fn phx_plx_phy_ply() {
        // These instructions exist only on the CMOS 65C02
        let (mut cpu, mut bus) = setup_cmos(
            0x8000,
            &[
                0xDA, // PHX
                0xFA, // PLX
                0xEA, // NOP
            ],
        );

        cpu.s = 0xFF;
        cpu.x = 0x37;
        cpu.a = 0x11;
        cpu.y = 0x22;
        cpu.p = U_6502 | C | Z | N | V; // non-trivial flags to ensure PHX/PLX don't scramble them (except N/Z on PLX)

        run_instruction(&mut cpu, &mut bus); // PHX
        assert_eq!(cpu.s, 0xFE, "PHX must decrement the stack pointer");
        assert_eq!(bus.mem[0x01FF], 0x37, "PHX must push X onto the stack");
        assert_eq!(cpu.x, 0x37, "PHX must not modify X");
        assert_eq!(cpu.a, 0x11, "PHX must not modify A");
        assert_eq!(cpu.y, 0x22, "PHX must not modify Y");
        assert_eq!(
            cpu.p,
            U_6502 | C | Z | N | V,
            "PHX must not modify any status flags"
        );

        cpu.x = 0x00; // inject into x to make sure we are pulling correctly
        run_instruction(&mut cpu, &mut bus); // PLX
        assert_eq!(cpu.s, 0xFF, "PLX must restore the stack pointer");
        assert_eq!(cpu.x, 0x37, "PLX must pull the value back into X");
        // 0x37 is positive and non-zero.
        assert_eq!(
            cpu.p & (N | Z),
            0,
            "PLX should clear N and Z for a positive, non-zero result"
        );
        // Other flags must remain as they were (C/V).
        assert_eq!(cpu.p & (C | V), C | V, "PLX must preserve C and V");
        assert_eq!(cpu.a, 0x11, "PLX must not modify A");
        assert_eq!(cpu.y, 0x22, "PLX must not modify Y");

        // PHY/PLY: push/pop Y with N/Z set on pull, no other flags touched.
        let (mut cpu, mut bus) = setup_cmos(
            0x9000,
            &[
                0x5A, // PHY
                0x7A, // PLY
                0xEA, // NOP
            ],
        );

        cpu.s = 0xFF;
        cpu.y = 0x80;
        cpu.a = 0x33;
        cpu.x = 0x44;
        cpu.p = U_6502 | C | Z; // Z set so we can see it clear, C set to ensure preserved

        run_instruction(&mut cpu, &mut bus); // PHY
        assert_eq!(cpu.s, 0xFE, "PHY must decrement the stack pointer");
        assert_eq!(bus.mem[0x01FF], 0x80, "PHY must push Y onto the stack");
        assert_eq!(cpu.y, 0x80, "PHY must not modify Y");
        assert_eq!(cpu.a, 0x33, "PHY must not modify A");
        assert_eq!(cpu.x, 0x44, "PHY must not modify X");
        assert_eq!(
            cpu.p,
            U_6502 | C | Z,
            "PHY must not modify any status flags"
        );

        run_instruction(&mut cpu, &mut bus); // PLY
        assert_eq!(cpu.s, 0xFF, "PLY must restore the stack pointer");
        assert_eq!(cpu.y, 0x80, "PLY must pull the value back into Y");
        // 0x80 is negative, non-zero.
        assert_eq!(cpu.p & (N | Z), N, "PLY with 0x80 should set N and clear Z");
        assert_eq!(cpu.p & C, C, "PLY must preserve carry flag");
        assert_eq!(cpu.a, 0x33, "PLY must not modify A");
        assert_eq!(cpu.x, 0x44, "PLY must not modify X");
    }

    #[test]
    fn rol_ror() {
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x2A, 0x6A, 0xEA]); // ROL A; ROR A; NOP

        cpu.a = 0x00;
        cpu.p = U_6502 | C | V; // set C (for ROL carry-in) and V (to verify it is preserved)

        run_instruction(&mut cpu, &mut bus); // ROL A
        assert_eq!(cpu.a, 0x01, "ROL A should shift in carry and produce 0x01");
        assert_eq!(
            cpu.p & (N | Z | C),
            0,
            "ROL A with A=0x00,C=1 should leave N/Z/C all clear"
        );
        assert_eq!(cpu.p & V, V, "ROL A must not modify V flag");

        run_instruction(&mut cpu, &mut bus); // ROR A
        assert_eq!(
            cpu.a, 0x00,
            "ROR A should rotate 0x01 with C=0 back to 0x00"
        );
        assert_eq!(
            cpu.p & (N | Z | C),
            Z | C,
            "ROR A on A=0x01,C=0 should set Z and C"
        );
        assert_eq!(cpu.p & V, V, "ROR A must not modify V flag");
    }

    #[test]
    fn sta_stx_sty_stz() {
        let (mut cpu, mut bus) = setup_cmos(
            0x8000,
            &[
                0x85, 0x10, // STA $10
                0x86, 0x11, // STX $11
                0x84, 0x12, // STY $12
                0x64, 0x13, // STZ $13
                0xEA, // NOP
            ],
        );

        // Initialize registers and flags to known, non-trivial values so we can
        // detect accidental modification by store instructions.
        cpu.a = 0xAA;
        cpu.x = 0xBB;
        cpu.y = 0xCC;
        bus.mem[0x13] = 0x26; // some data here so stz actually changes something
        cpu.p = U_6502 | C | Z | N | V; // some arbitrary mix of flags

        // --- STA $10 ---
        run_instruction(&mut cpu, &mut bus);
        assert_eq!(bus.mem[0x0010], 0xAA, "STA must store A into memory");
        assert_eq!(cpu.a, 0xAA, "STA must not modify A");
        assert_eq!(
            cpu.p,
            U_6502 | C | Z | N | V,
            "STA must not modify status flags on 65C02"
        );

        // --- STX $11 ---
        run_instruction(&mut cpu, &mut bus);
        assert_eq!(bus.mem[0x0011], 0xBB, "STX must store X into memory");
        assert_eq!(cpu.x, 0xBB, "STX must not modify X");
        assert_eq!(
            cpu.p,
            U_6502 | C | Z | N | V,
            "STX must not modify status flags on 65C02"
        );

        // --- STY $12 ---
        run_instruction(&mut cpu, &mut bus);
        assert_eq!(bus.mem[0x0012], 0xCC, "STY must store Y into memory");
        assert_eq!(cpu.y, 0xCC, "STY must not modify Y");
        assert_eq!(
            cpu.p,
            U_6502 | C | Z | N | V,
            "STY must not modify status flags on 65C02"
        );

        // --- STZ $13 ---
        run_instruction(&mut cpu, &mut bus);
        assert_eq!(bus.mem[0x0013], 0x00, "STZ must store zero into memory");
        // STZ should not alter any registers.
        assert_eq!(cpu.a, 0xAA, "STZ must not modify A");
        assert_eq!(cpu.x, 0xBB, "STZ must not modify X");
        assert_eq!(cpu.y, 0xCC, "STZ must not modify Y");
        assert_eq!(
            cpu.p,
            U_6502 | C | Z | N | V,
            "STZ must not modify status flags on 65C02"
        );
    }

    #[test]
    fn tax_tay_txa_tya_tsx_txs() {
        // TAX: transfer A to X, set N/Z from result, do not modify A or other flags.
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0xAA, 0xEA]); // TAX; NOP
        cpu.a = 0x00;
        cpu.x = 0xFF;
        cpu.p = U_6502 | N | C | V; // start with N set so we can see it change

        run_instruction(&mut cpu, &mut bus);

        assert_eq!(cpu.x, 0x00, "TAX should copy A into X");
        assert_eq!(cpu.a, 0x00, "TAX must not modify A");
        assert_eq!(
            cpu.p & (N | Z),
            Z,
            "TAX with zero result should set Z and clear N"
        );
        assert_eq!(
            cpu.p & (C | V),
            C | V,
            "TAX must not modify carry or overflow flags"
        );

        // TAY: transfer A to Y, with a negative result.
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0xA8, 0xEA]); // TAY; NOP
        cpu.a = 0x80;
        cpu.y = 0x00;
        cpu.p = U_6502 | Z | C; // Z set so we can see it clear

        run_instruction(&mut cpu, &mut bus);

        assert_eq!(cpu.y, 0x80, "TAY should copy A into Y");
        assert_eq!(cpu.a, 0x80, "TAY must not modify A");
        assert_eq!(cpu.p & (N | Z), N, "TAY with 0x80 should set N and clear Z");
        assert_eq!(cpu.p & C, C, "TAY must not modify carry flag");

        // TXA: transfer X to A, zero result.
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x8A, 0xEA]); // TXA; NOP
        cpu.x = 0x00;
        cpu.a = 0xFF;
        cpu.p = U_6502 | N | C; // N set so TXA has to clear it

        run_instruction(&mut cpu, &mut bus);

        assert_eq!(cpu.a, 0x00, "TXA should copy X into A");
        assert_eq!(cpu.x, 0x00, "TXA must not modify X");
        assert_eq!(
            cpu.p & (N | Z),
            Z,
            "TXA with zero result should set Z and clear N"
        );
        assert_eq!(cpu.p & C, C, "TXA must not modify carry flag");

        // TYA: transfer Y to A, negative result.
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x98, 0xEA]); // TYA; NOP
        cpu.y = 0xFF;
        cpu.a = 0x00;
        cpu.p = U_6502 | Z | C; // Z set so we can see it clear

        run_instruction(&mut cpu, &mut bus);

        assert_eq!(cpu.a, 0xFF, "TYA should copy Y into A");
        assert_eq!(cpu.y, 0xFF, "TYA must not modify Y");
        assert_eq!(cpu.p & (N | Z), N, "TYA with 0xFF should set N and clear Z");
        assert_eq!(cpu.p & C, C, "TYA must not modify carry flag");

        // TSX: transfer S to X, set N/Z, do not modify S or other flags.
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0xBA, 0xEA]); // TSX; NOP
        cpu.s = 0x80;
        cpu.x = 0x00;
        cpu.p = U_6502 | Z | C; // Z set so we can see it clear, C set to ensure preserved

        run_instruction(&mut cpu, &mut bus);

        assert_eq!(cpu.x, 0x80, "TSX should copy S into X");
        assert_eq!(cpu.s, 0x80, "TSX must not modify S");
        assert_eq!(cpu.p & (N | Z), N, "TSX with 0x80 should set N and clear Z");
        assert_eq!(cpu.p & C, C, "TSX must not modify carry flag");

        // TXS: transfer X to S, do not modify any flags.
        let (mut cpu, mut bus) = setup_nmos(0x8000, &[0x9A, 0xEA]); // TXS; NOP
        cpu.x = 0x12;
        cpu.s = 0xFF;
        cpu.p = U_6502 | N | Z | C; // non-trivial mix to ensure TXS leaves flags alone

        run_instruction(&mut cpu, &mut bus);

        assert_eq!(cpu.s, 0x12, "TXS should copy X into S");
        assert_eq!(cpu.x, 0x12, "TXS must not modify X");
        assert_eq!(
            cpu.p,
            U_6502 | N | Z | C,
            "TXS must not modify any status flags"
        );
    }

    #[test]
    fn bbr_bbs() {
        // Use a jump-over-landmines scheme: a chain of BBR/BBS instructions that must
        // correctly skip over embedded BRKs. If any branch condition is wrong, we will
        // hit a BRK and the test will either change PC/stack unexpectedly or fail the
        // final PC assertions.
        let (mut cpu, mut bus) = setup_cmos(
            0x8000,
            &[
                0x0F, 0xA9, 0x01, // BBR0 $EE, #$01
                0x3F, 0xA9, 0x01, // BBR3 $EE, #$01
                0x00, // BRK (will be jumped over by BBR3)
                0x7F, 0xA9, 0x01, // BBR7 $EE, #$01
                0x00, // BRK (will be jumped over by BBR7)
                0x9F, 0xA9, 0x01, // BBS1 $EE, #$01
                0xBF, 0xA9, 0x01, // BBS3 $EE, #$01
                0xCF, 0xA9, 0x01, // BBS4 $EE, #$01
                0x00, // BRK (will be jumped over by BBS4)
                0xEA, // final NOP for prefetch
            ],
        );
        bus.mem[0xA9] = 0b0101_0001;
        // Prime stack and flags so we can detect any accidental BRK
        // (which would push to the stack and change I/B).
        cpu.s = 0xFF;
        // We use Z here because if we jump over the first byte of a BBS/BBR we land on the two
        // bytes [0xA9, 0x01], which correspond to LDA #$01, which would clear the Z bit
        cpu.p = U_6502 | Z;

        // Helper to assert that BBR/BBS did not modify flags or stack and did not
        // clobber the test byte.
        let assert_side_effects_intact = |cpu: &Cpu6502<CMOS65C02, Harness>, bus: &Harness| {
            assert_eq!(cpu.s, 0xFF, "BBR/BBS must not change the stack pointer");
            assert_eq!(cpu.p, U_6502 | Z, "BBR/BBS must not modify status flags");
            assert_eq!(
                bus.mem[0xA9], 0b0101_0001,
                "BBR/BBS must not modify the zero-page operand"
            );
        };

        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.pc, 0x8004, "BBR0 should not branch when bit 0 is set");
        assert_side_effects_intact(&cpu, &bus);

        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.pc, 0x8008, "BBR3 should branch bit 3 is clear");
        assert_side_effects_intact(&cpu, &bus);

        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.pc, 0x800C, "BBR7 should branch when bit 7 is clear");
        assert_side_effects_intact(&cpu, &bus);

        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.pc, 0x800F, "BBS1 should not branch when bit 1 is clear");
        assert_side_effects_intact(&cpu, &bus);

        run_instruction(&mut cpu, &mut bus);
        assert_eq!(cpu.pc, 0x8012, "BBS3 should not branch when bit 3 is clear");
        assert_side_effects_intact(&cpu, &bus);

        run_instruction(&mut cpu, &mut bus);
        assert_eq!(
            cpu.pc, 0x8016,
            "BBS4 should branch over the BRK and finish at $8016 (post-prefetch)"
        );
        assert_side_effects_intact(&cpu, &bus);
        assert_eq!(cpu.current_opcode, 0xEA, "We should have prefetched NOP");
    }

    #[test]
    fn smb_rmb() {
        let (mut cpu, mut bus) = setup_cmos(
            0x8000,
            &[
                0x87, 0x10, // SMB0 $10 : set bit 0
                0xC7, 0x10, // SMB4 $10 : set bit 4
                0x27, 0x10, // RMB2 $10 : clear bit 2
                0x47, 0x10, // RMB4 $10 : clear bit 4
                0xEA, // NOP (landing / prefetch padding)
            ],
        );
        // Give registers and flags non-trivial values so we can detect
        // any accidental modifications.
        cpu.a = 0xAA;
        cpu.x = 0xBB;
        cpu.y = 0xCC;
        cpu.s = 0xF0;
        cpu.p = U_6502 | C | Z | N | V; // arbitrary mix; SMB/RMB must leave this exactly as-is
        let initial_p = cpu.p;
        // Helper to assert that only the target zero-page byte changed.
        let assert_state =
            |cpu: &Cpu6502<CMOS65C02, Harness>, bus: &Harness, expected_byte: u8, msg: &str| {
                assert_eq!(
                    bus.mem[0x0010], expected_byte,
                    "{msg}: zero-page byte should be {:08b}",
                    expected_byte
                );
                assert_eq!(cpu.a, 0xAA, "{msg}: SMB/RMB must not modify A");
                assert_eq!(cpu.x, 0xBB, "{msg}: SMB/RMB must not modify X");
                assert_eq!(cpu.y, 0xCC, "{msg}: SMB/RMB must not modify Y");
                assert_eq!(cpu.s, 0xF0, "{msg}: SMB/RMB must not modify S");
                assert_eq!(
                    cpu.p, initial_p,
                    "{msg}: SMB/RMB must not modify any status flags"
                );
            };

        bus.mem[0x0010] = 0b0001_0100;

        run_instruction(&mut cpu, &mut bus); // SMB0
        assert_state(&cpu, &bus, 0b0001_0101, "After SMB0 (set bit 0)");

        run_instruction(&mut cpu, &mut bus); // SMB4
        assert_state(&cpu, &bus, 0b0001_0101, "After SMB4 (set bit 4)");

        run_instruction(&mut cpu, &mut bus); // RMB2
        assert_state(&cpu, &bus, 0b0001_0001, "After RMB2 (clear bit 2)");

        run_instruction(&mut cpu, &mut bus); // RMB4
        assert_state(&cpu, &bus, 0b0000_0001, "After RMB4 (clear bit 4)");
    }

    #[test]
    fn trb_tsb() {
        // TSB/TRB are 65C02-only read-modify-write instructions that:
        //  - Compute Z from A & M (before modification):
        //      * Z = 1 if (A & M) == 0
        //      * Z = 0 otherwise
        //  - Modify memory:
        //      * TSB: M <- M | A  (set bits)
        //      * TRB: M <- M & !A (reset bits)
        //  - Do NOT modify A or any other flags (C/N/V, etc.) besides Z.
        //
        // We exercise a few cases to nail this down.
        //
        // --- Case 1: TSB zp with non-zero A&M ---
        let (mut cpu, mut bus) = setup_cmos(0x8000, &[0x04, 0x10, 0xEA]); // TSB $10; NOP
        bus.mem[0x0010] = 0b0001_0100;
        cpu.a = 0b0000_0101;
        cpu.p = U_6502 | C | N | V | Z; // Z initially set so we can see it clear

        run_instruction(&mut cpu, &mut bus);
        assert_eq!(
            bus.mem[0x0010], 0b0001_0101,
            "TSB must set bits in memory according to A"
        );
        assert_eq!(cpu.a, 0b0000_0101, "TSB must not modify A");
        assert_eq!(cpu.p & Z, 0, "TSB with non-zero A&M must clear Z");
        assert_eq!(
            cpu.p & (C | N | V),
            C | N | V,
            "TSB must not modify C/N/V flags"
        );

        // --- Case 2: TRB zp with overlapping bits (non-zero A&M, bits cleared) ---
        let (mut cpu, mut bus) = setup_cmos(0x9000, &[0x14, 0x10, 0xEA]); // TRB $10; NOP
        bus.mem[0x0010] = 0b0001_0100;
        cpu.a = 0b0001_0000;
        cpu.p = U_6502 | C | N | V | Z; // Z initially set

        run_instruction(&mut cpu, &mut bus);
        assert_eq!(
            bus.mem[0x0010], 0b0000_0100,
            "TRB must clear bits in memory where A has 1s"
        );
        assert_eq!(cpu.a, 0b0001_0000, "TRB must not modify A");
        assert_eq!(cpu.p & Z, 0, "TRB with non-zero A&M must clear Z");
        assert_eq!(
            cpu.p & (C | N | V),
            C | N | V,
            "TRB must not modify C/N/V flags"
        );

        // --- Case 3: TRB zp with no overlapping bits (A&M == 0) ---
        let (mut cpu, mut bus) = setup_cmos(0xA000, &[0x14, 0x10, 0xEA]); // TRB $10; NOP
        bus.mem[0x0010] = 0b0001_0100;
        cpu.a = 0b0000_0011;
        cpu.p = U_6502 | C | N | V; // Z initially clear

        run_instruction(&mut cpu, &mut bus);
        assert_eq!(
            bus.mem[0x0010], 0b0001_0100,
            "TRB must leave memory unchanged when A&M == 0"
        );
        assert_eq!(cpu.a, 0b0000_0011, "TRB must not modify A");
        assert_eq!(cpu.p & Z, Z, "TRB with A&M == 0 must set Z");
        assert_eq!(
            cpu.p & (C | N | V),
            C | N | V,
            "TRB must preserve C/N/V flags when A&M == 0"
        );
    }
}
