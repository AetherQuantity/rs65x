use core::fmt::{Display, Error, Formatter};

use crate::isa::{AddressMode, MemoryAction, NoMemType, address_mode_subtypes};

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub struct OpcodeError {
    pub opcode: u8,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Instruction {
    pub mnemonic: Mnemonic,
    pub address_mode: AddressMode,
    pub memory_action: MemoryAction,
}

const fn action_for_mnemonic(mnemonic: Mnemonic) -> MemoryAction {
    use Mnemonic::*;
    match mnemonic {
        Lda | Ldx | Ldy | And | Eor | Ora | Adc | Sbc | Cmp | Cpx | Cpy | Bit | Pla | Plp | Plx
        | Ply => MemoryAction::Read,
        Sta | Stx | Sty | Stz | Pha | Php | Phx | Phy => MemoryAction::Write,
        Asl | Lsr | Rol | Ror | Inc | Dec | Tsb | Trb => MemoryAction::ReadModifyWrite,
        // R65C02 instructions
        Bbr0 | Bbr1 | Bbr2 | Bbr3 | Bbr4 | Bbr5 | Bbr6 | Bbr7 | Bbs0 | Bbs1 | Bbs2 | Bbs3
        | Bbs4 | Bbs5 | Bbs6 | Bbs7 => MemoryAction::Read,
        Rmb0 | Rmb1 | Rmb2 | Rmb3 | Rmb4 | Rmb5 | Rmb6 | Rmb7 | Smb0 | Smb1 | Smb2 | Smb3
        | Smb4 | Smb5 | Smb6 | Smb7 => MemoryAction::ReadModifyWrite,
        // all the rest should default to Read i guess:
        _ => MemoryAction::Read,
    }
}

const fn make_empty_table() -> [Instruction; 256] {
    let undefined = Instruction {
        mnemonic: Mnemonic::Undefined,
        address_mode: AddressMode::NoMemory(NoMemType::Implied),
        memory_action: MemoryAction::Read,
    };
    [undefined; 256]
}

macro_rules! define_opcodes {
    (@emit_entry $table_var:ident, $table_ident:ident, $mnemonic:ident,
        $mode:ident $( ( $variant:ident ) )?, $code:expr
    ) => {{
        let memory_action = action_for_mnemonic(Mnemonic::$mnemonic);
        $table_var[$code as usize] = Instruction {
            mnemonic: Mnemonic::$mnemonic,
            address_mode: AddressMode::$mode $( ( $variant ) )?,
            memory_action,
        };
    }};
    (@emit_entry $table_var:ident, $table_ident:ident, $mnemonic:ident,
        $mode:ident $( ( $variant:ident ) )?, $code:expr; [$($filter:ident),+ $(,)?]
    ) => {{
        let include = false $(|| matches!(OpcodeTable::$table_ident, OpcodeTable::$filter))+;
        if include {
            define_opcodes!(@emit_entry $table_var, $table_ident, $mnemonic, $mode $( ( $variant ) )?, $code);
        }
    }};
    (@fill_table $table_var:ident, $table_ident:ident;) => {};
    (@fill_table $table_var:ident, $table_ident:ident;
        $(#[$doc:meta])*
        $mnemonic:ident {
            $(
                $mode:ident $( ( $variant:ident ) )?
                $( @ [ $($entry_table:ident),+ ] )?
                = $code:expr,
            )+
        },
        $($rest:tt)*
    ) => {
        $(
            define_opcodes!(
                @emit_entry $table_var, $table_ident, $mnemonic, $mode $( ( $variant ) )?, $code
                $( ; [$($entry_table),+] )?
            );
        )+
        define_opcodes!(@fill_table $table_var, $table_ident; $($rest)*);
    };
    (@fill_table $table_var:ident, $table_ident:ident;) => {};
    (@build_tables [] => {$($definitions:tt)*}) => {};
    (@build_tables [$table:ident $(, $rest:ident)*] => {$($definitions:tt)*}) => {
        #[allow(non_upper_case_globals)]
        pub static $table: [Instruction; 256] = {
            #[allow(unused_imports)]
            use address_mode_subtypes::{NoMemType::*, OffsetType::*, JumpType::*, BranchType::*};
            let mut table_data: [Instruction; 256] = make_empty_table();
            define_opcodes!(@fill_table table_data, $table; $($definitions)*);
            table_data
        };
        define_opcodes!(@build_tables [$($rest),*] => {$($definitions)*});
    };
    (
        tables: [$($table:ident),+ $(,)?];

        $(
            $(#[$doc:meta])*
            $mnemonic:ident {
                $(
                    $mode:ident $( ( $variant:ident ) )?
                    $( @ [ $($entry_table:ident),+ $(,)? ] )?
                    = $code:expr,
                )+
            },
        )+
    ) => {
        #[derive(Debug, PartialEq, Eq, Clone, Copy)]
        pub enum Mnemonic {
            Undefined,
            $(
                $(#[$doc])*
                $mnemonic,
            )+
        }

        #[derive(Debug, PartialEq, Eq, Clone, Copy)]
        pub enum OpcodeTable {
            $($table),+
        }

        pub mod opcode_tables {
            use super::*;

            define_opcodes!(
                @build_tables [$($table),+] => {
                    $(
                        $(#[$doc])*
                        $mnemonic {
                            $(
                                $mode $( ( $variant ) )?
                                $( @ [ $($entry_table),+ ] )?
                                = $code,
                            )+
                        },
                    )+
                }
            );
        }

        impl OpcodeTable {
            const fn data(self) -> &'static [Instruction; 256] {
                match self {
                    $(OpcodeTable::$table => &opcode_tables::$table),+
                }
            }

            #[inline(always)]
            pub fn decode(self, op: u8) -> Instruction {
                // SAFETY: each opcode table contains entries for all 256 opcodes
                unsafe { *self.data().get_unchecked(op as usize) }
            }
        }

        impl Instruction {
            pub fn from_byte(op: u8, table: OpcodeTable) -> Instruction {
                table.decode(op)
            }
        }

        impl Display for Mnemonic {
            fn fmt(&self, f: &mut Formatter) -> Result<(), Error> {
                match *self {
                    $(
                        Mnemonic::$mnemonic => write!(f, "{}", stringify!($mnemonic)),
                    )+
                    Mnemonic::Undefined => write!(f, "Und"),
                }
            }
        }
    };
}

// this macro also creates the pub enum Mnemonic
define_opcodes! {
    tables: [Nmos, Cmos, Wdc816];

    // 6502 Opcodes ///////////////////////////////////////////////////////////////////////////////

    /// # Load Accumulator
    /// Loads a byte of memory into the accumulator.
    ///
    /// Memory access type: Read
    ///
    /// A,Z,N = M
    Lda {
        NoMemory(Immediate) = 0xA9,
        DirectPage(None)    = 0xA5,
        DirectPage(X)       = 0xB5,
        Absolute(None)      = 0xAD,
        Absolute(X)         = 0xBD,
        Absolute(Y)         = 0xB9,
        DpIndirect(X)       = 0xA1,
        DpIndirect(Y)       = 0xB1,
        DpIndirect(None) @[Cmos, Wdc816] = 0xB2,
        StackRelative(None)  @[Wdc816]   = 0xA3,
        StackRelative(Y)     @[Wdc816]   = 0xB3,
        DpIndirectLong(None) @[Wdc816]   = 0xA7,
        DpIndirectLong(Y)    @[Wdc816]   = 0xB7,
        AbsoluteLong(None)   @[Wdc816]   = 0xAF,
        AbsoluteLong(X)      @[Wdc816]   = 0xBF,
    },
    /// # Load X Register
    /// Loads a byte of memory into the X register.
    ///
    /// Memory access type: Read
    ///
    /// X,Z,N = M
    Ldx {
        NoMemory(Immediate) = 0xA2,
        DirectPage(None)    = 0xA6,
        DirectPage(Y)       = 0xB6,
        Absolute(None)      = 0xAE,
        Absolute(Y)         = 0xBE,
    },
    /// # Load Y Register
    /// Loads a byte of memory into the Y register.
    ///
    /// Memory access type: Read
    ///
    /// Y,Z,N = M
    Ldy {
        NoMemory(Immediate) = 0xA0,
        DirectPage(None)    = 0xA4,
        DirectPage(X)       = 0xB4,
        Absolute(None)      = 0xAC,
        Absolute(X)         = 0xBC,
    },

    /// # Store Accumulator
    /// Stores the accumulator in memory.
    ///
    /// Memory access type: Write
    ///
    /// M = A
    Sta {
        DirectPage(None) = 0x85,
        DirectPage(X)    = 0x95,
        Absolute(None)   = 0x8D,
        Absolute(X)      = 0x9D,
        Absolute(Y)      = 0x99,
        DpIndirect(X)    = 0x81,
        DpIndirect(Y)    = 0x91,
        DpIndirect(None) @[Cmos, Wdc816] = 0x92,
        StackRelative(None)  @[Wdc816]   = 0x83,
        StackRelative(Y)     @[Wdc816]   = 0x93,
        DpIndirectLong(None) @[Wdc816]   = 0x87,
        DpIndirectLong(Y)    @[Wdc816]   = 0x97,
        AbsoluteLong(None)   @[Wdc816]   = 0x8F,
        AbsoluteLong(X)      @[Wdc816]   = 0x9F,
    },
    /// # Store X Register
    /// Stores the X register in memory.
    ///
    /// Memory access type: Write
    ///
    /// M = X
    Stx {
        DirectPage(None) = 0x86,
        DirectPage(Y)    = 0x96,
        Absolute(None)   = 0x8E,
    },
    /// # Store Y Register
    /// Stores the Y register in memory.
    ///
    /// Memory access type: Write
    ///
    /// M = Y
    Sty {
        DirectPage(None) = 0x84,
        DirectPage(X)    = 0x94,
        Absolute(None)   = 0x8C,
    },

    /// # Transfer A to X
    /// Transfers the accumulator to the X register.
    ///
    /// Memory access type: None
    ///
    /// X = A
    Tax { NoMemory(Implied) = 0xAA, },
    /// # Transfer A to Y
    /// Transfers the accumulator to the Y register.
    ///
    /// Memory access type: None
    ///
    /// Y = A
    Tay { NoMemory(Implied) = 0xA8, },
    /// # Transfer X to A
    /// Transfers the X register to the accumulator.
    ///
    /// Memory access type: None
    ///
    /// A = X
    Txa { NoMemory(Implied) = 0x8A, },
    /// # Transfer Y to A
    /// Transfers the Y register to the accumulator.
    ///
    /// Memory access type: None
    ///
    /// A = Y
    Tya { NoMemory(Implied) = 0x98, },
    /// # Transfer SP to X
    /// Transfers the stack pointer to the X register.
    ///
    /// Memory access type: None
    ///
    /// X = S
    Tsx { NoMemory(Implied) = 0xBA, },
    /// # Transfer X to SP
    /// Transfers the X register to the stack pointer.
    ///
    /// Memory access type: None
    ///
    /// S = X
    Txs { NoMemory(Implied) = 0x9A, },

    /// # Push Accumulator
    /// Pushes the accumulator onto the stack.
    ///
    /// Memory access type: Stack
    Pha { Stack = 0x48, },
    /// # Push Processor Status
    /// Pushes the status register onto the stack.
    ///
    /// Memory access type: Stack
    Php { Stack = 0x08, },
    /// # Pull Accumulator
    /// Pulls the accumulator from the stack.
    ///
    /// Memory access type: Stack
    Pla { Stack = 0x68, },
    /// # Pull Processor Status
    /// Pulls the status register from the stack.
    ///
    /// Memory access type: Stack
    Plp { Stack = 0x28, },

    /// # Logical AND
    /// Logical ANDs the accumulator with a byte of memory.
    ///
    /// Memory access type: Read
    ///
    /// A,Z,N = A&M
    And {
        NoMemory(Immediate) = 0x29,
        DirectPage(None)    = 0x25,
        DirectPage(X)       = 0x35,
        Absolute(None)      = 0x2D,
        Absolute(X)         = 0x3D,
        Absolute(Y)         = 0x39,
        DpIndirect(X)       = 0x21,
        DpIndirect(Y)       = 0x31,
        DpIndirect(None) @[Cmos, Wdc816] = 0x32,
        StackRelative(None)  @[Wdc816]   = 0x23,
        StackRelative(Y)     @[Wdc816]   = 0x33,
        DpIndirectLong(None) @[Wdc816]   = 0x27,
        DpIndirectLong(Y)    @[Wdc816]   = 0x37,
        AbsoluteLong(None)   @[Wdc816]   = 0x2F,
        AbsoluteLong(X)      @[Wdc816]   = 0x3F,
    },
    /// # Logical EOR (XOR)
    /// Exclusive ORs the accumulator with a byte of memory.
    ///
    /// Memory access type: Read
    ///
    /// A,Z,N = A^M
    Eor {
        NoMemory(Immediate) = 0x49,
        DirectPage(None)    = 0x45,
        DirectPage(X)       = 0x55,
        Absolute(None)      = 0x4D,
        Absolute(X)         = 0x5D,
        Absolute(Y)         = 0x59,
        DpIndirect(X)       = 0x41,
        DpIndirect(Y)       = 0x51,
        DpIndirect(None) @[Cmos, Wdc816] = 0x52,
        StackRelative(None)  @[Wdc816]   = 0x43,
        StackRelative(Y)     @[Wdc816]   = 0x53,
        DpIndirectLong(None) @[Wdc816]   = 0x47,
        DpIndirectLong(Y)    @[Wdc816]   = 0x57,
        AbsoluteLong(None)   @[Wdc816]   = 0x4F,
        AbsoluteLong(X)      @[Wdc816]   = 0x5F,
    },
    /// # Logical OR
    /// Logical ORs the accumulator with a byte of memory.
    ///
    /// Memory access type: Read
    ///
    /// A,Z,N = A|M
    Ora {
        NoMemory(Immediate) = 0x09,
        DirectPage(None)    = 0x05,
        DirectPage(X)       = 0x15,
        Absolute(None)      = 0x0D,
        Absolute(X)         = 0x1D,
        Absolute(Y)         = 0x19,
        DpIndirect(X)       = 0x01,
        DpIndirect(Y)       = 0x11,
        DpIndirect(None) @[Cmos, Wdc816] = 0x12,
        StackRelative(None)  @[Wdc816]   = 0x03,
        StackRelative(Y)     @[Wdc816]   = 0x13,
        DpIndirectLong(None) @[Wdc816]   = 0x07,
        DpIndirectLong(Y)    @[Wdc816]   = 0x17,
        AbsoluteLong(None)   @[Wdc816]   = 0x0F,
        AbsoluteLong(X)      @[Wdc816]   = 0x1F,
    },

    /// # Bit Test
    /// Compares the accumulator with a byte of memory. Mask pattern in accumulator is ANDed with
    /// the value in memory to set or clear the zero flag, but the result is not kept. Bits 6 and 7
    /// of the memory value are copied into the N and V flags.
    ///
    /// Memory access type: Read
    ///
    /// Z=(A==M), N=M7, V=M6
    Bit {
        DirectPage(None) = 0x24,
        Absolute(None)   = 0x2C,
        NoMemory(Immediate) @[Cmos, Wdc816] = 0x89,
        DirectPage(X)       @[Cmos, Wdc816] = 0x34,
        Absolute(X)         @[Cmos, Wdc816] = 0x3C,
    },

    /// # Add with Carry
    /// Adds a byte of memory to the accumulator using the carry flag. The carry flag is necessarily added.
    ///
    /// Memory access type: Read
    ///
    /// A,Z,C,N = A+M+C
    Adc {
        NoMemory(Immediate) = 0x69,
        DirectPage(None)    = 0x65,
        DirectPage(X)       = 0x75,
        Absolute(None)      = 0x6D,
        Absolute(X)         = 0x7D,
        Absolute(Y)         = 0x79,
        DpIndirect(X)       = 0x61,
        DpIndirect(Y)       = 0x71,
        DpIndirect(None) @[Cmos, Wdc816] = 0x72,
        StackRelative(None)  @[Wdc816]   = 0x63,
        StackRelative(Y)     @[Wdc816]   = 0x73,
        DpIndirectLong(None) @[Wdc816]   = 0x67,
        DpIndirectLong(Y)    @[Wdc816]   = 0x77,
        AbsoluteLong(None)   @[Wdc816]   = 0x6F,
        AbsoluteLong(X)      @[Wdc816]   = 0x7F,
    },
    /// # Subtract with Carry
    /// Subtracts a byte of memory from the accumulator using the carry flag. The carry flag is treated as a borrow,
    /// which is the opposite of carry, so the carry flag is cleared if a borrow is required.
    ///
    /// Memory access type: Read
    ///
    /// A,Z,C,N = A-M-(1-C)
    Sbc {
        NoMemory(Immediate) = 0xE9,
        DirectPage(None)    = 0xE5,
        DirectPage(X)       = 0xF5,
        Absolute(None)      = 0xED,
        Absolute(X)         = 0xFD,
        Absolute(Y)         = 0xF9,
        DpIndirect(X)       = 0xE1,
        DpIndirect(Y)       = 0xF1,
        DpIndirect(None) @[Cmos, Wdc816] = 0xF2,
        StackRelative(None)  @[Wdc816]   = 0xE3,
        StackRelative(Y)     @[Wdc816]   = 0xF3,
        DpIndirectLong(None) @[Wdc816]   = 0xE7,
        DpIndirectLong(Y)    @[Wdc816]   = 0xF7,
        AbsoluteLong(None)   @[Wdc816]   = 0xEF,
        AbsoluteLong(X)      @[Wdc816]   = 0xFF,
    },
    /// # Compare Accumulator
    /// Compares the accumulator with a byte of memory. If the accumulator is greater than or equal to the memory value,
    /// the carry flag is set. The zero flag is set if the accumulator is equal to the memory value.
    /// The negative flag is set if the most significant bit of the result is set.
    ///
    /// Memory access type: Read
    ///
    /// C,Z,N = A-M
    Cmp {
        NoMemory(Immediate) = 0xC9,
        DirectPage(None)    = 0xC5,
        DirectPage(X)       = 0xD5,
        Absolute(None)      = 0xCD,
        Absolute(X)         = 0xDD,
        Absolute(Y)         = 0xD9,
        DpIndirect(X)       = 0xC1,
        DpIndirect(Y)       = 0xD1,
        DpIndirect(None) @[Cmos, Wdc816] = 0xD2,
        StackRelative(None)  @[Wdc816]   = 0xC3,
        StackRelative(Y)     @[Wdc816]   = 0xD3,
        DpIndirectLong(None) @[Wdc816]   = 0xC7,
        DpIndirectLong(X)    @[Wdc816]   = 0xD7,
        AbsoluteLong(None)   @[Wdc816]   = 0xCF,
        AbsoluteLong(X)      @[Wdc816]   = 0xDF,
    },
    /// # Compare X Register
    /// Compares the X register with a byte of memory. If the X register is greater than or equal to the memory value,
    /// the carry flag is set. The zero flag is set if the X register is equal to the memory value.
    /// The negative flag is set if the most significant bit of the result is set.
    ///
    /// Memory access type: Read
    ///
    /// C,Z,N = X-M
    Cpx {
        NoMemory(Immediate) = 0xE0,
        DirectPage(None)    = 0xE4,
        Absolute(None)      = 0xEC,
    },
    /// # Compare Y Register
    /// Compares the Y register with a byte of memory. If the Y register is greater than or equal to the memory value,
    /// the carry flag is set. The zero flag is set if the Y register is equal to the memory value.
    /// The negative flag is set if the most significant bit of the result is set.
    ///
    /// Memory access type: Read
    ///
    /// C,Z,N = Y-M
    Cpy {
        NoMemory(Immediate) = 0xC0,
        DirectPage(None)    = 0xC4,
        Absolute(None)      = 0xCC,
    },

    /// # Increment Memory (or A)
    /// Increments a byte of memory by one. On the 65C02 and later, the accumulator can be incremented as well.
    ///
    /// Memory access type: Read-Modify-Write
    ///
    /// M,Z,N = M+1
    Inc {
        DirectPage(None) = 0xE6,
        DirectPage(X)    = 0xF6,
        Absolute(None)   = 0xEE,
        Absolute(X)      = 0xFE,
        NoMemory(Implied) @[Cmos, Wdc816] = 0x1A,
    },
    /// # Increment X Register
    /// Increments the X register by one.
    ///
    /// Memory access type: None
    ///
    /// X,Z,N = X+1
    Inx { NoMemory(Implied) = 0xE8, },
    /// # Increment Y Register
    /// Increments the Y register by one.
    ///
    /// Memory access type: None
    ///
    /// Y,Z,N = Y+1
    Iny { NoMemory(Implied) = 0xC8, },

    /// # Decrement Memory (or A)
    /// Decrements a byte of memory by one. On the 65C02 and later, the accumulator can be decremented as well.
    ///
    /// Memory access type: Read-Modify-Write
    ///
    /// M,Z,N = M-1
    Dec {
        DirectPage(None) = 0xC6,
        DirectPage(X)    = 0xD6,
        Absolute(None)   = 0xCE,
        Absolute(X)      = 0xDE,
        NoMemory(Implied) @[Cmos, Wdc816] = 0x3A,
    },
    /// # Decrement X Register
    /// Decrements the X register by one.
    ///
    /// Memory access type: None
    ///
    /// X,Z,N = X-1
    Dex { NoMemory(Implied) = 0xCA, },
    /// # Decrement Y Register
    /// Decrements the Y register by one.
    ///
    /// Memory access type: None
    ///
    /// Y,Z,N = Y-1
    Dey { NoMemory(Implied) = 0x88, },

    /// # Arithmetic Shift Left
    /// Shifts all bits in a byte of memory or the accumulator one bit to the left. The most significant bit
    /// is shifted into the carry flag, the least significant bit is set to zero.
    ///
    /// Memory access type: Read-Modify-Write
    ///
    /// M,Z,C,N = M*2 or A,Z,C,N = A*2
    Asl {
        NoMemory(Implied) = 0x0A,
        DirectPage(None)  = 0x06,
        DirectPage(X)     = 0x16,
        Absolute(None)    = 0x0E,
        Absolute(X)       = 0x1E,
    },
    /// # Logical Shift Right
    /// Shifts all bits in a byte of memory or the accumulator one bit to the right. The least significant bit
    /// is shifted into the carry flag, the most significant bit is set to zero.
    ///
    /// Memory access type: Read-Modify-Write
    ///
    /// M,Z,C,N = M/2 or A,Z,C,N = A/2
    Lsr {
        NoMemory(Implied) = 0x4A,
        DirectPage(None)  = 0x46,
        DirectPage(X)     = 0x56,
        Absolute(None)    = 0x4E,
        Absolute(X)       = 0x5E,
    },
    /// # Rotate Left
    /// Rotates all bits in a byte of memory or the accumulator one bit to the left through the carry flag.
    /// The most significant bit is shifted into the carry flag, and the carry flag is shifted into the least
    /// significant bit.
    ///
    /// Memory access type: Read-Modify-Write
    ///
    /// M,Z,C,N = M*2 or A,Z,C,N = A*2
    Rol {
        NoMemory(Implied) = 0x2A,
        DirectPage(None)  = 0x26,
        DirectPage(X)     = 0x36,
        Absolute(None)    = 0x2E,
        Absolute(X)       = 0x3E,
    },
    /// # Rotate Right
    /// Rotates all bits in a byte of memory or the accumulator one bit to the right through the carry flag.
    /// The least significant bit is shifted into the carry flag, and the carry flag is shifted into the most
    /// significant bit.
    ///
    /// Memory access type: Read-Modify-Write
    ///
    /// M,Z,C,N = M/2 or A,Z,C,N = A/2
    Ror {
        NoMemory(Implied) = 0x6A,
        DirectPage(None)  = 0x66,
        DirectPage(X)     = 0x76,
        Absolute(None)    = 0x6E,
        Absolute(X)       = 0x7E,
    },

    /// # Jump
    /// Sets the program counter to the address specified by the operand (or the operand's address).
    ///
    /// On the 6502, in indirect addressing mode, if the low byte of the address is nnFF, the high byte
    /// is taken from nn00. This is a bug in the 6502, and is fixed in the 65C02. This is known as the
    /// "page boundary bug".
    ///
    /// Memory access type: None
    Jmp {
        Jump(JmpAbsolute) = 0x4C,
        Jump(JmpIndirect) = 0x6C,
        Jump(JmpIndirectX) @[Cmos, Wdc816] = 0x7C,
    },
    /// # Jump to Subroutine
    /// Pushes the address of the next instruction onto the stack, then sets the program counter to the
    /// address specified by the operand (or the operand's address).
    ///
    /// Memory access type: None
    Jsr { Jump(ToSubroutine) = 0x20, },
    /// # Return from Subroutine
    /// Pulls the address of the next instruction from the stack and sets the program counter to that address.
    ///
    /// Memory access type: None
    Rts { Jump(FromSubroutine) = 0x60, },

    /// # Branch on Carry Clear
    /// Branches to the address specified by the operand if the carry flag is clear.
    ///
    /// Memory access type: None
    Bcc { Branch(Relative) = 0x90, },
    /// # Branch on Carry Set
    /// Branches to the address specified by the operand if the carry flag is set.
    ///
    /// Memory access type: None
    Bcs { Branch(Relative) = 0xB0, },
    /// # Branch on Equal
    /// Branches to the address specified by the operand if the zero flag is set.
    ///
    /// Memory access type: None
    Beq { Branch(Relative) = 0xF0, },
    /// # Branch on Minus
    /// Branches to the address specified by the operand if the negative flag is set.
    ///
    /// Memory access type: None
    Bmi { Branch(Relative) = 0x30, },
    /// # Branch on Not Equal
    /// Branches to the address specified by the operand if the zero flag is clear.
    ///
    /// Memory access type: None
    Bne { Branch(Relative) = 0xD0, },
    /// # Branch on Positive (Plus)
    /// Branches to the address specified by the operand if the negative flag is clear.
    ///
    /// Memory access type: None
    Bpl { Branch(Relative) = 0x10, },
    /// # Branch on Overflow Clear
    /// Branches to the address specified by the operand if the overflow flag is clear.
    ///
    /// Memory access type: None
    Bvc { Branch(Relative) = 0x50, },
    /// # Branch on Overflow Set
    /// Branches to the address specified by the operand if the overflow flag is set.
    ///
    /// Memory access type: None
    Bvs { Branch(Relative) = 0x70, },

    /// # Clear Carry Flag
    /// Clears the carry flag.
    ///
    /// Memory access type: None
    Clc { NoMemory(Implied) = 0x18, },
    /// # Clear Decimal Mode
    /// Clears the decimal mode flag.
    ///
    /// Memory access type: None
    Cld { NoMemory(Implied) = 0xD8, },
    /// # Clear Interrupt Disable
    /// Clears the interrupt disable flag.
    ///
    /// Memory access type: None
    Cli { NoMemory(Implied) = 0x58, },
    /// # Clear Overflow Flag
    /// Clears the overflow flag.
    ///
    /// Memory access type: None
    Clv { NoMemory(Implied) = 0xB8, },
    /// # Set Carry Flag
    /// Sets the carry flag.
    ///
    /// Memory access type: None
    Sec { NoMemory(Implied) = 0x38, },
    /// # Set Decimal Mode
    /// Sets the decimal mode flag.
    ///
    /// Memory access type: None
    Sed { NoMemory(Implied) = 0xF8, },
    /// # Set Interrupt Disable
    /// Sets the interrupt disable flag.
    ///
    /// Memory access type: None
    Sei { NoMemory(Implied) = 0x78, },

    /// # Break (Force Interrupt)
    /// Forces an interrupt by pushing the program counter and status register onto the stack, then
    /// setting the program counter to the address stored at $FFFE-$FFFF.
    ///
    /// B flag is set to 1 before pushing the status register onto the stack.
    ///
    /// Memory access type: None
    Brk { Jump(ToInterrupt) = 0x00, },

    /// # Return from Interrupt
    /// Pulls the status register and program counter from the stack.
    ///
    /// Memory access type: None
    Rti { Jump(FromInterrupt) = 0x40, },

    /// # No Operation
    /// Does nothing.
    ///
    /// Memory access type: None
    Nop {
        NoMemory(Implied) = 0xEA, // the real one

        // Various "illegal" opcodes on NMOS that do the memory access but then do nothing
        NoMemory(Implied) @[Nmos] = 0x1A,
        NoMemory(Implied) @[Nmos] = 0x3A,
        NoMemory(Implied) @[Nmos] = 0x5A,
        NoMemory(Implied) @[Nmos] = 0x7A,
        NoMemory(Implied) @[Nmos] = 0xDA,
        NoMemory(Implied) @[Nmos] = 0xFA,
        NoMemory(Immediate) @[Nmos] = 0x80,
        NoMemory(Immediate) @[Nmos] = 0x82,
        NoMemory(Immediate) @[Nmos] = 0x89,
        NoMemory(Immediate) @[Nmos] = 0xC2,
        NoMemory(Immediate) @[Nmos] = 0xE2,
        DirectPage(None) @[Nmos] = 0x04,
        DirectPage(None) @[Nmos] = 0x44,
        DirectPage(None) @[Nmos] = 0x64,
        DirectPage(X) @[Nmos] = 0x14,
        DirectPage(X) @[Nmos] = 0x34,
        DirectPage(X) @[Nmos] = 0x54,
        DirectPage(X) @[Nmos] = 0x74,
        DirectPage(X) @[Nmos] = 0xD4,
        DirectPage(X) @[Nmos] = 0xF4,
        Absolute(None) @[Nmos] = 0x0C,
        Absolute(X) @[Nmos] = 0x1C,
        Absolute(X) @[Nmos] = 0x3C,
        Absolute(X) @[Nmos] = 0x5C,
        Absolute(X) @[Nmos] = 0x7C,
        Absolute(X) @[Nmos] = 0xDC,
        Absolute(X) @[Nmos] = 0xFC,
    },

    // Illegal NMOS Opcodes ///////////////////////////////////////////////////////////////////////

    /// # ALR (or ASR): AND + LSR
    /// Illegal.
    Alr { NoMemory(Immediate) @[Nmos] = 0x4B, },

    /// # ANC: AND + set C flag
    /// Illegal.
    Anc {
        NoMemory(Immediate) @[Nmos] = 0x0B,
        NoMemory(Immediate) @[Nmos] = 0x2B,
    },

    /// # ANE (or XAA): * OR X + AND
    /// Illegal. Highly unstable.
    ///
    /// A base value in A is determined based on the contets of A and a constant, which may be typically
    /// $00, $ff, $ee, etc. The value of this constant depends on temerature, the chip series, and maybe
    /// other factors, as well. In order to eliminate these uncertaincies from the equation, use either
    /// 0 as the operand or a value of $FF in the accumulator.
    Ane { NoMemory(Immediate) @[Nmos] = 0x8B, },

    /// # ARR: AND + ROR
    /// Illegal. This operation involves the adder:
    /// - V-flag is set according to (A AND oper) + oper
    /// - The carry is not set, but bit 7 (sign) is exchanged with the carry
    Arr { NoMemory(Immediate) @[Nmos] = 0x6B, },

    /// # DCP (DCM): DEC + CMP
    /// Illegal. Decrements the operand and then compares the result to the accumulator.
    Dcp {
        DirectPage(None) @[Nmos] = 0xC7,
        DirectPage(X)    @[Nmos] = 0xD7,
        Absolute(None)   @[Nmos] = 0xCF,
        Absolute(X)      @[Nmos] = 0xDF,
        Absolute(Y)      @[Nmos] = 0xDB,
        DpIndirect(X)    @[Nmos] = 0xC3,
        DpIndirect(Y)    @[Nmos] = 0xD3,
    },

    /// # ISC (ISB, INS): INC + SBC
    /// Illegal.
    Isc {
        DirectPage(None) @[Nmos] = 0xE7,
        DirectPage(X)    @[Nmos] = 0xF7,
        Absolute(None)   @[Nmos] = 0xEF,
        Absolute(X)      @[Nmos] = 0xFF,
        Absolute(Y)      @[Nmos] = 0xFB,
        DpIndirect(X)    @[Nmos] = 0xE3,
        DpIndirect(Y)    @[Nmos] = 0xF3,
    },

    /// # LAS (LDR): LDA + TSX
    /// Illegal.
    Las { Absolute(Y) @[Nmos] = 0xBB, },

    /// # LAX: LDA + LDX
    /// Illegal.
    Lax {
        DirectPage(None) @[Nmos] = 0xA7,
        DirectPage(X)    @[Nmos] = 0xB7,
        Absolute(None)   @[Nmos] = 0xAF,
        Absolute(Y)      @[Nmos] = 0xBF,
        DpIndirect(X)    @[Nmos] = 0xA3,
        DpIndirect(Y)    @[Nmos] = 0xB3,
    },

    /// # LXA (LAX Immediate): Store * AND oper in A and X
    /// Illegal. Highly Unstable.
    ///
    /// See ANE for details
    Lxa { NoMemory(Immediate) @[Nmos] = 0xAB, },

    /// # RLA: ROL + AND
    /// Illegal.
    Rla {
        DirectPage(None) @[Nmos] = 0x27,
        DirectPage(X)    @[Nmos] = 0x37,
        Absolute(None)   @[Nmos] = 0x2F,
        Absolute(X)      @[Nmos] = 0x3F,
        Absolute(Y)      @[Nmos] = 0x3B,
        DpIndirect(X)    @[Nmos] = 0x23,
        DpIndirect(Y)    @[Nmos] = 0x33,
    },

    /// # RRA: ROR + ADC
    /// Illegal.
    Rra {
        DirectPage(None) @[Nmos] = 0x67,
        DirectPage(X)    @[Nmos] = 0x77,
        Absolute(None)   @[Nmos] = 0x6F,
        Absolute(X)      @[Nmos] = 0x7F,
        Absolute(Y)      @[Nmos] = 0x7B,
        DpIndirect(X)    @[Nmos] = 0x63,
        DpIndirect(Y)    @[Nmos] = 0x73,
    },

    /// # SAX (AXS, AAX): A AND X
    /// Illegal. A and X are put on the bus at the same time, effectively anding them
    Sax {
        DirectPage(None) @[Nmos] = 0x87,
        DirectPage(Y)    @[Nmos] = 0x97,
        Absolute(None)   @[Nmos] = 0x8F,
        DpIndirect(X)    @[Nmos] = 0x83,
    },

    /// # SBX (AXS, SAX): CMP + DEX
    /// Illegal.
    Sbx { NoMemory(Immediate) @[Nmos] = 0xCB, },

    /// # SHA (AHX, AXA)
    /// Illegal. Stores A AND X AND (high-byte of addr. + 1) at addr.
    ///
    /// unstable: sometimes 'AND (H+1)' is dropped, page boundary crossings may not work (with
    /// the high-byte of the value used as the high-byte of the address)
    Sha {
        Absolute(Y)   @[Nmos] = 0x9F,
        DpIndirect(Y) @[Nmos] = 0x93,
    },

    /// # SHX (A11, SXA, XAS)
    /// Illegal. Stores X AND (high-byte of addr. + 1) at addr.
    ///
    /// unstable: sometimes 'AND (H+1)' is dropped, page boundary crossings may not work (with
    /// the high-byte of the value used as the high-byte of the address)
    Shx { Absolute(Y) @[Nmos] = 0x9E, },

    /// # SHY (A11, SYA, SAY)
    /// Illegal. Stores Y AND (high-byte of addr. + 1) at addr.
    ///
    /// unstable: sometimes 'AND (H+1)' is dropped, page boundary crossings may not work (with
    /// the high-byte of the value used as the high-byte of the address)
    Shy { Absolute(Y) @[Nmos] = 0x9C, },

    /// # SLO (ASO): ASL + ORA
    /// Illegal.
    Slo {
        DirectPage(None) @[Nmos] = 0x07,
        DirectPage(X)    @[Nmos] = 0x17,
        Absolute(None)   @[Nmos] = 0x0F,
        Absolute(X)      @[Nmos] = 0x1F,
        Absolute(Y)      @[Nmos] = 0x1B,
        DpIndirect(X)    @[Nmos] = 0x03,
        DpIndirect(Y)    @[Nmos] = 0x13,
    },

    /// # SRE (LSE): LSR + EOR
    /// Illegal.
    Sre {
        DirectPage(None) @[Nmos] = 0x47,
        DirectPage(X)    @[Nmos] = 0x57,
        Absolute(None)   @[Nmos] = 0x4F,
        Absolute(X)      @[Nmos] = 0x5F,
        Absolute(Y)      @[Nmos] = 0x5B,
        DpIndirect(X)    @[Nmos] = 0x43,
        DpIndirect(Y)    @[Nmos] = 0x53,
    },

    /// # TAS (XAS, SHS)
    /// Illegal. Puts A AND X in SP and stores A AND X AND (high-byte of addr. + 1) at addr.
    ///
    /// unstable: sometimes 'AND (H+1)' is dropped, page boundary crossings may not work (with the
    /// high-byte of the value used as the high-byte of the address)
    Tas { Absolute(Y) @[Nmos] = 0x9B, },

    /// # USBC (SBC): SBC + NOP
    /// Illegal. Effectively same as normal SBC immediate, instr. E9.
    Usbc { NoMemory(Immediate) = 0xEB, },

    /// # Jam (or Kill, Halt)
    /// Illegal. CPU enters an infinite loop, eventually constantly reading from 0xFFFF until reset
    Jam {
        Jam @[Nmos] = 0x02,
        Jam @[Nmos] = 0x12,
        Jam @[Nmos] = 0x22,
        Jam @[Nmos] = 0x32,
        Jam @[Nmos] = 0x42,
        Jam @[Nmos] = 0x52,
        Jam @[Nmos] = 0x62,
        Jam @[Nmos] = 0x72,
        Jam @[Nmos] = 0x92,
        Jam @[Nmos] = 0xB2,
        Jam @[Nmos] = 0xD2,
        Jam @[Nmos] = 0xF2,
    },

    // 65C02 New Opcodes ////////////////////////////////////////////////////////////////////////

    /// # Branch Always
    /// Branches to the relative address specified by the signed offset operand.
    ///
    /// Memory access type: None
    Bra { Branch(Relative) @[Cmos, Wdc816] = 0x80, },

    /// # Push X Register
    /// Pushes the X register onto the stack.
    ///
    /// Memory access type: Write
    Phx { Stack @[Cmos, Wdc816] = 0xDA, },
    /// # Push Y Register
    /// Pushes the Y register onto the stack.
    ///
    /// Memory access type: Write
    Phy { Stack @[Cmos, Wdc816] = 0x5A, },
    /// # Pull X Register
    /// Pulls the X register from the stack.
    ///
    /// Memory access type: Read
    Plx { Stack @[Cmos, Wdc816] = 0xFA, },
    /// # Pull Y Register
    /// Pulls the Y register from the stack.
    ///
    /// Memory access type: Read
    Ply { Stack @[Cmos, Wdc816] = 0x7A, },

    /// # Store Zero
    /// Stores zero in memory.
    ///
    /// Memory access type: Write
    Stz {
        DirectPage(None) @[Cmos, Wdc816] = 0x64,
        DirectPage(X)    @[Cmos, Wdc816] = 0x74,
        Absolute(None)   @[Cmos, Wdc816] = 0x9C,
        Absolute(X)      @[Cmos, Wdc816] = 0x9E,
    },
    /// # Test and Reset Bits
    /// The Z bit is set to 1 if the AND of the accumulator and memory is zero.
    /// Then, the memory is ANDed with the complement of the accumulator. Essentially, it's a backwards opposite AND.
    ///
    /// This instruction is only available on the 65C02 and later.
    ///
    /// Memory access type: Read-Modify-Write
    ///
    /// M = M&~A, Z = (A&M)==0
    Trb {
        DirectPage(None) @[Cmos, Wdc816] = 0x14,
        Absolute(None)   @[Cmos, Wdc816] = 0x1C,
    },
    /// # Test and Set Bits
    /// The Z bit is set to 1 if the AND of the accumulator and memory is zero.
    /// Then, the memory is ORed with the accumulator.
    ///
    /// This instruction is only available on the 65C02 and later.
    ///
    /// Memory access type: Read-Modify-Write
    ///
    /// M = M|A, Z = (A&M)==0
    Tsb {
        DirectPage(None) @[Cmos, Wdc816] = 0x04,
        Absolute(None)   @[Cmos, Wdc816] = 0x0C,
    },

    /// # Stop the Processor
    /// Stop the clock input to the CPU, halting the processor. The processor will not respond to any
    /// interrupts or reset signals until the clock input is restored via a hardware reset.
    ///
    /// 65C02 and later only.
    Stp { NoMemory(Implied) @[Cmos, Wdc816] = 0xDB, },
    /// # Wait for Interrupt
    /// Stops the processor until an interrupt occurs. The processor will wait until an interrupt or reset signal
    /// (i.e. IRQ, NMI, RESET). In additiion to reducing power consumption, using WAI also ensures that the interrupt
    /// will be serviced immediately, as the processor will not be executing any instructions--it is the CPU's responsibility
    /// to finish all instructions before entering WAI.
    ///
    /// 65C02 and later only.
    Wai { NoMemory(Implied) @[Cmos, Wdc816] = 0xCB, },

    // R65C02 Opcodes (Rockwell) ////////////////////////////////////////////////////
    /// # Branch on Bit Reset
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbr0 { Branch(DpRelative) @[Cmos] = 0x0F, },
    /// # Branch on Bit Reset
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbr1 { Branch(DpRelative) @[Cmos] = 0x1F, },
    /// # Branch on Bit Reset
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbr2 { Branch(DpRelative) @[Cmos] = 0x2F, },
    /// # Branch on Bit Reset
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbr3 { Branch(DpRelative) @[Cmos] = 0x3F, },
    /// # Branch on Bit Reset
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbr4 { Branch(DpRelative) @[Cmos] = 0x4F, },
    /// # Branch on Bit Reset
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbr5 { Branch(DpRelative) @[Cmos] = 0x5F, },
    /// # Branch on Bit Reset
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbr6 { Branch(DpRelative) @[Cmos] = 0x6F, },
    /// # Branch on Bit Reset
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbr7 { Branch(DpRelative) @[Cmos] = 0x7F, },
    /// # Branch on Bit Set
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbs0 { Branch(DpRelative) @[Cmos] = 0x8F, },
    /// # Branch on Bit Set
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbs1 { Branch(DpRelative) @[Cmos] = 0x9F, },
    /// # Branch on Bit Set
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbs2 { Branch(DpRelative) @[Cmos] = 0xAF, },
    /// # Branch on Bit Set
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbs3 { Branch(DpRelative) @[Cmos] = 0xBF, },
    /// # Branch on Bit Set
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbs4 { Branch(DpRelative) @[Cmos] = 0xCF, },
    /// # Branch on Bit Set
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbs5 { Branch(DpRelative) @[Cmos] = 0xDF, },
    /// # Branch on Bit Set
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbs6 { Branch(DpRelative) @[Cmos] = 0xEF, },
    /// # Branch on Bit Set
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbs7 { Branch(DpRelative) @[Cmos] = 0xFF, },

    /// # Reset Memory Bit
    /// Clear the bit in th zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Rmb0 { DirectPage(None) @[Cmos] = 0x07, },
    /// # Reset Memory Bit
    /// Clear the bit in th zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Rmb1 { DirectPage(None) @[Cmos] = 0x17, },
    /// # Reset Memory Bit
    /// Clear the bit in th zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Rmb2 { DirectPage(None) @[Cmos] = 0x27, },
    /// # Reset Memory Bit
    /// Clear the bit in th zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Rmb3 { DirectPage(None) @[Cmos] = 0x37, },
    /// # Reset Memory Bit
    /// Clear the bit in th zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Rmb4 { DirectPage(None) @[Cmos] = 0x47, },
    /// # Reset Memory Bit
    /// Clear the bit in th zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Rmb5 { DirectPage(None) @[Cmos] = 0x57, },
    /// # Reset Memory Bit
    /// Clear the bit in th zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Rmb6 { DirectPage(None) @[Cmos] = 0x67, },
    /// # Reset Memory Bit
    /// Clear the bit in th zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Rmb7 { DirectPage(None) @[Cmos] = 0x77, },
    /// # Set Memory Bit
    /// Set the bit in the zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Smb0 { DirectPage(None) @[Cmos] = 0x87, },
    /// # Set Memory Bit
    /// Set the bit in the zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Smb1 { DirectPage(None) @[Cmos] = 0x97, },
    /// # Set Memory Bit
    /// Set the bit in the zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Smb2 { DirectPage(None) @[Cmos] = 0xA7, },
    /// # Set Memory Bit
    /// Set the bit in the zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Smb3 { DirectPage(None) @[Cmos] = 0xB7, },
    /// # Set Memory Bit
    /// Set the bit in the zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Smb4 { DirectPage(None) @[Cmos] = 0xC7, },
    /// # Set Memory Bit
    /// Set the bit in the zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Smb5 { DirectPage(None) @[Cmos] = 0xD7, },
    /// # Set Memory Bit
    /// Set the bit in the zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Smb6 { DirectPage(None) @[Cmos] = 0xE7, },
    /// # Set Memory Bit
    /// Set the bit in the zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Smb7 { DirectPage(None) @[Cmos] = 0xF7, },

    // 65C816 Opcodes /////////////////////////////////////////////////////////////////////////////
    // TODO
}
