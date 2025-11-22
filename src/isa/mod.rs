pub mod microcycle;
pub mod op;
pub mod table;

#[repr(u8)]
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum MemoryAction {
    /// Memory is read only, or nothing happens. Examples: LDA, NOP
    Read,
    /// Memory is only written (not read). Examples: STA
    Write,
    /// Memory is read, then some action is performed by the ALU, and then that memory is written.
    /// Examples: INC, ASL
    ReadModifyWrite,
}

impl Default for MemoryAction {
    fn default() -> Self {
        MemoryAction::Read
    }
}

pub mod address_mode_subtypes {
    #[repr(u8)]
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub enum NoMemType {
        Implied,
        Immediate,
    }

    #[repr(u8)]
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub enum OffsetType {
        None,
        X,
        Y,
    }

    #[repr(u8)]
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub enum JumpType {
        JmpAbsolute,
        JmpIndirect,
        JmpIndirectLong,
        JmpIndirectX,
        ToSubroutine,
        ToInterrupt,
        FromSubroutine,
        FromInterrupt,
    }

    #[repr(u8)]
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub enum BranchType {
        Relative,
        DpRelative,
        RelativeLong,
    }
}
use core::fmt;

pub use address_mode_subtypes::*;

use crate::isa::microcycle::{DecodeContext, MicroCode, UcycQueue};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum AddressMode {
    // 6502 /////////////////////////////////////////////////////////////////////////////////////////////
    /// # No memory access
    /// The instruction has no operands. The subtypes of implied include:
    /// - Implied: no special behavior (eg. NOP; INX) - The only memory access that happens is a read of
    /// PC+1, which is thrown away and the PC is not incremented.
    /// - Accumulator: the instruction operates on the accumulator only, no memory access required (eg. ASL A).
    /// The next byte is read from PC+1 and thrown away, PC not incremented.
    /// - Immediate: the operand is a 1-byte value following the opcode (eg. LDA #55) - The next byte is
    /// read from PC+1 and used as a literal operand, PC is incremented.
    NoMemory(NoMemType),

    /// # Stack
    /// PHA, PHP, PLA, PLP
    ///
    /// Push or pull a value to or from the stack. The stack pointer is decremented for pushes and incremented
    /// for pulls. The stack is located at $0100-$01FF.
    /// Push or pull is decided by the action type (Read or Write)
    Stack,

    /// # Direct Page / Zero Page
    /// The operand is a 1-byte value following the opcode, which is an address on the zero page / direct page.
    /// Optionally, the X or Y register can be added to the address before the read or write.
    /// - No offset (eg. LDA $55)
    /// - X offset (eg. LDA $55,X)
    /// - Y offset (eg. STX $55,Y) (only usable by LDX and STX)
    DirectPage(OffsetType),

    /// # Absolute
    /// The operand is a 2-byte value following the opcode, which is the address to read from or write to.
    /// Optionally, the X or Y register can be added to the address before the read or write.
    /// - No offset (eg. LDA $2000)
    /// - X offset (eg. LDA $2000,X)
    /// - Y offset (eg. LDA $2000,Y)
    Absolute(OffsetType),

    /// # Direct Page / Zero Page Indirect
    /// DpIndirect(None) is not possible on the 6502. For the 65C02 Jmp instruction, use Jump(Indirect).
    /// The operand is a 1-byte value following the opcode, which is an address on the zero page. There is
    /// zero-page wraparound, so $FF will wrap around to $00.
    /// The behavior of Indirect depends on which offset type we're using:
    /// - X offset (eg. LDA ($55,X)) - The X register is added to the zero page address to find the
    /// two-byte address to read from.
    /// - Y offset (eg. LDA ($55),Y) - The two-byte address is taken from the zero page at the provided address,
    /// and the Y register is added to it to find the final address.
    DpIndirect(OffsetType),

    /// # Jump
    /// Used for JMP instruction only.
    /// - Absolute (eg. JMP $2000) - The operand is a 2-byte value following the opcode, which is an address to
    /// jump to.
    /// - Indirect (eg. JMP ($2000)) - The operand is a 2-byte value following
    /// the opcode, which is an address containing the 2-byte address to jump to.
    /// - IndirectLong (eg. JMP ($2000)) - *for 16-bit only!* - Same as indirect, but three bytes are read from the address,
    /// forming a full 24-bit absolute address
    /// - IndirectX (eg. JMP (&2010,X)) - *Not available on the 6502!* - Same as Indirect, but the X register
    /// is added to the address before reading.
    /// - ToSubroutine (eg. JSR $2000) - The operand is a 2-byte value following the opcode, which is the address
    /// of a subroutine to jump to. The return address (next instruction - 1) is pushed to the stack.
    Jump(JumpType),

    /// # Branch
    /// The operand is a 1-byte value following the opcode, which is a signed offset from the PC.
    /// - Relative (eg. BEQ LABEL12) - The offset is added to the PC to find the new address to jump to.
    /// (note: LABEL12 would have to be between -128 and 127 bytes from the branch instruction)
    /// - ZeroPageRelative - *Only available on the R65C02!* - Used with the BBR and BBS instructions.
    /// - RelativeLong - *Only available in 16-bits!* the offset is a two-byte 16-bit operand
    /// The operands are:
    ///     - 1-byte value, which is the byte in the zero page to test the bit against
    ///     - 1-byte value, which is a signed relative address to jump to if the branch succeeds
    Branch(BranchType),

    /// # Jam
    /// This NMOS-only address type is used for illegal Jam opcodes, which make predictable memory accesses
    /// and then enter an infinite loop.
    Jam,

    // 16-bit instructions /////////////////////////////////////////////////////////////////////
    /// # Absolute Long
    /// The operand is a 3-byte value following the opcode, which is a full 24-bit address to read from or
    /// write to. Optionally, the X or Y register can be added to the address before use.
    /// - No offset (eg. LDA $200000)
    /// - X offset (eg. LDA $200000,X)
    /// - Y offset (eg. LDA $200000,Y)
    ///
    /// 65C816 only.
    AbsoluteLong(OffsetType),

    /// # Direct Page Indirect Long
    /// The operand is a one-byte DirectPage address, pointing to a three-byte absolute address. Similar to
    /// DpIndirect, but for 3-byte addresses instead of 2.
    /// - No offset (eg. LDA [$80])
    /// - Y offset (eg. LDA [$80],y)
    /// - X offset doesn't exist, as far as I can tell. TODO
    DpIndirectLong(OffsetType),

    /// # Block Move
    /// The second byte of the instruction contains the high-order 8 bits of the destination address  
    /// and the Y Index Register contains the low-order 16 bits of the destination address. The third
    /// byte of the instruction contains the high-order 8 bits of the source address and the X Index Register
    /// contains the low-order bits of the source address. The C Accumulator contains one less than  
    /// the number of bytes to move. The second byte of the block move instructions is also loaded
    /// into the Data Bank Register.
    ///
    /// - eg. "MVN $55,$44" moves C+1 bytes from 0x44XXXX to 0x55YYYY, and $55 is loaded into DBR
    ///
    /// 65C816 only.
    BlockMove,

    /// # Stack Relative
    /// The second byte of the instruction is added to the 16-bit stack pointer to form the effective address
    /// at DBR=0. With OffsetType=None, this value becomes the read/write target. With OffsetType=Y, this value
    /// is added to the Y register, and forms a pointer at the current DBR (not zero!), which becomes the
    /// read/write target.
    ///
    /// - No offset (eg. LDA 4,S)
    /// - Y offset (eg. LDA (5,S),Y)
    /// - X offset is invalid
    ///
    /// 65C816 only.
    StackRelative(OffsetType),

    CmosNop(u8, u8),
}

impl fmt::Display for AddressMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use AddressMode::*;
        match *self {
            NoMemory(NoMemType::Implied) => write!(f, "Implied"),
            NoMemory(NoMemType::Immediate) => write!(f, "Immediate"),
            DirectPage(OffsetType::None) => write!(f, "DirectPage"),
            DirectPage(OffsetType::X) => write!(f, "DirectPageX"),
            DirectPage(OffsetType::Y) => write!(f, "DirectPageY"),
            Absolute(OffsetType::None) => write!(f, "Absolute"),
            Absolute(OffsetType::X) => write!(f, "AbsoluteX"),
            Absolute(OffsetType::Y) => write!(f, "AbsoluteY"),
            DpIndirect(OffsetType::None) => write!(f, "Indirect"),
            DpIndirect(OffsetType::X) => write!(f, "IndirectX"),
            DpIndirect(OffsetType::Y) => write!(f, "IndirectY"),
            Jump(JumpType::JmpAbsolute) => write!(f, "JmpAbsolute"),
            Jump(JumpType::JmpIndirect) => write!(f, "JmpIndirect"),
            Jump(JumpType::JmpIndirectX) => write!(f, "JmpIndirectX"),
            Jump(JumpType::ToSubroutine) => write!(f, "Special (Jsr)"),
            Jump(JumpType::ToInterrupt) => write!(f, "Special (Brk)"),
            Jump(JumpType::FromSubroutine) => write!(f, "Special (Rts)"),
            Jump(JumpType::FromInterrupt) => write!(f, "Special (Rti)"),
            Branch(BranchType::Relative) => write!(f, "Branch Rel"),
            Branch(BranchType::DpRelative) => write!(f, "Branch DpRel"),
            Stack => write!(f, "Stack"),
            Jam => write!(f, "Jam"),
            CmosNop(bytes, cycles) => write!(f, "CMOS Nop, {bytes} bytes, {cycles} cycles"),

            // 16-bit
            Jump(JumpType::JmpIndirectLong) => write!(f, "JmpIndirectLong"),
            Branch(BranchType::RelativeLong) => write!(f, "Branch RelLong"),
            AbsoluteLong(OffsetType::None) => write!(f, "AbsoluteLong"),
            AbsoluteLong(OffsetType::X) => write!(f, "AbsoluteLongX"),
            AbsoluteLong(OffsetType::Y) => write!(f, "AbsoluteLongY"),
            DpIndirectLong(OffsetType::None) => write!(f, "IndirectLong"),
            DpIndirectLong(OffsetType::Y) => write!(f, "IndirectLongY"),
            DpIndirectLong(OffsetType::X) => write!(f, "no such thing as IndirectLongX"),
            BlockMove => write!(f, "BlockMove"),
            StackRelative(OffsetType::None) => write!(f, "StackRelative"),
            StackRelative(OffsetType::Y) => write!(f, "StackRelativeY"),
            StackRelative(OffsetType::X) => unreachable!("no such thing as StackRelativeX"),
        }
    }
}

/// No matter what Latch variant, anything read or written will update Data Latch.
/// This enum contains internal latches other than Data Latch
#[derive(Debug, Clone, Copy)]
pub enum Latch {
    None,
    Constant(u16),
    Pc,
    PcLo,
    PcHi,
    Sp,
    Ea,
    EaLo,
    EaHi,
    Op0,
    Ptr,
    PtrLo,
    PtrHi,
    Status,
    BrkStatus,     // Status with interrupt flags
    SignedOffset8, // for branches
}

impl AddressMode {
    pub fn emit_ucycs<M: MicroCode>(&self, queue: &mut UcycQueue, ctx: DecodeContext) {
        match *self {
            AddressMode::NoMemory(NoMemType::Implied) => M::emit_implied(queue, ctx),
            AddressMode::NoMemory(NoMemType::Immediate) => M::emit_immediate(queue, ctx),
            AddressMode::DirectPage(off) => M::emit_dp(queue, ctx, off),
            AddressMode::Absolute(off) => M::emit_absolute(queue, ctx, off),
            AddressMode::DpIndirect(off) => M::emit_dp_ind(queue, ctx, off),
            AddressMode::Stack => M::emit_stack(queue, ctx),
            AddressMode::Jump(JumpType::JmpAbsolute) => M::emit_jmpabs(queue, ctx),
            AddressMode::Jump(JumpType::JmpIndirect) => {
                M::emit_jmpind(queue, ctx, OffsetType::None)
            }
            AddressMode::Jump(JumpType::JmpIndirectX) => M::emit_jmpind(queue, ctx, OffsetType::X),
            AddressMode::Jump(JumpType::ToSubroutine) => M::emit_jsr(queue, ctx),
            AddressMode::Jump(JumpType::FromSubroutine) => M::emit_rts(queue, ctx),
            AddressMode::Jump(JumpType::ToInterrupt) => M::emit_brk(queue, ctx),
            AddressMode::Jump(JumpType::FromInterrupt) => M::emit_rti(queue, ctx),
            AddressMode::Branch(BranchType::DpRelative) => M::emit_branch_dprel(queue, ctx),
            AddressMode::Branch(BranchType::Relative) => M::emit_branch_rel(queue, ctx),
            AddressMode::Jam => M::emit_jam(queue, ctx),
            AddressMode::CmosNop(bytes, cycles) => M::emit_cmos_nop(queue, ctx, bytes, cycles),
            // 16-bit only:
            AddressMode::Branch(BranchType::RelativeLong) => todo!(),
            AddressMode::AbsoluteLong(_offset_type) => todo!(),
            AddressMode::DpIndirectLong(_offset_type) => todo!(),
            AddressMode::BlockMove => todo!(),
            AddressMode::StackRelative(_offset_type) => todo!(),
            AddressMode::Jump(JumpType::JmpIndirectLong) => todo!(),
        }
    }
}
