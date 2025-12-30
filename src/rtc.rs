use crate::{
    heap::{self, SyncUnsafeCell},
    sched::mycpu,
    vm,
};

struct U32ptr(*mut u32);
unsafe impl Sync for U32ptr {}

static MAP: SyncUnsafeCell<U32ptr> = SyncUnsafeCell::new(U32ptr(0 as *mut u32));

pub fn init() {
    let v = vm::map(0x9010000, 1, vm::PR_PW).unwrap();
    MAP.as_mut().0 = v as *mut u32;
}

pub fn read() -> u32 {
    unsafe { MAP.as_ref().0.read_volatile() }
}

#[repr(C)]
struct KernelTimespec {
    sec: i64,
    nsec: i64,
}

struct Clock;
impl Clock {
    const REALTIME: u64 = 0;
    const MONOTONIC: u64 = 1;
    const PROCESS_CPUTIME_ID: u64 = 2;
    const THREAD_CPUTIME_ID: u64 = 3;
    const MONOTONIC_RAW: u64 = 4;
    const REALTIME_COARSE: u64 = 5;
    const MONOTONIC_COARSE: u64 = 6;
    const BOOTTIME: u64 = 7;
    const REALTIME_ALARM: u64 = 8;
    const BOOTTIME_ALARM: u64 = 9;
}

pub fn clock_gettime() -> u64 {
    let task = mycpu().get_task().unwrap();
    let tf = task.get_trap_frame().unwrap();
    let ts = (tf.regs[1] as *mut KernelTimespec);
    match tf.regs[0] {
        Clock::REALTIME_COARSE => unsafe {
            ts.write(KernelTimespec {
                sec: read() as i64,
                nsec: 0,
            })
        },
        x => panic!("unimplemented clock: {}\n", x),
    }
    0
}
