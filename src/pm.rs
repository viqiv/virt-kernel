// use std::{fs::File, io::Read, mem::size_of, ptr::NonNull};

use core::{cell::UnsafeCell, ptr::NonNull};

use crate::{print, spin::Lock};

pub const MB: usize = 1024 * 1024;
pub const GB: usize = 1024 * 1024 * 1024;
pub const KB: usize = 1024;

pub fn align_b(n: usize, t: usize) -> usize {
    n & !t.wrapping_sub(1)
}

pub fn align_f(n: usize, t: usize) -> usize {
    align_b(n.wrapping_sub(1), t).wrapping_add(t)
}

#[derive(Debug, Clone, Copy)]
#[repr(u32)]
pub enum Flags {
    None,
    Used,
    Cow,
    Mid,
}

#[derive(Debug, Clone, Copy)]
pub struct Page {
    pub idx: usize,
    ord: usize,
    pub ref_cnt: usize,
    next: Option<NonNull<Page>>,
    magic: u32,
    pub flags: Flags,
}

const MAGIC: u32 = 0xDEADBEEF;

impl Page {
    pub const fn new(idx: usize, ord: usize, rc: usize) -> Page {
        Page {
            idx,
            ref_cnt: rc,
            ord,
            next: None,
            magic: MAGIC,
            flags: Flags::None,
        }
    }

    fn assert_ok(&self) {
        if self.magic != MAGIC {
            panic!("wrong magic")
        }
    }

    fn rm_links(&mut self) {
        self.next = None;
    }

    fn split(&mut self, order: usize, idx: usize, a: &mut Allocator) {
        assert!(order < 8);
        let len = (4096 << (Allocator::ORDER - order)) / 2;
        let p_idx_1 = self.idx;
        let p_idx_2 = self.idx + (len / 4096);
        a.free_lists[order].remove(self);
        let ptr1 = unsafe { a.page_ptr.add(p_idx_1) };
        let ptr2 = unsafe { a.page_ptr.add(p_idx_2) };
        let ptr1_r = unsafe { ptr1.as_mut() }.unwrap();
        let ptr2_r = unsafe { ptr2.as_mut() }.unwrap();
        ptr2_r.assert_ok();
        ptr1_r.assert_ok();
        assert!(ptr1_r.idx == p_idx_1);
        assert!(ptr2_r.idx == p_idx_2);
        ptr1_r.ord = order + 1;
        ptr2_r.ord = order + 1;
        a.free_lists[order + 1].add(ptr2);
        a.free_lists[order + 1].add(ptr1);
    }

    fn is_idle(&self) -> bool {
        self.ref_cnt == 0
    }

    fn join(&mut self, alloc: &mut Allocator) {
        let mut page = self;

        loop {
            if page.ord == 0 {
                return alloc.free_lists[page.ord].add(page);
            }
            let addr = page.idx * 4096;
            let b_idx = Allocator::get_buddy(addr, page.ord) / 4096;
            let buddy_ptr = unsafe { alloc.page_ptr.add(b_idx) };
            let buddy = unsafe { buddy_ptr.as_mut() }.unwrap();
            if buddy.ord != page.ord || !buddy.is_idle() {
                return alloc.free_lists[page.ord].add(page);
            }

            alloc.free_lists[page.ord].remove(buddy);
            let merg = if page.idx > buddy.idx { buddy } else { page };
            merg.ord -= 1;
            page = merg;
        }
    }

    pub fn dup_for_cow(&mut self) {
        self.assert_ok();
        assert!(self.ref_cnt > 0);
        self.ref_cnt += 1;
        self.flags = Flags::Cow;
    }

    pub fn len(&self) -> usize {
        4096 << (8 - self.ord)
    }

    pub fn mark_mids(&mut self) {
        let ptr = self as *mut Page;
        let n = self.len() / 4096;
        for i in 1..n {
            unsafe {
                let mid = ptr.add(i).as_mut().unwrap();
                mid.ref_cnt = i;
                mid.flags = Flags::Mid;
            }
        }
    }

    pub fn unmark_mids(&mut self) {
        let ptr = self as *mut Page;
        let n = self.len() / 4096;
        for i in 1..n {
            unsafe {
                let mid = ptr.add(i).as_mut().unwrap();
                mid.ref_cnt = 0;
                mid.flags = Flags::None;
            }
        }
    }

    pub fn get_head(&mut self) -> Option<&'static mut Page> {
        if let Flags::Mid = self.flags {
            unsafe { (self as *mut Page).sub(self.ref_cnt).as_mut() }
        } else {
            None
        }
    }

    pub fn eql(&self, other: &Page) -> bool {
        self as *const Page == other as *const Page
    }
}

impl Default for Page {
    fn default() -> Self {
        Self {
            idx: 0,
            ref_cnt: 0,
            ord: 0,
            next: None,
            magic: MAGIC,
            flags: Flags::None,
        }
    }
}

#[derive(Default, Clone, Copy)]
struct FL {
    head: Option<NonNull<Page>>,
}

impl FL {
    fn remove(&mut self, node: &mut Page) {
        assert!(self.head.is_some());
        let mut h = unsafe { self.head.unwrap().as_mut() };
        if h.idx == node.idx {
            self.head = h.next;
            h.next = None;
            return;
        }

        while let Some(mut nxt) = h.next {
            let nxt = unsafe { nxt.as_mut() };
            if nxt.idx == node.idx {
                h.next = node.next;
                node.next = None;
                return;
            }
            h = nxt;
        }

        unreachable!()
    }

    fn add(&mut self, node: *mut Page) {
        let node = unsafe { node.as_mut().unwrap() };
        node.next = self.head;
        self.head = NonNull::new(node as *mut Page);
    }

    fn rm_first(&mut self) -> Option<*mut Page> {
        match self.head {
            Some(mut h) => {
                let h = unsafe { h.as_mut() };
                self.head = h.next;
                Some(h)
            }
            _ => None,
        }
    }

    fn get_head(&self) -> Option<*mut Page> {
        match self.head {
            Some(mut h) => Some(unsafe { h.as_mut() }),
            _ => None,
        }
    }

    fn print_list(&self) {
        if self.head.is_none() {
            return;
        }
        let mut ptr = Some(unsafe { self.head.unwrap().as_ref() });
        while let Some(p) = ptr {
            print!("{:?}\n", p);
            ptr = match p.next {
                Some(n) => Some(unsafe { n.as_ref() }),
                _ => None,
            };
        }
    }
}

pub struct Allocator {
    free_lists: [FL; 9],
    page_ptr: *mut Page,
    offt: usize,
    size: usize,
    // meta_size: usize,
}

struct Pages {
    p: UnsafeCell<[Page; GB / 4096]>,
}

impl Pages {
    pub const fn new() -> Pages {
        Pages {
            p: UnsafeCell::new([Page::new(0, 0, 0); GB / 4096]),
        }
    }
}

unsafe impl Sync for Pages {}
unsafe impl Sync for Allocator {}

static PAGES: Pages = Pages::new();
static ALLOC: Lock<Allocator> = Lock::new("pm", Allocator::new());

impl Allocator {
    const ORDER: usize = 8;

    // ptr = 0xffff_0000_4000_0000
    // len = 1gb (hardcoded)
    pub const fn new() -> Allocator {
        Allocator {
            free_lists: [FL { head: None }; 9],
            page_ptr: 0 as *mut Page,
            offt: 0,
            size: 0,
        }
    }

    pub fn init(&mut self, k_begin: usize, k_end: usize) {
        let npages = GB / 4096;
        let page_ptr = PAGES.p.get() as *mut Page;
        for i in 0..npages {
            unsafe {
                *page_ptr.add(i) = Page::new(i, 8, 1);
            }
        }
        *self = Allocator {
            free_lists: Default::default(),
            page_ptr,
            offt: 0x4000_0000, /*hardcode*/
            size: GB,
        };
        self.init_free_list(k_begin, k_end);
    }

    fn init_free_list(&mut self, _k_begin: usize, k_end: usize) {
        let mut i = align_f(k_end, 4 * KB);
        let ram_end = self.offt + GB;
        while i < ram_end {
            self.free(i);
            i += 4 * KB;
        }
    }

    fn init_free_list2(&mut self, k_begin: usize) {
        let mut i = self.offt;
        let ram_end = align_b(k_begin, 4 * KB);
        while i < ram_end {
            self.free(i);
            i += 4 * KB;
        }
    }

    fn get_ord(n: usize) -> usize {
        8 - (align_f(n, 4096).next_power_of_two().ilog2() - 12) as usize
    }

    fn split_to(&mut self, page: &mut Page, mut cur_ord: usize, t_ord: usize) {
        assert!(t_ord > cur_ord);
        let mut page_tmp = page;
        let oc = cur_ord;
        while t_ord > cur_ord {
            page_tmp.split(cur_ord, 0, self);
            page_tmp = unsafe {
                self.free_lists[cur_ord + 1]
                    .get_head()
                    .unwrap()
                    .as_mut()
                    .unwrap()
            };
            cur_ord += 1;
        }
        assert!(oc != cur_ord);
    }

    fn _alloc(&mut self, n: usize) -> Option<*mut u8> {
        if n > MB || n == 0 || self.size == 0 {
            return None;
        }

        let ord = Self::get_ord(n) as usize;

        if let Some(p) = self.free_lists[ord].rm_first() {
            let p = unsafe { p.as_mut() }.unwrap();
            p.assert_ok();
            p.ref_cnt = 1;
            p.rm_links();
            return Some((p.idx * 4096) as *mut u8);
        }

        if ord == 0 {
            return None;
        }

        let mut i = ord - 1;
        loop {
            if let Some(p) = self.free_lists[i].get_head() {
                let p = unsafe { p.as_mut().unwrap() };
                p.assert_ok();
                self.split_to(p, i, ord);
                break;
            }
            if i == 0 {
                break;
            }
            i -= 1;
        }

        if let Some(p) = self.free_lists[ord].rm_first() {
            let p = unsafe { p.as_mut() }.unwrap();
            p.assert_ok();
            p.ref_cnt = 1;
            p.rm_links();
            return Some((p.idx * 4096) as *mut u8);
        } else {
            None
        }
    }

    fn alloc(&mut self, n: usize) -> Option<usize> {
        match self._alloc(n) {
            Some(n) => {
                let page = self.lookup(n as usize + self.offt).unwrap();
                page.mark_mids();
                assert!(page.ref_cnt == 1);
                page.flags = Flags::Used;
                Some(n as usize + self.offt)
            }
            None => None,
        }
    }

    pub fn idx2addr(&mut self, idx: usize) -> usize {
        idx * 4096 + self.offt
    }

    fn get_buddy(addr: usize, ord: usize) -> usize {
        addr ^ (4096 << (Allocator::ORDER - ord))
    }

    fn lookup(&self, addr: usize) -> Option<&'static mut Page> {
        if addr < self.offt {
            return None;
        }
        let addr = (addr as usize) - self.offt;
        if addr >= self.size {
            return None;
        }
        let idx = addr / 4096;
        unsafe { self.page_ptr.add(idx).as_mut() }
    }

    fn free(&mut self, addr: usize) {
        let page = self.lookup(addr).unwrap();
        page.assert_ok();
        if let Flags::Mid = page.flags {
            return;
        }
        assert!(page.ref_cnt > 0);
        page.ref_cnt -= 1;
        if page.ref_cnt > 0 {
            return;
        }
        page.unmark_mids();
        page.flags = Flags::None;
        page.join(self);
    }
}

pub fn alloc(n: usize) -> Result<usize, ()> {
    let lock = ALLOC.acquire();
    if let Some(p) = lock.as_mut().alloc(n) {
        Ok(p)
    } else {
        Err(())
    }
}

pub fn free(addr: usize) {
    let lock = ALLOC.acquire();
    lock.as_mut().free(addr);
}

pub fn lookup(addr: usize) -> Option<&'static mut Page> {
    let lock = ALLOC.acquire();
    lock.as_mut().lookup(addr)
}

pub fn init(k_begin: usize, k_end: usize) {
    let lock = ALLOC.acquire();
    lock.as_mut().init(k_begin, k_end);
}

pub fn free_low(k_begin: usize) {
    let lock = ALLOC.acquire();
    lock.as_mut().init_free_list2(k_begin);
}

pub fn idx2addr(idx: usize) -> usize {
    let lock = ALLOC.acquire();
    lock.as_mut().idx2addr(idx)
}

pub fn print_fl() {
    let lock = ALLOC.acquire();
    let a = lock.as_ref();

    for i in 0..a.free_lists.len() {
        print!("++++++++++++++++++++++++++++++ {}\n", i);
        a.free_lists[i].print_list();
    }
}
