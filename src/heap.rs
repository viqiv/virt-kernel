use core::cell::UnsafeCell;

use linked_list_allocator::LockedHeap;

use crate::pm::MB;

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

struct HeapBuf {
    buf: UnsafeCell<[u8; MB]>,
}

impl HeapBuf {
    fn as_mut_ptr(&self) -> *mut u8 {
        unsafe { self.buf.get().as_mut() }.unwrap().as_mut_ptr()
    }

    fn len(&self) -> usize {
        unsafe { self.buf.get().as_ref() }.unwrap().len()
    }
}

unsafe impl Sync for HeapBuf {}

static HEAP_MEMORY: HeapBuf = HeapBuf {
    buf: UnsafeCell::new([0; MB]),
};

pub fn init() {
    unsafe {
        ALLOCATOR
            .lock()
            .init(HEAP_MEMORY.as_mut_ptr(), HEAP_MEMORY.len());
    }
}
