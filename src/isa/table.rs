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
        // the rest of these gentlemen don't check for read or write or whatever, default to Read
        Tax | Tay | Txa | Tya | Tsx | Txs | Dex | Dey | Inx | Iny | Clc | Cld | Cli | Clv | Sec
        | Sed | Sei | Jmp | Jsr | Rts | Bcc | Bcs | Beq | Bmi | Bne | Bpl | Bvc | Bvs | Bra
        | Brk | Nop | Rti | Stp | Wai | Undefined => MemoryAction::Read, // who cares
        // R65C02 instructions
        Bbr0 | Bbr1 | Bbr2 | Bbr3 | Bbr4 | Bbr5 | Bbr6 | Bbr7 | Bbs0 | Bbs1 | Bbs2 | Bbs3
        | Bbs4 | Bbs5 | Bbs6 | Bbs7 => MemoryAction::Read,
        Rmb0 | Rmb1 | Rmb2 | Rmb3 | Rmb4 | Rmb5 | Rmb6 | Rmb7 | Smb0 | Smb1 | Smb2 | Smb3
        | Smb4 | Smb5 | Smb6 | Smb7 => MemoryAction::ReadModifyWrite,
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
    (
        $(
            $(#[$doc:meta])*
            $mnemonic:ident {
                $($mode:ident $( ( $variant:ident ) )? = $code:expr,)+
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
        pub static OPCODE_TABLE: [Instruction; 256] = {
            let mut table: [Instruction; 256] = make_empty_table();
            $(
                $(
                    #[allow(unused_imports)] // for some reason my use statement here is generating this warning
                    use Mnemonic::*;
                    #[allow(unused_imports)]
                    use address_mode_subtypes::
                        {NoMemType::*, OffsetType::*, JumpType::*, BranchType::*, JumpType::*};
                    let memory_action = action_for_mnemonic($mnemonic);
                    table[$code as usize] = Instruction {
                        mnemonic: Mnemonic::$mnemonic,
                        address_mode: AddressMode::$mode $( ( $variant ) )?,
                        memory_action,
                    };
                )+
            )+
            table
        };

        impl Instruction {
            pub fn from_byte(op: u8) -> Instruction {
                // SAFETY: OPCODE_TABLE is a completely filled array with the full range of u8 (0x00-0xFF) validly indexable
                unsafe { *OPCODE_TABLE.get_unchecked(op as usize) }
            }
        }

        impl Display for Mnemonic {
            fn fmt(&self, f: &mut Formatter) -> Result<(), Error> {
                match *self {
                    $(
                        Mnemonic::$mnemonic => write!(f, "{}", stringify!($mnemonic)),
                    )+
                    Mnemonic::Undefined => write!(f, "Und")
                }
            }
        }
    }
}

// this macro also creates the pub enum Mnemonic
define_opcodes! {
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

        // 65C02
        DpIndirect(None) = 0xB2,
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

        // 65C02
        DpIndirect(None) = 0x92,
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

        // 65C02
        DpIndirect(None) = 0x32,
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

        // 65C02
        DpIndirect(None) = 0x52,
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

        // 65C02
        DpIndirect(None) = 0x12,
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

        // 65C02
        NoMemory(Immediate) = 0x89,
        DirectPage(X)       = 0x34,
        Absolute(X)         = 0x3C,
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

        // 65C02
        DpIndirect(None) = 0x72,
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

        // 65C02
        DpIndirect(None) = 0xF2,
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

        // 65C02
        DpIndirect(None) = 0xD2,
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

        // 65C02
        NoMemory(Implied) = 0x1A,
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

        // 65C02
        NoMemory(Implied) = 0x3A,
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

        // 65C02
        Jump(JmpIndirectX) = 0x7C,
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
    /// # No Operation
    /// Does nothing.
    ///
    /// Memory access type: None
    Nop { NoMemory(Implied) = 0xEA, },
    /// # Return from Interrupt
    /// Pulls the status register and program counter from the stack.
    ///
    /// Memory access type: None
    Rti { Jump(FromInterrupt) = 0x40, },

    // 65C02 New Opcodes ////////////////////////////////////////////////////////////////////////

    /// # Branch Always
    /// Branches to the relative address specified by the signed offset operand.
    ///
    /// Memory access type: None
    Bra { Branch(Relative) = 0x80, },

    /// # Push X Register
    /// Pushes the X register onto the stack.
    ///
    /// Memory access type: Write
    Phx { Stack = 0xDA, },
    /// # Push Y Register
    /// Pushes the Y register onto the stack.
    ///
    /// Memory access type: Write
    Phy { Stack = 0x5A, },
    /// # Pull X Register
    /// Pulls the X register from the stack.
    ///
    /// Memory access type: Read
    Plx { Stack = 0xFA, },
    /// # Pull Y Register
    /// Pulls the Y register from the stack.
    ///
    /// Memory access type: Read
    Ply { Stack = 0x7A, },

    /// # Store Zero
    /// Stores zero in memory.
    ///
    /// Memory access type: Write
    Stz {
        DirectPage(None) = 0x64,
        DirectPage(X)    = 0x74,
        Absolute(None)   = 0x9C,
        Absolute(X)      = 0x9E,
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
        DirectPage(None) = 0x14,
        Absolute(None)   = 0x1C,
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
        DirectPage(None) = 0x04,
        Absolute(None)   = 0x0C,
    },

    /// # Stop the Processor
    /// Stop the clock input to the CPU, halting the processor. The processor will not respond to any
    /// interrupts or reset signals until the clock input is restored via a hardware reset.
    ///
    /// 65C02 and later only.
    Stp { NoMemory(Implied) = 0xDB, },
    /// # Wait for Interrupt
    /// Stops the processor until an interrupt occurs. The processor will wait until an interrupt or reset signal
    /// (i.e. IRQ, NMI, RESET). In additiion to reducing power consumption, using WAI also ensures that the interrupt
    /// will be serviced immediately, as the processor will not be executing any instructions--it is the CPU's responsibility
    /// to finish all instructions before entering WAI.
    ///
    /// 65C02 and later only.
    Wai { NoMemory(Implied) = 0xCB, },

    // R65C02 Opcodes (Rockwell) ////////////////////////////////////////////////////
    /// # Branch on Bit Reset
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbr0 { Branch(DpRelative) = 0x0F, },
    /// # Branch on Bit Reset
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbr1 { Branch(DpRelative) = 0x1F, },
    /// # Branch on Bit Reset
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbr2 { Branch(DpRelative) = 0x2F, },
    /// # Branch on Bit Reset
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbr3 { Branch(DpRelative) = 0x3F, },
    /// # Branch on Bit Reset
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbr4 { Branch(DpRelative) = 0x4F, },
    /// # Branch on Bit Reset
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbr5 { Branch(DpRelative) = 0x5F, },
    /// # Branch on Bit Reset
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbr6 { Branch(DpRelative) = 0x6F, },
    /// # Branch on Bit Reset
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbr7 { Branch(DpRelative) = 0x7F, },
    /// # Branch on Bit Set
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbs0 { Branch(DpRelative) = 0x8F, },
    /// # Branch on Bit Set
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbs1 { Branch(DpRelative) = 0x9F, },
    /// # Branch on Bit Set
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbs2 { Branch(DpRelative) = 0xAF, },
    /// # Branch on Bit Set
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbs3 { Branch(DpRelative) = 0xBF, },
    /// # Branch on Bit Set
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbs4 { Branch(DpRelative) = 0xCF, },
    /// # Branch on Bit Set
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbs5 { Branch(DpRelative) = 0xDF, },
    /// # Branch on Bit Set
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbs6 { Branch(DpRelative) = 0xEF, },
    /// # Branch on Bit Set
    /// This instruction has two operands: 1) a zero page address to test the bit of, and 2) a relative offset.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Bbs7 { Branch(DpRelative) = 0xFF, },

    /// # Reset Memory Bit
    /// Clear the bit in th zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Rmb0 { DirectPage(None) = 0x07, },
    /// # Reset Memory Bit
    /// Clear the bit in th zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Rmb1 { DirectPage(None) = 0x17, },
    /// # Reset Memory Bit
    /// Clear the bit in th zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Rmb2 { DirectPage(None) = 0x27, },
    /// # Reset Memory Bit
    /// Clear the bit in th zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Rmb3 { DirectPage(None) = 0x37, },
    /// # Reset Memory Bit
    /// Clear the bit in th zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Rmb4 { DirectPage(None) = 0x47, },
    /// # Reset Memory Bit
    /// Clear the bit in th zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Rmb5 { DirectPage(None) = 0x57, },
    /// # Reset Memory Bit
    /// Clear the bit in th zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Rmb6 { DirectPage(None) = 0x67, },
    /// # Reset Memory Bit
    /// Clear the bit in th zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Rmb7 { DirectPage(None) = 0x77, },
    /// # Set Memory Bit
    /// Set the bit in the zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Smb0 { DirectPage(None) = 0x87, },
    /// # Set Memory Bit
    /// Set the bit in the zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Smb1 { DirectPage(None) = 0x97, },
    /// # Set Memory Bit
    /// Set the bit in the zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Smb2 { DirectPage(None) = 0xA7, },
    /// # Set Memory Bit
    /// Set the bit in the zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Smb3 { DirectPage(None) = 0xB7, },
    /// # Set Memory Bit
    /// Set the bit in the zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Smb4 { DirectPage(None) = 0xC7, },
    /// # Set Memory Bit
    /// Set the bit in the zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Smb5 { DirectPage(None) = 0xD7, },
    /// # Set Memory Bit
    /// Set the bit in the zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Smb6 { DirectPage(None) = 0xE7, },
    /// # Set Memory Bit
    /// Set the bit in the zero page memory location specified in the operand.
    /// This instruction is ONLY available on the R65C02 (not the 16-bit cpus).
    Smb7 { DirectPage(None) = 0xF7, },
}
