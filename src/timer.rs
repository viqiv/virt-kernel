use core::arch::asm;

use crate::{
    heap::SyncUnsafeCell,
    print,
    sched::{self, Task, Wq, cpuid, mycpu, wakeup},
    spin::Lock,
    trap,
};

#[allow(unused)]
#[inline]
fn r_freq() -> u64 {
    let mut r = 0u64;
    unsafe { asm!("MRS {}, CNTFRQ_EL0", out(reg) r) };
    r
}

#[allow(unused)]
#[inline]
fn w_freq(r: u64) {
    unsafe { asm!("MSR CNTFRQ_EL0, {}", in(reg) r) };
}

#[allow(unused)]
#[inline]
fn r_pctl_el0() -> u64 {
    let mut r = 0u64;
    unsafe { asm!("MRS {}, CNTP_CTL_EL0", out(reg) r) };
    r
}

#[allow(unused)]
#[inline]
fn w_pctl_el0(r: u64) {
    unsafe { asm!("MSR CNTP_CTL_EL0, {}", in(reg) r) };
}

#[allow(unused)]
#[inline]
fn r_pct_el0() -> u64 {
    let mut r = 0u64;
    unsafe { asm!("MRS {}, CNTPCT_EL0", out(reg) r) };
    r
}

#[allow(unused)]
#[inline]
fn r_ptval_el0() -> u64 {
    let mut r = 0u64;
    unsafe { asm!("MRS {}, CNTP_TVAL_EL0", out(reg) r) };
    r
}

#[allow(unused)]
#[inline]
fn w_ptval_el0(r: u64) {
    unsafe { asm!("MSR CNTP_TVAL_EL0, {}", in(reg) r) };
}

#[allow(unused)]
#[inline]
fn r_pcval_el0() -> u64 {
    let mut r = 0u64;
    unsafe { asm!("MRS {}, CNTP_CVAL_EL0", out(reg) r) };
    r
}

#[allow(unused)]
#[inline]
fn w_pcval_el0(r: u64) {
    unsafe { asm!("MSR CNTP_CVAL_EL0, {}", in(reg) r) };
}

pub fn init() {
    trap::gic_enable_intr(30);
    w_pctl_el0(1);
}

pub fn handle_tik(el: u8) {
    let freq = r_freq();

    // freq = ticks/s
    // freq/100 = ticks/(s/100)

    if cpuid() == 0 {
        let lock = TICKLOCK.acquire();
        // print!("T {} {} {}\n", lock.as_ref().0, lock.as_ref().1.count, el);
        lock.as_mut().0 += 1;
        // wakeup(lock.as_ref() as *const u64 as u64);
        lock.as_mut().1.wake_all();
        drop(lock);
    }

    w_ptval_el0(freq / 100);

    if (el == 1 && mycpu().get_task().is_some()) || el == 0 {
        sched::yild();
    }
}

static TICKLOCK: Lock<(u64, Wq)> = Lock::new("TICK", (0, Wq::new("ticks")));

pub fn sleep(millis: u64) {
    let lock = TICKLOCK.acquire();
    let mut start = lock.as_ref().0;

    while (lock.as_ref().0 - start < millis) {
        // sched::sleep(lock.as_ref() as *const u64 as u64, lock.get_lock());
        lock.as_mut().1.sleep(lock.get_lock());
    }
}

pub fn read_tick() -> u64 {
    let lock = TICKLOCK.acquire();
    lock.as_ref().0
}

pub fn add2wait() {
    let lock = TICKLOCK.acquire();
    let task = mycpu().get_task().unwrap();
    lock.as_mut().1.add(task as *mut Task);
}
