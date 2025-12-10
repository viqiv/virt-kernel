use alloc::vec::Vec;

use crate::{
    _bss_end, _data_end, _rodata_end, _text_end, _user_end,
    arch::{self, tlbi_vaee1},
    dsb, isb,
    pm::{GB, KB, MB},
    print,
    stuff::BitSet128,
    tlbi_vmalle1,
};
use core::{arch::asm, cell::UnsafeCell, ptr::slice_from_raw_parts_mut};
use core::{cell::OnceCell, fmt::Display, hash::Hash, ptr::NonNull};

use crate::{
    pm::{self, align_b, align_f},
    spin::Lock,
};

// Table D8-65 Summary of possible memory access permissions using Direct permissions
// UXN[54] PXN[53] AP[2:1][6:7] WXN[SCTLR_ELx.WXN] Permission
// 0          0          00          0             PrivRead, PrivWrite, PrivExecute, UnprivExecute
// 0          0          00          1             PrivRead, PrivWrite, PrivWXN, UnprivExecute
pub const PR_PW_PX_UX: u64 = 0;
// 0          0          01          0             PrivRead, PrivWrite, UnprivRead, UnprivWrite, UnprivExecute
// 0          0          01          1             PrivRead, PrivWrite, UnprivRead, UnprivWrite, UnprivWXN
pub const PR_PW_UR_UW_UX1: u64 = 0b01 << 6;
// 0          0          10          x             PrivRead, PrivExecute, UnprivExecute
pub const PR_PX_UX: u64 = 0b10 << 6;
// 0          0          11          x             PrivRead, PrivExecute, UnprivRead, UnprivExecute
pub const PR_PX_UR_UX: u64 = 0b11 << 6;
// 0          1          00          x             PrivRead, PrivWrite, UnprivExecute
pub const PR_PW_UX: u64 = 0b1 << 53;
// 0          1          01          0             PrivRead, PrivWrite, UnprivRead, UnprivWrite, UnprivExecute
// 0          1          01          1             PrivRead, PrivWrite, UnprivRead, UnprivWrite, UnprivWXN
pub const PR_PW_UR_UW_UX2: u64 = 0b1 << 53 | 0b01 << 6;
// 0          1          10          x             PrivRead, UnprivExecute
pub const PR_UX: u64 = 0b1 << 53 | 0b10 << 6;
// 0          1          11          x             PrivRead, UnprivRead, UnprivExecute
pub const PR_UR_UX: u64 = 0b1 << 53 | 0b11 << 6;
// 1          0          00          0             PrivRead, PrivWrite, PrivExecute
// 1          0          00          1             PrivRead, PrivWrite, PrivWXN
pub const PR_PW_PX: u64 = 0b1 << 54;
// 1          0          01          x             PrivRead, PrivWrite, UnprivRead, UnprivWrite
pub const PR_PW_UR_UW1: u64 = 0b1 << 54 | 0b01 << 6;
// 1          0          10          x             PrivRead, PrivExecute
pub const PR_PX: u64 = 0b1 << 54 | 0b10 << 6;
// 1          0          11          x             PrivRead, PrivExecute, UnprivRead
pub const PR_PX_UR: u64 = 0b1 << 54 | 0b11 << 6;
// 1          1          00          x             PrivRead, PrivWrite
pub const PR_PW: u64 = 0b1 << 54 | 0b1 << 53;
// 1          1          01          x             PrivRead, PrivWrite, UnprivRead, UnprivWrite
pub const PR_PW_UR_UW2: u64 = 0b1 << 54 | 0b1 << 53 | 0b01 << 6;
// 1          1          10          x             PrivRead
pub const PR: u64 = 0b1 << 54 | 0b1 << 53 | 0b10 << 6;
// 1          1          11          x             PrivRead, UnprivRead
pub const PR_UR: u64 = 0b1 << 54 | 0b1 << 53 | 0b11 << 6;

pub const PHY_MASK: usize = 0x0000_ffff_ffff_f000;

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
    unsafe { DPT_L3.inner.get().as_mut() }.unwrap().data[vaddr.l3() as usize] =
        p as u64 | PR_PW | 0x403;
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
            ((nxt_pt as *mut u64 as usize & PHY_MASK) + VOFFT as usize) as *mut u64,
            512,
        )
        .as_mut()
    }
    .unwrap();

    nxt_pt
}

#[macro_export]
macro_rules! pt_from_u64 {
    ($ptr:expr) => {
        unsafe {
            slice_from_raw_parts_mut(
                wrap($ptr as *mut u64 as usize & $crate::vm::PHY_MASK) as *mut u64,
                512,
            )
            .as_mut()
        }
        .unwrap()
    };
}

/// direct pointer must be freed
pub fn pt_alloc_if_0_2(idx: usize, pt: &mut [u64]) -> &mut [u64] {
    assert!(pt.len() == 512);
    let mut nxt_pt = pt[idx];
    let mut new = false;
    if nxt_pt == 0 {
        match pm::alloc(4096) {
            Some(ptr) => {
                pt[idx] = ptr as u64 | 3;
                nxt_pt = ptr as u64;
                new = true;
            }
            None => unreachable!(), /*TODO handle error*/
        };
    }

    let nxt_pt = pt_from_u64!(nxt_pt);
    if new {
        zero_pt(nxt_pt.as_mut_ptr());
    }
    // let nxt_pt = unsafe {
    //     slice_from_raw_parts_mut(wrap(nxt_pt as *mut u64 as usize & !0xfff) as *mut u64, 512)
    //         .as_mut()
    // }
    // .unwrap();

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

fn region_perms(vaddr: u64) -> u64 {
    let text_end = unsafe { (&_text_end) as *const u64 as u64 };
    let data_end = unsafe { (&_data_end) as *const u64 as u64 };
    let rodata_end = unsafe { (&_rodata_end) as *const u64 as u64 };
    let bss_end = unsafe { (&_bss_end) as *const u64 as u64 };
    let user_end = unsafe { (&_user_end) as *const u64 as u64 };

    if vaddr < text_end {
        return PR_PX;
    } else if vaddr >= text_end && vaddr < data_end {
        return PR_PW;
    } else if vaddr >= data_end && vaddr < rodata_end {
        return PR;
    } else if vaddr >= rodata_end && vaddr < bss_end {
        return PR_PW;
    } else if vaddr >= bss_end && vaddr < user_end {
        // loop {}
        return PR_PW_UR_UW_UX2;
    }

    0
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
        l3_pt[vaddr.l3() as usize] = (i as u64 | region_perms(i as u64 + VOFFT) | 0x403) as u64;
        i += 4 * KB;
    }
}

#[derive(Debug)]
pub struct Error;

pub fn map_v2p_4k_inner(l0_pt: &mut [u64], v: usize, p: usize, perms: u64) -> Result<usize, Error> {
    let vaddr = Vaddr::new(v);
    let l1_pt = pt_alloc_if_0_2(vaddr.l0() as usize, l0_pt);
    let l1_pt_w = l1_pt.as_ptr();
    let l2_pt = pt_alloc_if_0_2(vaddr.l1() as usize, l1_pt);
    let l2_pt_w = l2_pt.as_ptr();
    let l3_pt = pt_alloc_if_0_2(vaddr.l2() as usize, l2_pt);

    if l3_pt[vaddr.l3() as usize] != 0 {
        return Err(Error {});
    }

    l3_pt[vaddr.l3() as usize] = (p as u64 | perms | 0x403) as u64;

    free_4k_direct(l1_pt_w as usize);
    free_4k_direct(l2_pt_w as usize);
    free_4k_direct(l3_pt.as_ptr() as usize);

    // print!("va {:x} pa = {:x}\n", v, p);
    tlbi_vaee1(v as u64);
    Ok(v)
}

#[unsafe(no_mangle)]
fn map_v2p_4k(v: usize, p: usize, perms: u64) -> Result<usize, Error> {
    let pt_lock = PT.acquire();
    let l0_pt = &mut pt_lock.as_mut().data;

    map_v2p_4k_inner(l0_pt, v, p, perms)

    // let vaddr = Vaddr::new(v);
    // let l1_pt = pt_alloc_if_0_2(vaddr.l0() as usize, l0_pt);
    // let l1_pt_w = l1_pt.as_ptr();
    // let l2_pt = pt_alloc_if_0_2(vaddr.l1() as usize, l1_pt);
    // let l2_pt_w = l2_pt.as_ptr();
    // let l3_pt = pt_alloc_if_0_2(vaddr.l2() as usize, l2_pt);
    // l3_pt[vaddr.l3() as usize] = (p as u64 | perms | 0x403) as u64;

    // free_4k_direct(l1_pt_w as usize);
    // free_4k_direct(l2_pt_w as usize);
    // free_4k_direct(l3_pt.as_ptr() as usize);

    // Ok(v)
}

#[unsafe(no_mangle)]
fn map_v2p_4k2(v: usize, p: usize, perms: u64) -> Result<usize, Error> {
    let pt_lock = PT.acquire();
    let l0_pt = &mut pt_lock.as_mut().data;

    map_v2p_4k_inner(l0_pt, v, p, perms)
}

pub fn v2p(v: usize) -> Option<usize> {
    let pt_lock = PT.acquire();
    let l0_pt = &mut pt_lock.as_mut().data;

    let vaddr = Vaddr::new(v);

    let l1_pt = l0_pt[vaddr.l0() as usize];

    if l1_pt == 0 {
        return None;
    }

    let l1_pt = wrap(l1_pt as usize & PHY_MASK) as *const u64;
    let l2_pt = unsafe { *l1_pt.add(vaddr.l1() as usize) };

    if l2_pt == 0 {
        return None;
    }

    let l2_pt = wrap(l2_pt as usize & PHY_MASK) as *const u64;
    let l3_pt = unsafe { *l2_pt.add(vaddr.l2() as usize) };

    if l3_pt == 0 {
        return None;
    }

    let l3_pt = wrap(l3_pt as usize & PHY_MASK) as *const u64;
    let paddr = unsafe { *l3_pt.add(vaddr.l3() as usize) };

    free_4k_direct(l1_pt as usize);
    free_4k_direct(l2_pt as usize);
    free_4k_direct(l3_pt as usize);

    Some((paddr as usize & PHY_MASK) | (v & 0xfff))
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
    bs: BitSet128,
    nxt: Option<NonNull<Region>>,
}

unsafe impl Sync for Region {}

impl Region {
    const fn new(start: usize) -> Region {
        Region {
            start,
            bs: BitSet128::new(128),
            nxt: None,
        }
    }

    fn is_full(&self) -> bool {
        self.bs.full()
    }

    // fn is_uninit(&self) -> bool {
    //     self.start == 0 && self.nxt.is_none() && self.bits == 0
    // }

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
        if self.is_full() || n != 1 {
            // TODO append if full
            return None;
        }

        match self.bs.first_clr() {
            Some(i) => {
                self.bs.set(i);
                Some(i as usize * 4096 + self.start)
            }
            _ => None,
        }
    }

    fn free_inner(&mut self, addr: usize) -> Option<()> {
        if addr >= self.start {
            let local = addr - self.start;
            let bit = local / (4096);

            if bit >= 128 {
                return None;
            }

            tlbi_vaee1(addr as u64);
            assert!(self.bs.tst(bit as u8));
            self.bs.set(bit as u8);
            Some(())
        } else {
            None
        }
    }

    pub fn free_1(&mut self, addr: usize) {
        if self.free_inner(addr).is_some() {
            return;
        }

        // let mut last = self;
        // while let Some(mut nxt) = last.nxt() {
        //     let tmp = unsafe { nxt.as_mut() };
        //     if let Some(_) = tmp.free_inner(addr) {
        //         return;
        //     }
        //     last = tmp;
        // }
        print!("addr: 0x{:x} self.start: 0x{:x}\n", addr, self.start);
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

pub fn free_4k_direct(v: usize) {
    let lock = DIRECTS.acquire();
    lock.as_mut().free_1(v)
}

pub fn map_4k(p: usize, perms: u64) -> Result<usize, Error> {
    match alloc_4k() {
        Some(v) => map_v2p_4k(v, p, perms),
        _ => Err(Error),
    }
}

pub fn map(p: usize, n: usize, perms: u64) -> Result<usize, Error> {
    match alloc(n) {
        Some(v) => {
            for i in 0..n {
                map_v2p_4k(v + (i * 4 * KB), p + (i * 4 * KB), perms).unwrap();
            }
            Ok(v)
        }
        _ => Err(Error),
    }
}

pub fn map2(p: usize, n: usize, perms: u64) -> Result<usize, Error> {
    match alloc(n) {
        Some(v) => {
            for i in 0..n {
                map_v2p_4k2(v + (i * 4 * KB), p + (i * 4 * KB), perms).unwrap();
            }
            Ok(v)
        }
        _ => Err(Error),
    }
}
