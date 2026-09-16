use crate::isa::{InterruptType, Latch, MemoryAction, OffsetType};

const MAX_UCYC: usize = 14;

/// A completely stack-based queue that i'm trying to make super duper
/// lightning fast since it's in the hot path
pub struct UcycQueue {
    buf: [MicroOp; MAX_UCYC],
    head: u8, // next to execute (pop front)
    len: u8,  // number of valid entries
}

impl Default for UcycQueue {
    fn default() -> Self {
        Self {
            buf: [MicroOp::read(Latch::Ea, Latch::None, false); MAX_UCYC],
            head: 0,
            len: 0,
        }
    }
}

impl UcycQueue {
    #[inline(always)]
    pub fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.head == self.len
    }
    #[inline(always)]
    pub fn push(&mut self, u: MicroOp) {
        debug_assert!((self.len as usize) < MAX_UCYC);
        unsafe {
            *self.buf.get_unchecked_mut(self.len as usize) = u;
        }
        self.len += 1;
    }
    #[inline(always)]
    pub fn front(&self) -> Option<MicroOp> {
        if self.head >= self.len {
            return None;
        }
        debug_assert!(self.head < MAX_UCYC as u8);
        unsafe { Some(*self.buf.get_unchecked(self.head as usize)) }
    }
    #[inline(always)]
    pub fn pop(&mut self) -> Option<MicroOp> {
        let front = self.front()?;
        self.head += 1;
        if self.head >= self.len {
            self.clear();
        }
        Some(front)
    }
    /// insert can only be called when head is >0! this is a REALLY HACKY
    /// insert that moves head back and inserts there, leaving everything
    /// afterwards in tact but replacing the *last executed* uop.
    #[inline(always)]
    pub fn insert(&mut self, new: MicroOp) {
        debug_assert!(self.head > 0);
        //println!("inserting into buf[{}]", self.head);
        self.head -= 1;
        unsafe { *self.buf.get_unchecked_mut(self.head as usize) = new }
    }

    /// Used for JAM instructions: decreasing the head on each cycle creates
    /// an infinite loop
    #[inline(always)]
    pub fn dec_head(&mut self) {
        self.head -= 1;
    }

    #[inline(always)]
    pub fn len(&self) -> u8 {
        self.len - self.head
    }
}

#[derive(Clone, Copy)]
pub enum BusAction {
    None,
    Read,
    Write,
    DummyRead,
    DummyWrite,
}

#[derive(Clone, Copy, Debug)]
pub struct BusCycle {
    pub addr: Latch,
    pub vda: bool,
    pub vpa: bool,
    pub read: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum AluOp {
    None,
    AddOffset { latch: Latch, offset: OffsetType },
    OffsetWithExtraCycle(OffsetType),
    DecSp,
    Modify,
    SwapEaPc,
    IncLatch(Latch),
    Branch,
    SetPc(u16),
    Jam,
    FixPtr,
    SpecialAddOffset,
}

#[derive(Clone, Copy, Debug)]
pub struct MicroOp {
    pub bus: BusCycle,
    pub inc_src: bool,
    pub local_latch: Latch,
    pub alu: AluOp,
}

impl MicroOp {
    pub fn finish_action(queue: &mut UcycQueue, ctx: DecodeContext) {
        match ctx.action {
            MemoryAction::Read => queue.push(MicroOp::read(Latch::Ea, Latch::Op0, false)),
            MemoryAction::Write => queue.push(MicroOp::alu_write()),
            MemoryAction::ReadModifyWrite => {
                queue.push(MicroOp::read(Latch::Ea, Latch::Op0, false));
                queue.push(MicroOp::alu_modify(ctx.modify_read)); // performs some kind of dummy read/write
                queue.push(MicroOp::alu_write());
            }
        }
    }
    pub const fn read(src: Latch, dest: Latch, inc_src: bool) -> Self {
        let vpa = matches!(src, Latch::Pc);
        MicroOp {
            bus: BusCycle {
                addr: src,
                vda: !vpa,
                vpa,
                read: true,
            },
            inc_src,
            local_latch: dest,
            alu: AluOp::None,
        }
    }
    pub const fn dummy_read(addr: Latch) -> Self {
        MicroOp {
            bus: BusCycle {
                addr,
                vda: false, // on dummy reads, vda and vpa are 0
                vpa: false,
                read: true,
            },
            inc_src: false,
            local_latch: Latch::None,
            alu: AluOp::None,
        }
    }
    pub const fn push(src: Latch) -> Self {
        MicroOp {
            bus: BusCycle {
                addr: Latch::Sp,
                vda: true,
                vpa: false, // literally impossible for vpa to be true on write
                read: false,
            },
            inc_src: false, // bad variable names: if true would actually inc SP
            local_latch: src,
            alu: AluOp::DecSp,
        }
    }
    pub const fn read_and_offset(src: Latch, offset: OffsetType) -> Self {
        MicroOp {
            bus: BusCycle {
                addr: src,
                vda: true,
                vpa: false,
                read: true,
            },
            inc_src: false,
            local_latch: Latch::None,
            alu: AluOp::AddOffset { latch: src, offset },
        }
    }
    pub const fn read_pc_set_pc(new_pc: u16) -> Self {
        MicroOp {
            bus: BusCycle {
                addr: Latch::Pc,
                vda: false,
                vpa: true,
                read: true,
            },
            inc_src: false,
            local_latch: Latch::None,
            alu: AluOp::SetPc(new_pc),
        }
    }
    pub const fn offset_opt_fix(offset_type: OffsetType) -> Self {
        MicroOp {
            bus: BusCycle {
                addr: Latch::Ea,
                vda: true,
                vpa: false,
                read: true,
            },
            inc_src: false,
            local_latch: Latch::Op0,
            alu: AluOp::OffsetWithExtraCycle(offset_type),
        }
    }
    pub const fn alu_write() -> Self {
        MicroOp {
            bus: BusCycle {
                addr: Latch::Ea, // write dest
                vda: true,
                vpa: false,
                read: false,
            },
            inc_src: false,
            local_latch: Latch::None,
            alu: AluOp::None,
        }
    }
    pub const fn alu_modify(read: bool) -> Self {
        MicroOp {
            bus: BusCycle {
                addr: Latch::Ea,
                vda: true,
                vpa: false,
                read,
            },
            inc_src: false,
            local_latch: Latch::None,
            alu: AluOp::Modify,
        }
    }
    pub const fn alu_push() -> Self {
        MicroOp {
            bus: BusCycle {
                addr: Latch::Sp,
                vda: true,
                vpa: false,
                read: false,
            },
            inc_src: false,
            local_latch: Latch::None,
            alu: AluOp::DecSp,
        }
    }
    pub const fn read_ptr1(dest: Latch) -> Self {
        MicroOp {
            bus: BusCycle {
                addr: Latch::Ptr,
                vda: true,
                vpa: false,
                read: true,
            },
            inc_src: false,
            local_latch: dest,
            alu: AluOp::IncLatch(Latch::PtrLo),
        }
    }
    pub const fn read_inv_ptr2(dest: Latch) -> Self {
        MicroOp {
            bus: BusCycle {
                addr: Latch::Ptr, // invalid address!
                vda: true,
                vpa: false,
                read: true,
            },
            inc_src: false,
            local_latch: dest,
            alu: AluOp::FixPtr,
        }
    }
}

pub struct DecodeContext {
    pub e_flag: bool,
    pub m_flag: bool,
    pub x_flag: bool,
    pub action: MemoryAction,
    pub modify_read: bool,
    pub opcode: u8,
}

/// Overrideable MicroCode emitters!
///
/// The default microcode reflects the behavior of the 65816.
pub trait MicroCode {
    fn emit_implied(queue: &mut UcycQueue, _ctx: DecodeContext) {
        queue.push(MicroOp::read(Latch::Pc, Latch::None, false));
    }

    fn emit_immediate(queue: &mut UcycQueue, _ctx: DecodeContext) {
        queue.push(MicroOp::read(Latch::Pc, Latch::Op0, true));
    }

    fn emit_dp(queue: &mut UcycQueue, ctx: DecodeContext, offset: OffsetType) {
        queue.push(MicroOp::read(Latch::Pc, Latch::EaLo, true));
        if offset != OffsetType::None {
            queue.push(MicroOp::read_and_offset(Latch::EaLo, offset));
        }
        MicroOp::finish_action(queue, ctx);
    }

    fn emit_absolute(queue: &mut UcycQueue, ctx: DecodeContext, offset: OffsetType) {
        queue.push(MicroOp::read(Latch::Pc, Latch::EaLo, true));
        queue.push(MicroOp::read(Latch::Pc, Latch::EaHi, true));
        if offset != OffsetType::None {
            queue.push(MicroOp::offset_opt_fix(offset));
        }
        MicroOp::finish_action(queue, ctx);
    }

    fn emit_dp_ind(queue: &mut UcycQueue, ctx: DecodeContext, offset: OffsetType) {
        let x = offset == OffsetType::X;
        let y = offset == OffsetType::Y;
        queue.push(MicroOp::read(Latch::Pc, Latch::PtrLo, true));
        if x {
            // always one cycle because DP overflows through u8, no need to fix high byte
            let cycle = MicroOp {
                bus: BusCycle {
                    addr: Latch::PtrLo, // read from DP pre-offset
                    vda: true,
                    vpa: false,
                    read: true,
                },
                inc_src: false,
                local_latch: Latch::None,
                alu: AluOp::AddOffset {
                    latch: Latch::PtrLo,
                    offset,
                },
            };
            queue.push(cycle);
        }
        queue.push(MicroOp::read(Latch::PtrLo, Latch::EaLo, true)); // read from address in Ptr into EaLow
        queue.push(MicroOp::read(Latch::PtrLo, Latch::EaHi, false)); // read from address in Ptr+1 (with DP wraparound) into EaHigh
        if y {
            queue.push(MicroOp::offset_opt_fix(offset))
        }
        MicroOp::finish_action(queue, ctx);
    }

    fn emit_stack(queue: &mut UcycQueue, ctx: DecodeContext) {
        debug_assert!(
            ctx.action != MemoryAction::ReadModifyWrite,
            "we don't RMW stack, that is meaningless"
        );
        queue.push(MicroOp::read(Latch::Pc, Latch::None, false)); // dummy read whether push or pull
        if ctx.action == MemoryAction::Read {
            queue.push(MicroOp::read(Latch::Sp, Latch::None, true)); // inc Sp first
            queue.push(MicroOp::read(Latch::Sp, Latch::Op0, false)); // pull into Op0
        } else {
            queue.push(MicroOp::alu_push()); // one cycle to push and also dec sp
        }
    }

    fn emit_jmpabs(queue: &mut UcycQueue, _ctx: DecodeContext) {
        queue.push(MicroOp::read(Latch::Pc, Latch::EaLo, true));
        queue.push(MicroOp {
            bus: BusCycle {
                addr: Latch::Pc,
                vda: false,
                vpa: true,
                read: true,
            },
            inc_src: true,
            local_latch: Latch::EaHi,
            alu: AluOp::SwapEaPc,
        });
    }

    fn emit_jmpabsx(queue: &mut UcycQueue, _ctx: DecodeContext) {
        queue.push(MicroOp::read(Latch::Pc, Latch::EaLo, true));
        queue.push(MicroOp::read(Latch::Pc, Latch::EaHi, false));
        queue.push(MicroOp {
            bus: BusCycle {
                addr: Latch::Pc,
                vda: false,
                vpa: true,
                read: true,
            },
            inc_src: false,
            local_latch: Latch::None,
            alu: AluOp::SpecialAddOffset,
        });
        queue.push(MicroOp::read(Latch::Ea, Latch::PcLo, true));
        queue.push(MicroOp::read(Latch::Ea, Latch::PcHi, false));
    }

    fn emit_jmpind(_queue: &mut UcycQueue, _ctx: DecodeContext) {
        todo!("all 8-bit cores override this. we will put the 816 code here when we need it");
    }

    fn emit_jsr(queue: &mut UcycQueue, _ctx: DecodeContext) {
        queue.push(MicroOp::read(Latch::Pc, Latch::EaLo, true));
        queue.push(MicroOp::read(Latch::Pc, Latch::EaHi, false));
        // internal op:
        queue.push(MicroOp {
            bus: BusCycle {
                addr: Latch::Pc,
                vda: false,
                vpa: false,
                read: true,
            },
            inc_src: false,
            local_latch: Latch::None,
            alu: AluOp::SwapEaPc, // ready for next opcode fetch!
        });
        // first have to push PC Hi and Lo to the stack though
        // we've swapped Ea and Pc, which means Ea contains old Pc
        queue.push(MicroOp::push(Latch::EaHi));
        queue.push(MicroOp::push(Latch::EaLo));
    }

    fn emit_rts(queue: &mut UcycQueue, _ctx: DecodeContext) {
        queue.push(MicroOp::dummy_read(Latch::Pc));
        queue.push(MicroOp {
            bus: BusCycle {
                addr: Latch::Pc,
                vda: false,
                vpa: false,
                read: true,
            },
            inc_src: false,
            local_latch: Latch::None,
            alu: AluOp::IncLatch(Latch::Sp), // pre-increment
        });
        queue.push(MicroOp::read(Latch::Sp, Latch::PcLo, true));
        queue.push(MicroOp::read(Latch::Sp, Latch::PcHi, true));
        queue.push(MicroOp::dummy_read(Latch::Sp));
    }

    fn emit_branch_rel(queue: &mut UcycQueue, _ctx: DecodeContext) {
        // on this cycle, we read the signed offset while at the same time deciding whether to branch.
        // if not, this is the only cycle; next cycle is opcode fetch.
        queue.push(MicroOp {
            bus: BusCycle {
                addr: Latch::Pc,
                vda: false,
                vpa: true,
                read: true,
            },
            inc_src: true,
            local_latch: Latch::SignedOffset8,
            alu: AluOp::Branch,
        });
        // either way, AluOp::Branch is going to add the rest of the cycles
    }

    fn emit_branch_dprel(queue: &mut UcycQueue, _ctx: DecodeContext) {
        queue.push(MicroOp::read(Latch::Pc, Latch::EaLo, true));
        queue.push(MicroOp::read(Latch::Ea, Latch::Op0, false));
        queue.push(MicroOp::read(Latch::Ea, Latch::Op0, false));
        queue.push(MicroOp {
            bus: BusCycle {
                addr: Latch::Pc,
                vda: false,
                vpa: true,
                read: true,
            },
            inc_src: true,
            local_latch: Latch::SignedOffset8,
            alu: AluOp::Branch,
        });
        // as above, AluOp::Branch fills out the queue from here
    }

    fn emit_int(queue: &mut UcycQueue, _ctx: DecodeContext, interrupt_type: InterruptType) {
        let inc = interrupt_type == InterruptType::Brk;
        queue.push(MicroOp::read(Latch::Pc, Latch::None, inc));
        queue.push(MicroOp::push(Latch::PcHi));
        queue.push(MicroOp::push(Latch::PcLo));
        let addr = match interrupt_type {
            InterruptType::Brk => {
                queue.push(MicroOp::push(Latch::BrkStatus));
                0xFFFE
            }
            InterruptType::Irq => {
                queue.push(MicroOp::push(Latch::Status));
                0xFFFE
            }
            InterruptType::Nmi => {
                queue.push(MicroOp::push(Latch::Status));
                0xFFFA
            }
            InterruptType::Reset => {
                todo!()
            }
        };
        queue.push(MicroOp::read(Latch::Constant(addr), Latch::PcLo, false));
        queue.push(MicroOp::read(Latch::Constant(addr + 1), Latch::PcHi, false));
    }

    fn emit_rti(queue: &mut UcycQueue, _ctx: DecodeContext) {
        queue.push(MicroOp::read(Latch::Pc, Latch::None, false));
        queue.push(MicroOp::read(Latch::Sp, Latch::None, true));
        queue.push(MicroOp::read(Latch::Sp, Latch::Status, true));
        queue.push(MicroOp::read(Latch::Sp, Latch::PcLo, true));
        queue.push(MicroOp::read(Latch::Sp, Latch::PcHi, false));
    }

    fn emit_jam(_queue: &mut UcycQueue, _ctx: DecodeContext) {
        unreachable!("JAM only occurs on NMOS6502-based cores");
    }

    fn emit_cmos_nop(_queue: &mut UcycQueue, _ctx: DecodeContext, _bytes: u8, _cycles: u8) {
        unreachable!("CMOS NOPs only work on CMOS");
    }
}
