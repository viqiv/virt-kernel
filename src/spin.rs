use core::{
    cell::UnsafeCell,
    hint::spin_loop,
    sync::atomic::{AtomicBool, Ordering},
};

struct Inner<T> {
    data: T,
    pub name: &'static str,
}

pub struct Lock<T> {
    inner: UnsafeCell<Inner<T>>,
    pub locked: AtomicBool,
}

unsafe impl<T> Sync for Lock<T> {}

impl<T> Lock<T> {
    pub const fn new(name: &'static str, data: T) -> Lock<T> {
        Lock {
            inner: UnsafeCell::new(Inner { name, data }),
            locked: AtomicBool::new(false),
        }
    }

    pub fn acquire(&self) -> LockGuard<'_, T> {
        while let Err(_) =
            self.locked
                .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        {
            spin_loop();
        }
        LockGuard { lock: self }
    }
}

pub struct LockGuard<'a, T> {
    lock: &'a Lock<T>,
}

impl<'a, T> LockGuard<'a, T> {
    pub fn as_ref(&self) -> &T {
        unsafe { &self.lock.inner.get().as_ref().unwrap().data }
    }

    pub fn as_mut(&self) -> &mut T {
        unsafe { &mut self.lock.inner.get().as_mut().unwrap().data }
    }
}

impl<'a, T> Drop for LockGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
    }
}
