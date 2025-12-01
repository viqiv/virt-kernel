use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, Ordering},
};

struct Inner<T> {
    data: T,
    pub name: &'static str,
    pub locked: AtomicBool,
}

pub struct Lock<T> {
    inner: UnsafeCell<Inner<T>>,
}

unsafe impl<T: Sync> Sync for Lock<T> {}

impl<T> Lock<T> {
    pub const fn new(name: &'static str, data: T) -> Lock<T> {
        Lock {
            inner: UnsafeCell::new(Inner {
                name,
                locked: AtomicBool::new(false),
                data,
            }),
        }
    }

    pub fn acquire(&self) -> LockGuard<T> {
        let inner = unsafe { self.inner.get().as_ref() }.unwrap();
        while let Err(_) =
            inner
                .locked
                .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        {}
        LockGuard {
            lock: self as *const Lock<T> as *mut Lock<T>,
        }
    }

    pub fn release(&self) {
        let inner = unsafe { self.inner.get().as_ref() }.unwrap();
        inner.locked.store(false, Ordering::Release);
    }
}

pub struct LockGuard<T> {
    lock: *mut Lock<T>,
}

impl<T> LockGuard<T> {
    pub fn as_ref(&self) -> &T {
        unsafe {
            &self
                .lock
                .as_ref()
                .unwrap()
                .inner
                .get()
                .as_ref()
                .unwrap()
                .data
        }
    }

    pub fn as_mut(&self) -> &mut T {
        unsafe {
            &mut self
                .lock
                .as_ref()
                .unwrap()
                .inner
                .get()
                .as_mut()
                .unwrap()
                .data
        }
    }
}

impl<T> Drop for LockGuard<T> {
    fn drop(&mut self) {
        unsafe { self.lock.as_ref().unwrap() }.release();
    }
}
