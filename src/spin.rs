use core::{
    cell::UnsafeCell,
    hint::spin_loop,
    sync::atomic::{AtomicBool, Ordering},
};

use crate::sched::{Cpu, mycpu};

pub struct Lock<T> {
    data: UnsafeCell<T>,
    pub name: &'static str,
    cpu: UnsafeCell<*mut Cpu>,
    pub locked: AtomicBool,
}

unsafe impl<T> Sync for Lock<T> {}

impl<T> Lock<T> {
    pub const fn new(name: &'static str, data: T) -> Lock<T> {
        Lock {
            data: UnsafeCell::new(data),
            name,
            cpu: UnsafeCell::new(0 as *mut Cpu),
            locked: AtomicBool::new(false),
        }
    }

    pub fn acquire(&self) -> LockGuard<'_, T> {
        let cur = unsafe { self.cpu.get().read() };
        let cpu = mycpu();

        if cur == cpu {
            panic!("another lock {}", self.name);
        }

        cpu.disable_intr();
        while let Err(_) =
            self.locked
                .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        {
            spin_loop();
        }
        unsafe { self.cpu.get().write(cpu as *mut Cpu) };
        LockGuard(self)
    }

    pub fn release(&self) {
        self.locked.store(false, Ordering::Release);
        let cur = unsafe { self.cpu.get().read() };
        let cpu = mycpu();
        assert!(cpu as *mut Cpu == cur);
        cpu.enable_intr();
        unsafe { self.cpu.get().write(0 as *mut Cpu) }
    }
}

pub struct LockGuard<'a, T>(&'a Lock<T>);

impl<'a, T> LockGuard<'a, T> {
    pub fn as_ref(&self) -> &T {
        unsafe { self.0.data.get().as_ref().unwrap() }
    }

    pub fn as_mut(&self) -> &mut T {
        unsafe { self.0.data.get().as_mut().unwrap() }
    }

    pub fn get_lock(&self) -> &Lock<T> {
        self.0
    }
}

impl<'a, T> Drop for LockGuard<'a, T> {
    fn drop(&mut self) {
        self.0.release();
    }
}
