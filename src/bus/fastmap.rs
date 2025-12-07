//! Fast, page-mapped bus for 65x cores.
//!
//! Goal: O(1) RAM/ROM access using a 4 KiB page table over the full 16 MiB space.
//! IO pages go through a small dynamic jump table; only those accesses pay a vtable cost.

use super::{Bus, Lines, OpenBus};
use core::ptr::NonNull;

const PAGE_SHIFT: u32 = 12; // 4 KiB
const PAGE_SIZE: usize = 1 << PAGE_SHIFT; // 4096
const PAGE_MASK: u32 = (PAGE_SIZE as u32) - 1;
const NUM_PAGES: usize = (1 << 24) / PAGE_SIZE; // 16 MiB / 4 KiB = 4096

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum PageKind {
    Open = 0,
    Ram = 1,
    Rom = 2,
    Io = 3,
    RamIo = 4,
    RomIo = 5,
}

/// Compact per-page mapping:  
/// - `ptr`: base pointer for RAM/ROM; unused for Open/Io  
/// - `io`: optional NonNull pointer to IO handler when `kind == Io`  
/// - `io_len`: length for IO overlay pages (if applicable)  
/// - `kind`: tag
#[derive(Clone, Copy)]
struct Page {
    ptr: *mut u8,
    io: Option<NonNull<dyn IoHandler>>,
    io_len: u16,
    kind: PageKind,
}

impl Default for Page {
    #[inline(always)]
    fn default() -> Self {
        Self {
            ptr: core::ptr::null_mut(),
            io: None,
            io_len: 0,
            kind: PageKind::Open,
        }
    }
}

/// Minimal IO handler interface. Implementors may add wait states.
pub trait IoHandler {
    fn read(&mut self, addr: u32, vda: bool, vpa: bool) -> u8;
    fn write(&mut self, addr: u32, data: u8, vda: bool, vpa: bool);
}

pub struct FastMapBus {
    pages: [Page; NUM_PAGES],
    lines: Lines,
    open: OpenBus,
}

impl FastMapBus {
    #[inline]
    pub fn new() -> Self {
        Self {
            pages: [Page::default(); NUM_PAGES],
            lines: Lines::none(),
            open: OpenBus::default(),
        }
    }

    /// Compute the page index for a 24-bit address.
    #[inline(always)]
    fn page_index(addr: u32) -> usize {
        (addr >> PAGE_SHIFT) as usize
    }

    /// Map a single 4 KiB page of RAM at (bank,page_in_bank).
    /// Safety: `base` must point to at least 4096 writable bytes for this page.
    #[inline]
    pub unsafe fn map_ram_page(&mut self, bank: u8, page_in_bank: u8, base: *mut u8) {
        let page = ((bank as usize) << (16 - PAGE_SHIFT)) | (page_in_bank as usize);
        self.pages[page] = Page {
            ptr: base,
            io: None,
            io_len: 0,
            kind: PageKind::Ram,
        };
    }

    /// Map a single 4 KiB page of ROM at (bank,page_in_bank).
    /// Safety: `base` must point to at least 4096 readable bytes for this page.
    #[inline]
    pub unsafe fn map_rom_page(&mut self, bank: u8, page_in_bank: u8, base: *const u8) {
        let page = ((bank as usize) << (16 - PAGE_SHIFT)) | (page_in_bank as usize);
        self.pages[page] = Page {
            ptr: base as *mut u8,
            io: None,
            io_len: 0,
            kind: PageKind::Rom,
        };
    }

    /// Safe wrapper: map a 4 KiB RAM page from a fixed-size slice.
    ///
    /// This is zero-cost after inlining; it simply forwards the slice's pointer
    /// to the unsafe mapping function while preserving Rust's lifetime/aliasing
    /// guarantees at the call site.
    #[inline(always)]
    pub fn map_ram_page_from_slice(
        &mut self,
        bank: u8,
        page_in_bank: u8,
        slice: &mut [u8; PAGE_SIZE],
    ) {
        let ptr = slice.as_mut_ptr();
        // Safety: &[u8; PAGE_SIZE] guarantees a contiguous 4 KiB region valid for writes.
        unsafe { self.map_ram_page(bank, page_in_bank, ptr) }
    }

    /// Safe wrapper: map a 4 KiB ROM page from a fixed-size slice.
    #[inline(always)]
    pub fn map_rom_page_from_slice(&mut self, bank: u8, page_in_bank: u8, slice: &[u8; PAGE_SIZE]) {
        let ptr = slice.as_ptr();
        // Safety: &[u8; PAGE_SIZE] guarantees a contiguous 4 KiB region valid for reads.
        unsafe { self.map_rom_page(bank, page_in_bank, ptr) }
    }

    /// Map a 4 KiB page to an IO slot. Safety: handler must be a valid pointer.
    #[inline]
    pub unsafe fn map_io_page(&mut self, bank: u8, page_in_bank: u8, handler: *mut dyn IoHandler) {
        let page = ((bank as usize) << (16 - PAGE_SHIFT)) | (page_in_bank as usize);
        self.pages[page] = Page {
            ptr: core::ptr::null_mut(),
            io: Some(unsafe { NonNull::new_unchecked(handler) }),
            io_len: 0,
            kind: PageKind::Io,
        };
    }

    /// Map a 4 KiB page as RAM with an IO prefix.
    ///
    /// Bytes in `[0, io_len)` within this page are handled via `handler`; the rest
    /// are treated as normal RAM backed by `base`.
    ///
    /// Safety:
    /// - `base` must point to at least 4096 writable bytes for this page.
    /// - `handler` must remain valid and outlive this `FastMapBus`.
    /// - The caller must ensure no aliasing violations when using the handler.
    #[inline]
    pub unsafe fn map_ram_io_prefix_page(
        &mut self,
        bank: u8,
        page_in_bank: u8,
        base: *mut u8,
        handler: *mut dyn IoHandler,
        io_len: u16,
    ) {
        debug_assert!(io_len as usize <= PAGE_SIZE);
        let page = ((bank as usize) << (16 - PAGE_SHIFT)) | (page_in_bank as usize);
        self.pages[page] = Page {
            ptr: base,
            io: Some(unsafe { NonNull::new_unchecked(handler) }),
            io_len,
            kind: PageKind::RamIo,
        };
    }

    /// Map a 4 KiB page as ROM with an IO prefix.
    ///
    /// Bytes in `[0, io_len)` within this page are handled via `handler`; the rest
    /// are treated as normal ROM backed by `base`.
    ///
    /// Safety: same as `map_ram_io_prefix_page`, but `base` need only be readable.
    #[inline]
    pub unsafe fn map_rom_io_prefix_page(
        &mut self,
        bank: u8,
        page_in_bank: u8,
        base: *const u8,
        handler: *mut dyn IoHandler,
        io_len: u16,
    ) {
        debug_assert!(io_len as usize <= PAGE_SIZE);
        let page = ((bank as usize) << (16 - PAGE_SHIFT)) | (page_in_bank as usize);
        self.pages[page] = Page {
            ptr: base as *mut u8,
            io: Some(unsafe { NonNull::new_unchecked(handler) }),
            io_len,
            kind: PageKind::RomIo,
        };
    }

    /// Unmap a page to open bus.
    #[inline]
    pub fn map_open_page(&mut self, bank: u8, page_in_bank: u8) {
        let page = ((bank as usize) << (16 - PAGE_SHIFT)) | (page_in_bank as usize);
        self.pages[page] = Page::default();
    }

    /// Set the input lines for the next sample.
    #[inline(always)]
    pub fn set_lines(&mut self, lines: Lines) {
        self.lines = lines;
    }

    /// Convenience setters
    #[inline(always)]
    pub fn set_irq(&mut self, v: bool) {
        self.lines.irq = v;
    }
    #[inline(always)]
    pub fn set_nmi(&mut self, v: bool) {
        self.lines.nmi = v;
    }
    #[inline(always)]
    pub fn set_abort(&mut self, v: bool) {
        self.lines.abort_ = v;
    }
    #[inline(always)]
    pub fn set_rdy(&mut self, v: bool) {
        self.lines.rdy = v;
    }
    #[inline(always)]
    pub fn set_be(&mut self, v: bool) {
        self.lines.be = v;
    }
    #[inline(always)]
    pub fn open_bus_value(&self) -> u8 {
        self.open.last
    }
}

impl Default for FastMapBus {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus for FastMapBus {
    #[inline(always)]
    fn read(&mut self, addr: u32, vda: bool, vpa: bool) -> u8 {
        debug_assert!(addr >> 24 == 0, "address out of 24-bit range");
        let p = unsafe { *self.pages.get_unchecked(Self::page_index(addr)) }; // copy Page to avoid holding an & borrow
        let off = (addr & PAGE_MASK) as usize;
        match p.kind {
            PageKind::Ram => unsafe {
                let v = core::ptr::read(p.ptr.add(off));
                self.open.drive(v);
                v
            },
            PageKind::Rom => unsafe {
                let v = core::ptr::read(p.ptr.add(off));
                self.open.drive(v);
                v
            },
            PageKind::RamIo | PageKind::RomIo => self.read_overlay(p, addr, vda, vpa),
            _ => self.read_slow(p, addr, vda, vpa),
        }
    }

    #[inline(always)]
    fn write(&mut self, addr: u32, data: u8, vda: bool, vpa: bool) {
        debug_assert!(addr >> 24 == 0, "address out of 24-bit range");
        let idx = Self::page_index(addr);
        let p = unsafe { *self.pages.get_unchecked(idx) }; // copy once
        let off = (addr & PAGE_MASK) as usize;
        match p.kind {
            PageKind::Ram => unsafe {
                core::ptr::write(p.ptr.add(off), data);
                self.open.drive(data);
            },
            PageKind::Rom => {
                // Writes to ROM ignored but still drive open-bus
                self.open.drive(data);
            }
            PageKind::RamIo | PageKind::RomIo => self.write_overlay(p, addr, data, vda, vpa),
            _ => self.write_slow(p, addr, data, vda, vpa),
        }
    }

    #[inline(always)]
    fn sample_lines(&mut self) -> Lines {
        self.lines
    }
}

impl FastMapBus {
    #[inline(never)]
    #[cold]
    fn read_overlay(&mut self, p: Page, addr: u32, vda: bool, vpa: bool) -> u8 {
        let off = (addr & PAGE_MASK) as u16;
        if off < p.io_len {
            debug_assert!(p.io.is_some(), "RamIo/RomIo page without handler");
            let mut nn = p.io.unwrap();
            let handler: &mut dyn IoHandler = unsafe { nn.as_mut() };
            let v = handler.read(addr, vda, vpa);
            self.open.drive(v);
            v
        } else {
            debug_assert!(!p.ptr.is_null(), "RamIo/RomIo page without backing ptr");
            unsafe {
                let v = core::ptr::read(p.ptr.add(off as usize));
                self.open.drive(v);
                v
            }
        }
    }

    #[inline(never)]
    #[cold]
    fn write_overlay(&mut self, p: Page, addr: u32, data: u8, vda: bool, vpa: bool) {
        let off = (addr & PAGE_MASK) as u16;
        if off < p.io_len {
            debug_assert!(p.io.is_some(), "RamIo page without handler");
            let mut nn = p.io.unwrap();
            let handler: &mut dyn IoHandler = unsafe { nn.as_mut() };
            handler.write(addr, data, vda, vpa);
            self.open.drive(data);
        } else {
            match p.kind {
                PageKind::RamIo => {
                    debug_assert!(!p.ptr.is_null(), "RamIo page without backing ptr");
                    unsafe {
                        core::ptr::write(p.ptr.add(off as usize), data);
                    }
                    self.open.drive(data);
                }
                PageKind::RomIo => {
                    // writes to ROM portion ignored but still drive open bus
                    self.open.drive(data);
                }
                _ => unreachable!(),
            }
        }
    }

    #[inline(never)]
    #[cold]
    fn read_slow(&mut self, p: Page, addr: u32, vda: bool, vpa: bool) -> u8 {
        match p.kind {
            PageKind::Io => {
                debug_assert!(p.io.is_some(), "Io page without handler");
                let mut nn = p.io.unwrap();
                let handler: &mut dyn IoHandler = unsafe { nn.as_mut() };
                let v = handler.read(addr, vda, vpa);
                self.open.drive(v);
                v
            }
            PageKind::Open => self.open.sample(),
            _ => unreachable!(),
        }
    }

    #[inline(never)]
    #[cold]
    fn write_slow(&mut self, p: Page, addr: u32, data: u8, vda: bool, vpa: bool) {
        match p.kind {
            PageKind::Io => {
                debug_assert!(p.io.is_some(), "Io page without handler");
                let mut nn = p.io.unwrap();
                let handler: &mut dyn IoHandler = unsafe { nn.as_mut() };
                handler.write(addr, data, vda, vpa);
                self.open.drive(data);
            }
            PageKind::Open => {
                self.open.drive(data);
            }
            _ => unreachable!(),
        }
    }
}
