use rs65x::bus::{Bus, Lines, WaitStates};
use serde::Deserialize;

fn de_read_write<'de, D>(deserializer: D) -> Result<AccessType, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    match s.as_str() {
        "read" => Ok(AccessType::Read),
        "write" => Ok(AccessType::Write),
        other => Err(serde::de::Error::custom(format!(
            "invalid read/write string: {other}"
        ))),
    }
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
pub struct Access {
    addr: u16,
    data: u8,
    #[serde(deserialize_with = "de_read_write")]
    access_type: AccessType,
}

pub struct Harness {
    pub mem: [u8; 0x10000],
    pub last: Access,
    pub lines: Lines,
    pub cycle: u64,
}

impl Default for Harness {
    fn default() -> Self {
        Self {
            mem: [0; 0x10000],
            last: Access::basic_read(0, 0),
            lines: Default::default(),
            cycle: Default::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub enum AccessType {
    Read,
    Write,
}

impl Harness {
    pub fn last_cycle(&self) -> Access {
        Access {
            addr: self.last.addr,
            data: self.last.data,
            access_type: self.last.access_type,
        }
    }
}

impl Bus for Harness {
    fn read(&mut self, addr: u32, _vda: bool, _vpa: bool) -> (u8, WaitStates) {
        let access_type = AccessType::Read;
        let addr16 = addr as u16;
        let data = self.mem[addr16 as usize];
        //println!("read access at {addr:#04X} | data = {data:#04X}");
        self.last = Access {
            addr: addr16,
            data,
            access_type,
        };
        self.cycle += 1;
        (data, 0)
    }

    fn write(&mut self, addr: u32, data: u8, _vda: bool, _vpa: bool) -> WaitStates {
        //println!("write access at {addr:#04X} | data = {data:#04X}");
        let access_type = AccessType::Write;
        let addr16 = addr as u16;
        self.mem[addr16 as usize] = data;
        self.last = Access {
            addr: addr16,
            data,
            access_type,
        };
        self.cycle += 1;
        0
    }

    fn sample_lines(&mut self) -> Lines {
        self.lines
    }
}

impl Access {
    pub fn basic_read(addr: u16, data: u8) -> Self {
        Access {
            addr,
            data,
            access_type: AccessType::Read,
        }
    }

    pub fn basic_write(addr: u16, data: u8) -> Self {
        Access {
            addr,
            data,
            access_type: AccessType::Write,
        }
    }
}

pub struct InstructionTrace {
    pub cycles: u64,
    pub accesses: Vec<Access>,
}

impl InstructionTrace {
    pub fn assert_accesses(&self, expected: Vec<Access>) {
        let mut cycle = 0;
        let expected_cycles = expected.len() as u64;
        for e in expected {
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
}

#[derive(Deserialize)]
pub struct State {
    pub pc: u16,
    pub s: u8,
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub p: u8,
    pub ram: Vec<(u16, u8)>,
}

#[derive(Deserialize)]
pub struct SingleStepCase {
    pub name: String,
    pub initial: State,
    pub r#final: State,
    pub cycles: Vec<Access>,
}
