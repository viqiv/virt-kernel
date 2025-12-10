use core::{
    arch::{asm, naked_asm},
    hint::spin_loop,
    mem::forget,
    ptr::slice_from_raw_parts_mut,
    sync::atomic::{AtomicBool, Ordering},
};

use crate::{
    arch::{
        pstate_i_clr, pstate_i_set, r_pstate_daif, r_ttbr0_el1, tlbi_aside1, tlbi_vaee1,
        w_ttbr0_el1,
    },
    dsb,
    heap::SyncUnsafeCell,
    isb,
    p9::{self, open},
    pm::{self, GB},
    print, pt_from_u64,
    spin::Lock,
    stuff::BitSet128,
    trap,
    vm::{self, Vaddr, map_v2p_4k_inner, wrap},
    wfe, wfi,
};

pub struct Cpu {
    int_enable: bool,
    pub int_disables: u32,
    task_idx: Option<usize>,
    shed_ctx: [u64; 14],
}

unsafe impl Sync for Cpu {}

impl Cpu {
    pub fn disable_intr(&mut self) {
        if self.int_disables == 0 {
            self.int_enable = (r_pstate_daif() | 0b10) == 0;
            pstate_i_set();
        }
        self.int_disables += 1;
    }

    pub fn enable_intr(&mut self) {
        if self.int_disables == 1 && self.int_enable {
            pstate_i_clr();
        }
        self.int_disables -= 1;
    }

    pub fn get_task(&mut self) -> Option<&'static mut Task> {
        self.disable_intr();
        let task = match self.task_idx {
            Some(idx) => Some(&mut TASKS.as_mut()[idx]),
            _ => None,
        };
        self.enable_intr();
        task
    }
}

pub const NCPU: usize = 1;

static CPUS: SyncUnsafeCell<[Cpu; NCPU]> = SyncUnsafeCell::new([Cpu {
    int_enable: false,
    int_disables: 0,
    task_idx: None,
    shed_ctx: [0; 14],
}]);

fn cpuid() -> usize {
    0
}

pub fn mycpu() -> &'static mut Cpu {
    &mut CPUS.as_mut()[cpuid()]
}

static NTASKS: usize = 32;

enum State {
    Free,
    used,
    Ready,
    Running,
    Sleeping,
}

pub struct Task {
    state: State,
    ctx: [u64; 14],
    lock: Lock<()>,
    trapframe: u64,
    user_pt: Option<u64>,
    chan: Option<u64>,
    pid: u16,
}

unsafe impl Sync for Task {}

impl Task {
    const fn zeroed() -> Task {
        Task {
            state: State::Free,
            ctx: [0; 14],
            lock: Lock::new("T", ()),
            trapframe: 0,
            user_pt: None,
            chan: None,
            pid: 0,
        }
    }

    fn map(&mut self, v: usize, p: usize, perms: u64) -> Result<(), ()> {
        match self.user_pt {
            Some(l0_pt) => {
                let l0_pt = pt_from_u64!(l0_pt);
                if let Err(_) = map_v2p_4k_inner(l0_pt, v, p, perms) {
                    return Err(());
                }
                vm::free_4k_direct(l0_pt.as_ptr() as usize);
                Ok(())
            }
            _ => Err(()),
        }
    }

    fn init_1(&mut self, pc: u64) {
        self.user_pt = Some(pm::alloc(4096).unwrap() as u64);

        let sp_el0 = pm::alloc(4096).unwrap();
        self.map(GB - 4096, sp_el0, vm::PR_PW_UR_UW1).unwrap();
        let sp_el0 = GB;

        let sp_el1 = pm::alloc(4096).unwrap();
        let sp_el1 = vm::map(sp_el1, 1, vm::PR_PW).unwrap() + 4096;

        let tf_ptr = unsafe { (sp_el1 as *mut trap::Frame).sub(1) };
        let tf = unsafe { tf_ptr.as_mut().unwrap() };

        tf.pc = pc;
        tf.pstate = 0x0;
        tf.sp_el0 = sp_el0 as u64;

        self.ctx[12] = tf_ptr as u64;
        self.ctx[13] = forkret as *const fn() as u64;

        self.trapframe = tf_ptr as u64;
    }
}

pub fn sleep<T>(chan: u64, lock: &Lock<T>) {
    let task = mycpu().get_task().unwrap();
    let task_lock = task.lock.acquire();
    lock.release();
    task.state = State::Sleeping;
    task.chan = Some(chan);
    sched();
    task.chan = None;
    let old = lock.acquire();
    let _ = task_lock;
    forget(old);
}

pub fn wakeup(chan: u64) {
    let tasks = TASKS.as_mut();
    for i in 0..tasks.len() {
        let task = &mut tasks[i];
        let lock = task.lock.acquire();
        if let State::Sleeping = task.state {
            if let Some(c) = task.chan {
                if c == chan {
                    task.state = State::Ready;
                }
            }
        }
        let _ = lock;
    }
}

pub fn scheduler() {
    let tasks = TASKS.as_mut();
    let cpu = mycpu();

    loop {
        pstate_i_clr();
        pstate_i_set();
        let mut found = false;
        for i in 0..tasks.len() {
            let task = &mut tasks[i];
            let lock = task.lock.acquire();
            match task.state {
                State::Ready => {
                    task.state = State::Running;
                    cpu.task_idx = Some(i);
                    switch(cpu.shed_ctx.as_mut_ptr(), task.ctx.as_ptr());
                    cpu.task_idx = None;
                    found = true;
                }
                _ => {}
            }
            let _ = lock;
        }

        if !found {
            wfi!();
        }
    }
}

pub fn sched() {
    let cpu = mycpu();
    assert!(cpu.int_disables == 1);
    // go back to sheduler()
    switch(
        cpu.get_task().unwrap().ctx.as_mut_ptr(),
        cpu.shed_ctx.as_ptr(),
    );
    mycpu().int_enable = cpu.int_enable;
}

pub fn yild() {
    let task = mycpu().get_task().unwrap();
    let lock = task.lock.acquire(); // re-acquire one released at fork ret
    task.state = State::Ready;
    sched();
    let _ = lock;
}

fn alloc_pid() -> Option<u16> {
    let tasks = TASKS.as_mut();
    for i in 0..tasks.len() {
        let task = &mut tasks[i];
        let lock = task.lock.acquire();
        if let State::Free = task.state {
            task.state = State::used;
            return Some(i as u16);
        }
        let _ = lock;
    }
    None
}

fn restore_ttbr0(task_idx: usize, pt: usize) {
    let ttbr0 = (task_idx << 48) | pt as usize;
    w_ttbr0_el1(ttbr0 as u64);
    dsb!();
    isb!();
    tlbi_aside1(task_idx as u64);
    dsb!();
    isb!();
}

pub fn create_task(entry: u64) {
    let tasks = TASKS.as_mut();
    let pid = alloc_pid().unwrap();
    let task = &mut tasks[pid as usize];
    task.pid = pid;
    task.init_1(entry);
    task.state = State::Ready;
}

#[unsafe(naked)]
#[unsafe(no_mangle)]
extern "C" fn switch(save: *mut u64, load: *const u64) {
    naked_asm!(
        "stp x19, x18, [x0], #16",
        "stp x21, x20, [x0], #16",
        "stp x23, x22, [x0], #16",
        "stp x25, x24, [x0], #16",
        "stp x27, x26, [x0], #16",
        "stp x29, x28, [x0], #16",
        "mov x2, sp",
        "stp x2, x30, [x0], #16",
        //==========================
        "ldp x19, x18, [x1], #16",
        "ldp x21, x20, [x1], #16",
        "ldp x23, x22, [x1], #16",
        "ldp x25, x24, [x1], #16",
        "ldp x27, x26, [x1], #16",
        "ldp x29, x28, [x1], #16",
        "ldp x2, x30, [x1], #16",
        "mov sp, x2",
        "ret"
    )
}

static FIRST: AtomicBool = AtomicBool::new(true);

#[unsafe(no_mangle)]
#[allow(unused)]
pub extern "C" fn forkret() {
    let cpu = mycpu();
    let task = cpu.get_task().unwrap();
    // was held in scheduler()
    task.lock.release();

    restore_ttbr0(task.pid as usize, task.user_pt.unwrap() as usize);

    let ptr = (GB - 4096) as *mut u8;
    print!("PTR ===== {:x} {:x}\n", ptr as usize, r_ttbr0_el1());

    let f = open("fox", p9::O::RDONLY as u32).unwrap();

    if FIRST.swap(false, Ordering::Release) {
        print!("FIRST SHIT!\n");
    }

    unsafe {
        asm!(
            "mov sp, {}",
            "ldp x0, x1, [sp], #16",
            "msr elr_el1, x0",
            "msr sp_el0, x1",
            "ldp x0, x30, [sp], #16",
            "msr spsr_el1, x0",
            "ldp x29, x28, [sp], #16",
            "ldp x27, x26, [sp], #16",
            "ldp x25, x24, [sp], #16",
            "ldp x23, x22, [sp], #16",
            "ldp x21, x20, [sp], #16",
            "ldp x19, x18, [sp], #16",
            "ldp x17, x16, [sp], #16",
            "ldp x15, x14, [sp], #16",
            "ldp x13, x12, [sp], #16",
            "ldp x11, x10, [sp], #16",
            "ldp x9, x8, [sp], #16",
            "ldp x7, x6, [sp], #16",
            "ldp x5, x4, [sp], #16",
            "ldp x3, x2, [sp], #16",
            "ldp x1, x0, [sp], #16",
            "eret",
            in(reg) task.trapframe
        )
    }
}

static TASKS: SyncUnsafeCell<[Task; NTASKS]> = SyncUnsafeCell::new([
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
    Task::zeroed(),
]);
