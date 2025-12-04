use crate::{
    arch::{self, tlbi_vaee1},
    dsb, isb,
    pm::{GB, KB, MB},
    print, tlbi_vmalle1,
};
use core::{arch::asm, cell::UnsafeCell, ptr::slice_from_raw_parts_mut};
use core::{cell::OnceCell, fmt::Display, hash::Hash, ptr::NonNull};

use crate::{
    pm::{self, align_b, align_f},
    spin::Lock,
};

#[repr(align(4096))]
struct Table {
    data: [u64; 512],
}

impl Table {
    const fn new() -> Table {
        Table { data: [0; 512] }
    }
}

#[repr(align(4096))]
struct Table2 {
    inner: UnsafeCell<Table>,
}

impl Table2 {
    const fn new() -> Table2 {
        Table2 {
            inner: UnsafeCell::new(Table::new()),
        }
    }
}

unsafe impl Sync for Table2 {}

const VOFFT: u64 = 0xffff_0000_0000_0000;

static PT: Lock<Table> = Lock::new("vm", Table::new());

static DPT_L1: Table2 = Table2::new();
static DPT_L2: Table2 = Table2::new();
static DPT_L3: Table2 = Table2::new();

#[unsafe(no_mangle)]
pub fn wrap(p: usize) -> usize {
    let v = alloc_4k_direct().unwrap();
    let vaddr = Vaddr::new(v);
    unsafe { DPT_L3.inner.get().as_mut() }.unwrap().data[vaddr.l3() as usize] = p as u64 | 0x403;
    tlbi_vaee1(v as u64);
    dsb!();
    isb!();
    v
}

fn zero_pt(ptr: *mut u64) {
    for i in 0..512 {
        unsafe { ptr.add(i).write(0) }
    }
}

fn pt_alloc_if_0(idx: usize, pt: &mut [u64]) -> &mut [u64] {
    assert!(pt.len() == 512);
    let mut nxt_pt = pt[idx];
    if nxt_pt == 0 {
        nxt_pt = match pm::alloc(4096) {
            Some(ptr) => {
                zero_pt((ptr as u64 + VOFFT) as *mut u64);
                ptr as u64 | 3
            }
            None => unreachable!(), /*TODO handle error*/
        };
        pt[idx] = nxt_pt;
    }
    let nxt_pt = unsafe {
        slice_from_raw_parts_mut(
            ((nxt_pt as *mut u64 as usize & !0xfff) + VOFFT as usize) as *mut u64,
            512,
        )
        .as_mut()
    }
    .unwrap();

    nxt_pt
}

#[unsafe(no_mangle)]
/// direct pointer must be freed
fn pt_alloc_if_0_2(idx: usize, pt: &mut [u64]) -> &mut [u64] {
    assert!(pt.len() == 512);
    let mut nxt_pt = pt[idx];
    if nxt_pt == 0 {
        match pm::alloc(4096) {
            Some(ptr) => {
                pt[idx] = nxt_pt as u64 | 3;
                let w = wrap(ptr as usize);
                zero_pt(w as *mut u64);
                nxt_pt = ptr as u64;
                free_4k_direct(w);
            }
            None => unreachable!(), /*TODO handle error*/
        };
    }
    let nxt_pt = unsafe {
        slice_from_raw_parts_mut(wrap(nxt_pt as *mut u64 as usize & !0xfff) as *mut u64, 512)
            .as_mut()
    }
    .unwrap();

    nxt_pt
}

#[unsafe(no_mangle)]
fn use_gb_blocks(l0_pt: &mut [u64], mut k_begin: usize, mut k_end: usize) {
    assert!(l0_pt.len() == 512);
    k_begin = align_b(k_begin, GB);
    k_end = align_f(k_end, GB);

    let mut i = k_begin;

    while i < k_end {
        let vaddr = Vaddr::new(i + VOFFT as usize);
        let l1_pt = pt_alloc_if_0(vaddr.l0() as usize, l0_pt);
        l1_pt[vaddr.l1() as usize] = (i | 0x401) as u64;
        i += GB;
    }
}

fn use_2mb_blocks(l0_pt: &mut [u64], mut k_begin: usize, mut k_end: usize) {
    assert!(l0_pt.len() == 512);
    k_begin = align_b(k_begin, 2 * MB);
    k_end = align_f(k_end, 2 * MB);

    let mut i = k_begin;

    while i < k_end {
        let vaddr = Vaddr::new(i + VOFFT as usize);
        let l1_pt = pt_alloc_if_0(vaddr.l0() as usize, l0_pt);
        let l2_pt = pt_alloc_if_0(vaddr.l1() as usize, l1_pt);
        l2_pt[vaddr.l2() as usize] = (i | 0x401) as u64;
        i += 2 * MB;
    }
}

fn use_4k_blocks(l0_pt: &mut [u64], mut k_begin: usize, mut k_end: usize) {
    assert!(l0_pt.len() == 512);
    k_begin = align_b(k_begin, 4 * KB);
    k_end = align_f(k_end, 4 * KB);

    let mut i = k_begin;

    while i < k_end {
        let vaddr = Vaddr::new(i + VOFFT as usize);
        let l1_pt = pt_alloc_if_0(vaddr.l0() as usize, l0_pt);
        let l2_pt = pt_alloc_if_0(vaddr.l1() as usize, l1_pt);
        let l3_pt = pt_alloc_if_0(vaddr.l2() as usize, l2_pt);
        l3_pt[vaddr.l3() as usize] = (i | 0x403) as u64;
        i += 4 * KB;
    }
}

#[derive(Debug)]
pub struct Error;

#[unsafe(no_mangle)]
fn map_v2p_4k(v: usize, p: usize) -> Result<usize, Error> {
    let pt_lock = PT.acquire();
    let l0_pt = &mut pt_lock.as_mut().data;

    let vaddr = Vaddr::new(v);
    let l1_pt = pt_alloc_if_0_2(vaddr.l0() as usize, l0_pt);
    let l1_pt_w = l1_pt.as_ptr();
    let l2_pt = pt_alloc_if_0_2(vaddr.l1() as usize, l1_pt);
    let l2_pt_w = l2_pt.as_ptr();
    let l3_pt = pt_alloc_if_0_2(vaddr.l2() as usize, l2_pt);
    l3_pt[vaddr.l3() as usize] = (p | 0x403) as u64;

    free_4k_direct(l1_pt_w as usize);
    free_4k_direct(l2_pt_w as usize);
    free_4k_direct(l3_pt.as_ptr() as usize);

    Ok(v)
}

pub fn v2p(v: usize) -> Option<usize> {
    let pt_lock = PT.acquire();
    let l0_pt = &mut pt_lock.as_mut().data;

    let vaddr = Vaddr::new(v);

    let l1_pt = l0_pt[vaddr.l0() as usize];

    if l1_pt == 0 {
        return None;
    }

    let l1_pt = wrap(l1_pt as usize & !0xfff) as *const u64;
    let l2_pt = unsafe { *l1_pt.add(vaddr.l1() as usize) };

    if l2_pt == 0 {
        return None;
    }

    let l2_pt = wrap(l2_pt as usize & !0xfff) as *const u64;
    let l3_pt = unsafe { *l2_pt.add(vaddr.l2() as usize) };

    if l3_pt == 0 {
        return None;
    }

    let l3_pt = wrap(l3_pt as usize & !0xfff) as *const u64;
    let paddr = unsafe { *l3_pt.add(vaddr.l3() as usize) };

    free_4k_direct(l1_pt as usize);
    free_4k_direct(l2_pt as usize);
    free_4k_direct(l3_pt as usize);

    Some((paddr as usize & !0xfff) | (v & 0xfff))
}

pub fn init(k_begin: usize, k_end: usize) {
    let pt_lock = PT.acquire();
    let l0_pt = &mut pt_lock.as_mut().data;

    use_4k_blocks(l0_pt, k_begin, k_end);
    let v = Vaddr::new(!0usize);

    l0_pt[v.l0() as usize] = ((DPT_L1.inner.get() as u64) - VOFFT) | 3;

    unsafe {
        *DPT_L1.inner.get().cast::<u64>().add(v.l1() as usize) =
            ((DPT_L2.inner.get()) as u64 - VOFFT) | 3;
        *DPT_L2.inner.get().cast::<u64>().add(v.l2() as usize) =
            ((DPT_L3.inner.get()) as u64 - VOFFT) | 3
    };
    // WRAP.set(());

    let neg1 = pt_lock.as_ref() as *const Table as u64 - VOFFT;

    dsb!();
    isb!();
    arch::w_ttbr0_el1(0);
    arch::w_ttbr1_el1(neg1);
    tlbi_vmalle1!();
    dsb!();
    isb!();

    pm::free_low(k_begin);
    init_regions(k_end);
}

#[repr(packed)]
pub struct Vaddr {
    back: usize,
}

impl Vaddr {
    #[inline]
    pub fn new(addr: usize) -> Vaddr {
        Vaddr { back: addr }
    }

    #[inline]
    pub fn offt(&self) -> u16 {
        (self.back & 0xfff) as u16
    }

    #[inline]
    pub fn l0(&self) -> u16 {
        ((self.back >> 39) & 0x1ff) as u16
    }

    #[inline]
    pub fn l1(&self) -> u16 {
        ((self.back >> 30) & 0x1ff) as u16
    }

    #[inline]
    pub fn l2(&self) -> u16 {
        ((self.back >> 21) & 0x1ff) as u16
    }

    #[inline]
    pub fn l3(&self) -> u16 {
        ((self.back >> 12) & 0x1ff) as u16
    }
}

impl Display for Vaddr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "[l0: {}, l1: {}, l2: {}, l3: {}, offt: {}]",
            self.l0(),
            self.l1(),
            self.l2(),
            self.l3(),
            self.offt()
        )
    }
}

pub struct Region {
    start: usize,
    bits: u64,
    nxt: Option<NonNull<Region>>,
}

unsafe impl Sync for Region {}

impl Region {
    const fn new(start: usize) -> Region {
        Region {
            start,
            bits: 0,
            nxt: None,
        }
    }

    fn is_full(&self) -> bool {
        self.bits == !0u64
    }

    fn is_uninit(&self) -> bool {
        self.start == 0 && self.nxt.is_none() && self.bits == 0
    }

    fn nxt(&self) -> Option<NonNull<Region>> {
        match self.nxt {
            Some(ptr) => Some(ptr),
            _ => None,
        }
    }

    fn append(&mut self, other: &mut Region) {
        let mut last = self;
        while let Some(mut nxt) = last.nxt() {
            last = unsafe { nxt.as_mut() }
        }
        last.nxt = NonNull::new(other as *mut Region)
    }

    fn alloc(&mut self, n: usize) -> Option<usize> {
        if self.is_full() || self.is_uninit() {
            // TODO append if full
            return None;
        }

        if self.bits.count_zeros() < n as u32 {
            // TODO append if full
            return None;
        }

        let mut found = 0;

        let mut bits = self.bits;
        let mut tmp_bits = self.bits;
        for i in 0..64 {
            if (bits & 1) == 0 {
                tmp_bits |= 1u64 << i;
                found += 1;

                if found == n {
                    self.bits = tmp_bits;
                    return Some(self.start + (i - (n - 1)) * 4 * KB);
                }
            } else {
                found = 0;
            }
            bits >>= 1;
        }

        None
    }

    fn free_inner(&mut self, addr: usize) -> Option<()> {
        if addr >= self.start && addr < (self.start + 64 * 4 * KB) {
            let addr = addr - self.start;
            let bit = addr / (4 * KB);
            assert!((self.bits & (1u64 << bit)) != 0);
            self.bits &= !(1u64 << bit);
            Some(())
        } else {
            None
        }
    }

    #[unsafe(no_mangle)]
    pub fn free_1(&mut self, addr: usize) {
        if self.free_inner(addr).is_some() {
            return;
        }

        let mut last = self;
        while let Some(mut nxt) = last.nxt() {
            let tmp = unsafe { nxt.as_mut() };
            if let Some(_) = tmp.free_inner(addr) {
                return;
            }
            last = tmp;
        }
        unreachable!()
    }
}

static REGIONS: Lock<Region> = Lock::new("vm_regions", Region::new(0));
static DIRECTS: Lock<Region> = Lock::new("vm_regions", Region::new(0));

fn init_regions(start_p: usize) {
    let lock = REGIONS.acquire();
    let first = lock.as_mut();
    first.start = align_f(start_p, 4 * KB) + VOFFT as usize;

    let lock = DIRECTS.acquire();
    let first = lock.as_mut();
    first.start = 0xffff_ffff_fff0_0000;
}

pub fn alloc_4k() -> Option<usize> {
    let lock = REGIONS.acquire();
    lock.as_mut().alloc(1)
}

pub fn alloc(n: usize) -> Option<usize> {
    let lock = REGIONS.acquire();
    lock.as_mut().alloc(n)
}

pub fn free(addr: usize, n: usize) {
    let lock = REGIONS.acquire();
    for i in 0..n {
        lock.as_mut().free_1(addr + (i * 4 * KB));
    }
}

pub fn free_4k(v: usize) {
    let lock = REGIONS.acquire();
    lock.as_mut().free_1(v)
}

pub fn alloc_4k_direct() -> Option<usize> {
    let lock = DIRECTS.acquire();
    lock.as_mut().alloc(1)
}

fn free_4k_direct(v: usize) {
    let lock = DIRECTS.acquire();
    lock.as_mut().free_1(v)
}

pub fn map_4k(p: usize) -> Result<usize, Error> {
    match alloc_4k() {
        Some(v) => map_v2p_4k(v, p),
        _ => Err(Error),
    }
}

pub fn map(p: usize, n: usize) -> Result<usize, Error> {
    match alloc(n) {
        Some(v) => {
            for i in 0..n {
                map_v2p_4k(v + (i * 4 * KB), p + (i * 4 * KB)).unwrap();
            }
            Ok(v)
        }
        _ => Err(Error),
    }
}
