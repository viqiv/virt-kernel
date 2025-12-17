use core::cell::UnsafeCell;

use linked_list_allocator::LockedHeap;

use crate::pm::MB;

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

#[repr(transparent)]
pub struct SyncUnsafeCell<T>(pub UnsafeCell<T>);
impl<T> SyncUnsafeCell<T> {
    pub const fn new(v: T) -> SyncUnsafeCell<T> {
        SyncUnsafeCell(UnsafeCell::new(v))
    }

    pub fn as_mut(&self) -> &mut T {
        unsafe { self.0.get().as_mut() }.unwrap()
    }

    pub fn as_ref(&self) -> &T {
        unsafe { self.0.get().as_ref() }.unwrap()
    }
}

unsafe impl<T: Sync> Sync for SyncUnsafeCell<T> {}

#[unsafe(link_section = ".bss.heap")]
static HEAP_MEMORY: SyncUnsafeCell<[u8; MB]> = SyncUnsafeCell(UnsafeCell::new([0u8; MB]));

pub fn init() {
    unsafe {
        let ptr = HEAP_MEMORY.0.get() as *mut u8;
        ALLOCATOR.lock().init(ptr, MB);
    }
}
