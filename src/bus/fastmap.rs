//! Fast, page-mapped bus for 65x cores.
//!
//! Goal: O(1) RAM/ROM access using a page table.
//! This bus intentionally does not model IO dispatch; clients should intercept IO ranges and
//! handle side effects externally, then delegate pure memory to FastMapBus.

use super::{Bus, Lines, OpenBus};

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum PageKind {
    Open = 0,
    Ram = 1,
    Rom = 2,
}

/// Compact per-page mapping:  
/// - `ptr`: base pointer for RAM/ROM; unused for Open  
/// - `kind`: tag
#[derive(Clone, Copy)]
struct Page {
    ptr: *mut u8,
    kind: PageKind,
}

impl Default for Page {
    #[inline(always)]
    fn default() -> Self {
        Self {
            ptr: core::ptr::null_mut(),
            kind: PageKind::Open,
        }
    }
}

/// FastMapBus page table: heap-allocated, fixed-size after construction (never reallocates).
pub struct FastMapBus<const ADDR_BITS: u32, const PAGE_SHIFT: u32> {
    /// Page table is heap-allocated and never resizes/reallocates after construction.
    pages: Box<[Page]>,
    num_pages: usize,
    lines: Lines,
    open: OpenBus,
}

// Common instantiations
pub type FastMapBus24 = FastMapBus<24, 12>;
pub type FastMapBus16 = FastMapBus<16, 12>;
pub type FastMapBus14 = FastMapBus<14, 12>;

impl<const ADDR_BITS: u32, const PAGE_SHIFT: u32> FastMapBus<ADDR_BITS, PAGE_SHIFT> {
    #[inline]
    pub fn new() -> Self {
        debug_assert!(
            ADDR_BITS <= 24,
            "FastMapBus expects <= 24-bit addressing in this core"
        );
        debug_assert!(PAGE_SHIFT < ADDR_BITS, "PAGE_SHIFT must be < ADDR_BITS");

        let num_pages = 1usize << (ADDR_BITS - PAGE_SHIFT);

        // One-time allocation. `Box<[Page]>` is fixed-size and can never reallocate.
        let pages: Box<[Page]> = vec![Page::default(); num_pages].into_boxed_slice();

        Self {
            pages,
            num_pages,
            lines: Lines::none(),
            open: OpenBus::default(),
        }
    }

    const PAGE_SIZE: usize = 1usize << PAGE_SHIFT;
    const PAGE_MASK: u32 = (Self::PAGE_SIZE as u32) - 1;

    #[inline(always)]
    const fn addr_mask() -> u32 {
        // Mask for the configured address width. In debug we also assert the address range
        // at the call sites; this mask prevents accidental OOB indexing in release.
        if ADDR_BITS == 32 {
            u32::MAX
        } else {
            (1u32 << ADDR_BITS) - 1
        }
    }

    /// Compute the page index for an address in this bus' address space.
    #[inline(always)]
    fn page_index(addr: u32) -> usize {
        // Masking keeps accidental high bits from indexing past the table in release builds.
        // The debug_asserts in read/write still enforce correctness.
        let a = addr & Self::addr_mask();
        (a >> PAGE_SHIFT) as usize
    }

    /// Map a single page of RAM by page index.
    /// Safe: `slice` is bounds-checked to exactly one page.
    #[inline(always)]
    pub fn map_ram_page_idx(&mut self, page_idx: usize, slice: &mut [u8]) {
        debug_assert_eq!(slice.len(), Self::PAGE_SIZE);
        debug_assert!(page_idx < self.num_pages);
        let ptr = slice.as_mut_ptr();
        self.pages[page_idx] = Page {
            ptr,
            kind: PageKind::Ram,
        };
    }

    /// Map a single page of ROM by page index.
    #[inline(always)]
    pub fn map_rom_page_idx(&mut self, page_idx: usize, slice: &[u8]) {
        debug_assert_eq!(slice.len(), Self::PAGE_SIZE);
        debug_assert!(page_idx < self.num_pages);
        let ptr = slice.as_ptr() as *mut u8;
        self.pages[page_idx] = Page {
            ptr,
            kind: PageKind::Rom,
        };
    }

    /// Unmap a page to open bus by page index.
    #[inline(always)]
    pub fn map_open_page_idx(&mut self, page_idx: usize) {
        debug_assert!(page_idx < self.num_pages);
        self.pages[page_idx] = Page::default();
    }

    /// Map by address (convenience). The address is interpreted in this bus' address space.
    #[inline(always)]
    pub fn map_ram_page_at(&mut self, addr: u32, slice: &mut [u8]) {
        let idx = Self::page_index(addr);
        self.map_ram_page_idx(idx, slice);
    }

    #[inline(always)]
    pub fn map_rom_page_at(&mut self, addr: u32, slice: &[u8]) {
        let idx = Self::page_index(addr);
        self.map_rom_page_idx(idx, slice);
    }

    #[inline(always)]
    pub fn map_open_page_at(&mut self, addr: u32) {
        let idx = Self::page_index(addr);
        self.map_open_page_idx(idx);
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

impl<const ADDR_BITS: u32, const PAGE_SHIFT: u32> Default for FastMapBus<ADDR_BITS, PAGE_SHIFT> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const ADDR_BITS: u32, const PAGE_SHIFT: u32> Bus for FastMapBus<ADDR_BITS, PAGE_SHIFT> {
    #[inline(always)]
    fn read(&mut self, addr: u32, _vda: bool, _vpa: bool) -> u8 {
        debug_assert!(addr < (1u32 << ADDR_BITS), "address out of range");
        let p = unsafe { *self.pages.get_unchecked(Self::page_index(addr)) }; // copy Page to avoid holding an & borrow
        let off = (addr & Self::PAGE_MASK) as usize;
        match p.kind {
            PageKind::Ram | PageKind::Rom => unsafe {
                let v = core::ptr::read(p.ptr.add(off));
                self.open.drive(v);
                v
            },
            PageKind::Open => self.open.sample(),
        }
    }

    #[inline(always)]
    fn write(&mut self, addr: u32, data: u8, _vda: bool, _vpa: bool) {
        debug_assert!(addr < (1u32 << ADDR_BITS), "address out of range");
        let p = unsafe { *self.pages.get_unchecked(Self::page_index(addr)) }; // copy once
        let off = (addr & Self::PAGE_MASK) as usize;
        match p.kind {
            PageKind::Ram => unsafe {
                core::ptr::write(p.ptr.add(off), data);
                self.open.drive(data);
            },
            PageKind::Rom => {
                // Writes to ROM ignored but still drive open-bus
                self.open.drive(data);
            }
            PageKind::Open => {
                self.open.drive(data);
            }
        }
    }

    #[inline(always)]
    fn sample_lines(&mut self) -> Lines {
        self.lines
    }
}
