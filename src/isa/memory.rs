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

use crate::isa::op::{Uop, UopQueue};

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

            // 16-bit
            Jump(JumpType::JmpIndirectLong) => write!(f, "JmpIndirectLong"),
            Branch(BranchType::RelativeLong) => write!(f, "Branch RelLong"),
            AbsoluteLong(OffsetType::None) => write!(f, "AbsoluteLong"),
            AbsoluteLong(OffsetType::X) => write!(f, "AbsoluteLongX"),
            AbsoluteLong(OffsetType::Y) => write!(f, "AbsoluteLongY"),
            BlockMove => write!(f, "BlockMove"),
            StackRelative(OffsetType::None) => write!(f, "StackRelative"),
            StackRelative(OffsetType::Y) => write!(f, "StackRelativeY"),
            StackRelative(OffsetType::X) => unreachable!("no such thing as StackRelativeX"),
        }
    }
}

/// No matter what Latch variant, anything read or written will update Data Latch.
/// This enum contains internal latches other than Data Latch
#[derive(Clone, Copy)]
pub enum Latch {
    None,
    Pc,
    PcLo,
    PcHi,
    Ea,
    EaLo,
    EaHi,
    Op0,
    Ptr,
    PtrLo,
    PtrHi,
    Status,
    SignedOffset8, // for branches
}

#[derive(Clone, Copy)]
pub enum MemLoc {
    Pc,
    PcInc, // i.e. PC++
    Ea,
    Ptr8,
    Ptr8Inc,
    Sp,
    Const(u16),
}

// useful const fn's to wrap common uop patterns
impl Uop {
    const fn fetch_into(dest: Latch) -> Self {
        Uop::Read {
            src: if !matches!(dest, Latch::None) {
                MemLoc::PcInc
            } else {
                MemLoc::Pc
            },
            dest,
        }
    }
    const fn read_into(dest: Latch) -> Self {
        Uop::Read {
            src: MemLoc::Ea,
            dest,
        }
    }
    const fn read_ptr8_low() -> Self {
        Uop::Read {
            src: MemLoc::Ptr8Inc,
            dest: Latch::EaLo,
        }
    }
    const fn read_ptr8_hi() -> Self {
        Uop::Read {
            src: MemLoc::Ptr8,
            dest: Latch::EaHi,
        }
    }
    const fn read_dp_then_offset(latch: Latch, offset_type: OffsetType) -> Self {
        Uop::AddOffset8 {
            latch: latch,
            offset_type,
        }
    }
    const fn read_or_fix(offset_type: OffsetType, memory_action: MemoryAction) -> Self {
        Uop::ReadOrFix(offset_type, memory_action)
    }
    const fn push(src: Latch, dec: bool) -> Self {
        Uop::Push { src, dec }
    }
    const fn pull(dest: Latch, inc: bool) -> Self {
        Uop::Pull { dest, inc }
    }

    /// the differentiation between read, write, and read-modify-write
    fn finish_action(queue: &mut UopQueue, action: MemoryAction) {
        match action {
            MemoryAction::Read => queue.push(Uop::read_into(Latch::Op0)),
            MemoryAction::Write => queue.push(Uop::AluWrite),
            MemoryAction::ReadModifyWrite => {
                queue.push(Uop::read_into(Latch::Op0));
                queue.push(Uop::AluModify); // performs some kind of dummy read/write
                queue.push(Uop::AluWrite);
            }
        }
        queue.push(Uop::Finished); // clean up and fetch next opcode
    }
}

impl AddressMode {
    pub fn emit_uops(&self, queue: &mut UopQueue, action: MemoryAction) {
        use Latch::*;
        match *self {
            AddressMode::NoMemory(no_mem_type) => {
                // only READS. immediate value read into Op0
                match no_mem_type {
                    NoMemType::Implied => queue.push(Uop::fetch_into(None)),
                    NoMemType::Immediate => queue.push(Uop::fetch_into(Op0)),
                }
                queue.push(Uop::Finished);
                // done! this is the full queue. there's no such thing as an implied or immediate write or rmw
                // so as long as we do the thing on the Finished cycle at the same time as we fetch the next opcode
                // this is cycle accurate guaranteed!
            }
            AddressMode::DirectPage(offset_type) => {
                queue.push(Uop::fetch_into(EaLo));
                if offset_type != OffsetType::None {
                    // need an extra cycle to add offset
                    // also reads from pre-offset EA
                    queue.push(Uop::read_dp_then_offset(Ea, offset_type));
                }
                // now, we read from the correct address (either fetched EaLow or offsetted one from last cycle)
                Uop::finish_action(queue, action);
            }
            AddressMode::Absolute(offset_type) => {
                queue.push(Uop::fetch_into(EaLo));
                queue.push(Uop::fetch_into(EaHi));
                if offset_type != OffsetType::None {
                    // we have a 16-bit address in EA. technically, last cycle when we fetched the high byte,
                    // we could have added the offset to EaLo on that cycle. If that results in a valid address
                    // (i.e., no overflow), and the action is read, that's the end of the whole instruction--next
                    // cycle is opcode fetch. if it's write or read-modify-write, this cycle is a read from EA
                    // regardless of its validity
                    queue.push(Uop::read_or_fix(offset_type, action));
                    // if read action from valid, queue is cleared and opcode fetch hapens next cycle. therefore we dont
                    // care about the uops added by finish_action, they will be skipped
                    // if read action from invalid, or nonread, this cycle fixes, and finish_action uops will execute
                }
                // these finish_action uops only execute if reading from overflow fixed addr, or nonread
                Uop::finish_action(queue, action);
            }
            AddressMode::DpIndirect(offset_type) => {
                let x = offset_type == OffsetType::X;
                let y = offset_type == OffsetType::Y;
                queue.push(Uop::fetch_into(Ptr));
                if x {
                    // always one cycle because DP overflows through u8, no need to fix high byte
                    queue.push(Uop::read_dp_then_offset(Ptr, offset_type));
                }
                queue.push(Uop::read_ptr8_low()); // read from address in Ptr into EaLow
                queue.push(Uop::read_ptr8_hi()); // read from address in Ptr+1 (with DP wraparound) into EaHigh
                if y {
                    // see above (Absolute indexed) for rationale regarding cycle counts here
                    queue.push(Uop::read_or_fix(offset_type, action));
                }
                Uop::finish_action(queue, action);
            }
            AddressMode::Stack => {
                debug_assert!(
                    action != MemoryAction::ReadModifyWrite,
                    "we don't RMW stack, that is meaningless"
                );
                queue.push(Uop::fetch_into(None)); // dummy read whether push or pull
                if action == MemoryAction::Read {
                    queue.push(Uop::pull(None, true)); // inc Sp first
                    queue.push(Uop::pull(Op0, false)); // pull into Op0
                } else {
                    queue.push(Uop::AluPush); // one cycle to push and also dec sp
                }
                queue.push(Uop::Finished); // fetch next opcode
            }
            AddressMode::Jump(jump_type) => {
                match jump_type {
                    JumpType::JmpAbsolute => {
                        queue.push(Uop::fetch_into(EaLo));
                        // on cycle 2, we fetch EaHi from PC and then set PC to EA all on the same cycle.
                        // therefore, we need a bespoke Uop
                        queue.push(Uop::FetchEaHiAndJump);
                        queue.push(Uop::Finished);
                    }
                    JumpType::JmpIndirect | JumpType::JmpIndirectX => {
                        todo!("JmpIndirect requires absolute pointer support");
                    }
                    JumpType::ToSubroutine => {
                        // this is very similar to JmpAbsolute, with stuff in the middle
                        queue.push(Uop::fetch_into(EaLo));
                        // for cycle 2, i can't find amazing documentation about what happens here, but it seems like
                        // some kind of internal operation happens, replete with a dummy read at SP
                        queue.push(Uop::Read {
                            src: MemLoc::Sp,
                            dest: None,
                        });
                        queue.push(Uop::push(PcHi, true));
                        queue.push(Uop::push(PcLo, true));
                        queue.push(Uop::FetchEaHiAndJump); // again, similar to JmpAbsolute
                        queue.push(Uop::Finished);
                    }
                    JumpType::FromSubroutine => {
                        queue.push(Uop::fetch_into(None));
                        queue.push(Uop::pull(None, true)); // inc SP
                        queue.push(Uop::pull(EaLo, true));
                        queue.push(Uop::pull(EaHi, false));
                        queue.push(Uop::ReturnToEa); // set PC to EA, dummy read PC, then inc PC
                        queue.push(Uop::Finished);
                    }
                    JumpType::ToInterrupt => {
                        queue.push(Uop::fetch_into(None));
                        queue.push(Uop::push(PcHi, true));
                        queue.push(Uop::push(PcLo, true));
                        queue.push(Uop::push(Status, true));
                        queue.push(Uop::Read {
                            src: MemLoc::Const(0xFFFE),
                            dest: PcLo,
                        }); // read 0xFFFE directly into PC low
                        queue.push(Uop::Read {
                            src: MemLoc::Const(0xFFFF),
                            dest: PcHi,
                        }); // read 0xFFFF directly into PC high
                        queue.push(Uop::Finished);
                    }
                    JumpType::FromInterrupt => {
                        queue.push(Uop::fetch_into(None));
                        queue.push(Uop::pull(None, true)); // inc SP
                        queue.push(Uop::pull(Status, true));
                        queue.push(Uop::pull(PcLo, true));
                        queue.push(Uop::pull(PcHi, false));
                        queue.push(Uop::Finished);
                    }
                    JumpType::JmpIndirectLong => {
                        todo!("16 bit opcodes")
                    }
                }
            }
            AddressMode::Branch(branch_type) => match branch_type {
                BranchType::Relative => {
                    queue.push(Uop::fetch_into(SignedOffset8));
                    queue.push(Uop::AluBranch); // branch will populate the queue here
                }
                BranchType::DpRelative => {
                    // BBR and BBS
                    queue.push(Uop::fetch_into(EaLo)); // the addr of byte to test
                    queue.push(Uop::read_into(Op0)); // the byte to test
                    queue.push(Uop::fetch_into(SignedOffset8)); // the jump to take
                    queue.push(Uop::AluBranch); // branch will populate the queue here
                }
                BranchType::RelativeLong => {
                    todo!("16 bit opcodes")
                }
            },
            AddressMode::AbsoluteLong(_) => {
                todo!("16 bit instructions not implemented yet")
            }
            AddressMode::BlockMove => {
                todo!("16 bit instructions not implemented yet")
            }
            AddressMode::StackRelative(_) => {
                todo!("16 bit instructions not implemented yet")
            }
        }
    }
}
