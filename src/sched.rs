use core::{
    arch::{asm, naked_asm},
    cmp::min,
    hint::spin_loop,
    mem::forget,
    ptr::slice_from_raw_parts_mut,
    sync::atomic::{AtomicBool, Ordering},
};

use alloc::{collections::vec_deque::VecDeque, vec::Vec};

use crate::{
    arch::{
        pstate_i_clr, pstate_i_set, r_pstate_daif, r_ttbr0_el1, tlbi_aside1, tlbi_vaee1,
        w_ttbr0_el1,
    },
    dsb,
    elf::{self, Elf, Elf64Phdr, PhIter},
    fs::{self, File, open},
    heap::SyncUnsafeCell,
    isb, p9,
    pm::{self, GB, align_b, align_f},
    print, pt_from_u64,
    spin::Lock,
    stuff::{BitSet128, as_slice_mut, cstr_as_slice, defer},
    trap,
    vm::{self, Vaddr, free_pt, map_v2p_4k_inner, wrap, zero_phys_pt},
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

    fn alloc(&mut self, vfrom: usize, mut len: usize) {}

    fn init_1(&mut self, pc: u64) {
        self.user_pt = Some(pm::alloc(4096).unwrap() as u64);
        zero_phys_pt(self.user_pt.unwrap());

        // writable by all
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

pub fn execv(path: &str, argv: &[*const u8], envp: &[*const u8]) -> Result<(), ()> {
    let task = mycpu().get_task().unwrap();

    let old_user_pt = task.user_pt.unwrap();

    let new_user_pt = if let Some(p) = pm::alloc(4096) {
        p
    } else {
        return Err(());
    } as u64;

    zero_phys_pt(new_user_pt);

    task.user_pt = Some(new_user_pt);
    restore_ttbr0(task.pid as usize, new_user_pt as usize);

    if let Some(new_user_sp) = pm::alloc(4096) {
        if let Err(_) = task.map(GB - 4096, new_user_sp, vm::PR_PW_UR_UW1) {
            pm::free(new_user_sp as *mut u8);
            task.user_pt = Some(old_user_pt);
            restore_ttbr0(task.pid as usize, old_user_pt as usize);
            return Err(());
        }
    } else {
        task.user_pt = Some(old_user_pt);
        restore_ttbr0(task.pid as usize, old_user_pt as usize);
        return Err(());
    }

    let mut elf = if let Ok(elf) = Elf::new(path) {
        elf
    } else {
        free_pt(new_user_pt);
        task.user_pt = Some(old_user_pt);
        restore_ttbr0(task.pid as usize, old_user_pt as usize);
        return Err(());
    };

    let file = unsafe { (elf.file as *mut File).as_mut() }.unwrap();
    let mut phit = PhIter::new(&mut elf);
    let mut ph = Elf64Phdr::zeroed();
    let mut error = false;
    while let Some(p) = phit.next((&mut ph) as *mut Elf64Phdr) {
        if error {
            break;
        }

        if p.kind as u64 != elf::PT_LOAD {
            continue;
        }
        let len = align_f((p.vaddr as usize % 4096) + p.memsz as usize, 4096);
        let vfrom = align_b(p.vaddr as usize, 4096);

        let pages = len / 4096;
        let mut wofft = p.vaddr;
        for i in 0..pages {
            let pm = pm::alloc(4096).unwrap();
            if let Err(_) = task.map(
                vfrom + i * 4096,
                pm,
                if p.flags == elf::PF_R | elf::PF_X {
                    vm::PR_UR_UX
                } else if p.flags == elf::PF_R | elf::PF_W {
                    vm::PR_PW_UR_UW1
                } else if p.flags == elf::PF_R {
                    vm::PR_UR
                } else {
                    error = true;
                    break;
                },
            ) {
                pm::free(pm as *mut u8);
                error = true;
                break;
            }

            if let Ok(vm) = vm::map(pm, 1, vm::PR_PW) {
                let buf_offt = wofft as usize % 4096;
                let written = wofft - p.vaddr;
                let buf = as_slice_mut(
                    (vm + buf_offt) as *mut u8, //
                    min(4096 - buf_offt, p.filesz as usize - written as usize),
                );
                file.seek_to(p.offset as usize + written as usize);

                if let Ok(n) = file.read(buf) {
                    if n != buf.len() as usize {
                        error = true;
                        vm::free(vm, 1);
                        break;
                    }
                    wofft += n as u64;
                    vm::free(vm, 1);
                } else {
                    error = true;
                    vm::free(vm, 1);
                    break;
                }
            } else {
                error = true;
                break;
            }
        }

        if error {
            break;
        }
    }

    let defer = defer(|| {
        free_pt(new_user_pt);
        task.user_pt = Some(old_user_pt);
        restore_ttbr0(task.pid as usize, old_user_pt as usize);
    });

    if error {
        return Err(());
    }

    let mut s = Vec::new();
    let sp_el0 = as_slice_mut((GB - 4096) as *mut u8, 4096);
    let mut w_idx = 4095;
    for i in 0..envp.len() {
        let slice = cstr_as_slice(envp[envp.len() - i - 1]);
        if slice.len() == 0 {
            break;
        }
        if slice.len() + 1 > w_idx {
            return Err(());
        }
        sp_el0[w_idx] = 0;
        w_idx -= slice.len() + 1;
        sp_el0[w_idx..w_idx + slice.len()].copy_from_slice(slice);
        s.push(&sp_el0[w_idx] as *const u8);
    }

    for i in 0..argv.len() {
        let slice = cstr_as_slice(argv[argv.len() - i - 1]);
        if slice.len() == 0 {
            break;
        }
        if slice.len() + 1 > w_idx {
            return Err(());
        }
        sp_el0[w_idx] = 0;
        w_idx -= slice.len() + 1;
        sp_el0[w_idx..w_idx + slice.len()].copy_from_slice(slice);
        s.push(&sp_el0[w_idx] as *const u8);
    }

    if w_idx == 4095 {
        if path.len() + 1 + 8 > w_idx {
            return Err(());
        }
        sp_el0[w_idx] = 0;
        w_idx -= path.len() + 1;
        sp_el0[w_idx..w_idx + path.len()].copy_from_slice(path.as_bytes());
        s.push(&sp_el0[w_idx] as *const u8);
    }

    w_idx = align_b(w_idx, 8);
    let ptrs_len = 8 * (2/*nulls*/ + s.len()/*ptrs*/);
    if w_idx < ptrs_len {
        return Err(());
    }

    let ptrs = as_slice_mut(
        &sp_el0[w_idx - ptrs_len] as *const u8 as *mut usize,
        ptrs_len,
    );

    w_idx = 0;

    for _ in 0..argv.len() {
        ptrs[w_idx] = s.pop().unwrap() as usize;
        w_idx += 1;
    }

    ptrs[w_idx] = 0;
    w_idx += 1;

    for _ in 0..envp.len() {
        ptrs[w_idx] = s.pop().unwrap() as usize;
        w_idx += 1;
    }

    ptrs[w_idx] = 0;

    let tf = unsafe { (task.trapframe as *mut trap::Frame).as_mut() }.unwrap();

    tf.pc = elf.header.entry;
    tf.pstate = 0x0;
    tf.sp_el0 = ptrs.as_ptr() as u64;

    tf.regs[0] = argv.len() as u64 + 1;

    free_pt(old_user_pt);
    forget(defer);
    Ok(())
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

    if FIRST.swap(false, Ordering::Release) {
        execv("main", &["main\0".as_ptr()], &["FOO:bar\0".as_ptr()]).unwrap();
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
