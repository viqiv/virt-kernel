use core::{
    arch::{asm, naked_asm},
    cmp::min,
    hint::spin_loop,
    mem::forget,
    ops::Sub,
    ptr::slice_from_raw_parts_mut,
    sync::atomic::{AtomicBool, Ordering},
};

use alloc::{
    collections::{btree_map::BTreeMap, vec_deque::VecDeque},
    string::String,
    vec::Vec,
};

use crate::{
    arch::{
        pstate_i_clr, pstate_i_set, r_far_el1, r_pstate_daif, r_ttbr0_el1, tlbi_aside1, tlbi_vaee1,
        w_tpidrro_el0, w_ttbr0_el1,
    },
    dsb,
    elf::{self, Elf, Elf64Phdr, PhIter},
    fs::{self, File, open},
    heap::SyncUnsafeCell,
    isb, p9,
    pm::{self, Flags, GB, align_b, align_f},
    print,
    spin::Lock,
    stuff::{BitSet128, as_slice_mut, cstr_as_slice, defer},
    tlbi_vmalle1, trap,
    vm::{self, PmWrap, Vaddr, free_pt, map_v2p_4k_inner, unmap_4k_inner, v2p, v2p_pt},
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
    Used,
    Ready,
    Running,
    Sleeping,
    Zombie,
}

#[derive(Clone, Copy, Debug)]
struct Region {
    len: usize,
    flags: u32,
}

type RTree = BTreeMap<usize, Region>;

pub struct Task {
    parent: Option<*const Task>,
    exit_code: u64,
    state: State,
    ctx: [u64; 14],
    lock: Lock<()>,
    pub trapframe: u64,
    user_pt: Option<u64>,
    chan: Option<u64>,
    pid: u16,
    pub files: [Option<&'static mut fs::File>; 8],
    regions: RTree,
    mappings: RTree,
    pub cwd: Option<String>,
}

unsafe impl Sync for Task {}

impl Task {
    const fn zeroed() -> Task {
        Task {
            parent: None,
            exit_code: 0,
            state: State::Free,
            ctx: [0; 14],
            lock: Lock::new("T", ()),
            trapframe: 0,
            user_pt: None,
            chan: None,
            pid: 0,
            files: [None, None, None, None, None, None, None, None],
            regions: BTreeMap::new(),
            mappings: RTree::new(),
            cwd: None,
        }
    }

    pub fn get_trap_frame(&self) -> Option<&'static mut trap::Frame> {
        unsafe { (self.trapframe as *mut trap::Frame).as_mut() }
    }

    fn init_1(&mut self, pc: u64) {
        let user_pt = pm::alloc(4096).unwrap() as u64;
        self.user_pt = Some(user_pt);
        let _ = PmWrap::new(self.user_pt.unwrap() as usize, vm::PR_PW, true).unwrap();

        let sp_el1 = pm::alloc(4096).unwrap();
        let sp_el1 = vm::map(sp_el1, 1, vm::PR_PW).unwrap() + 4096;

        let tf_ptr = unsafe { (sp_el1 as *mut trap::Frame).sub(1) };
        let tf = unsafe { tf_ptr.as_mut().unwrap() };

        tf.pc = pc;
        tf.pstate = 0x0;

        self.ctx[12] = tf_ptr as u64;
        self.ctx[13] = forkret as *const fn() as u64;

        self.trapframe = tf_ptr as u64;
    }
}

fn map(l0_pt: &mut [u64], v: usize, p: usize, n: usize, perms: u64) -> Result<usize, vm::Error> {
    for i in 0..n {
        map_v2p_4k_inner(l0_pt, v + (4096 * i), p + (4096 * i), perms, false).map_err(|e| e)?;
    }
    Ok(v)
}

fn unmap(l0_pt: &mut [u64], v: usize, n: usize) -> Result<(), vm::Error> {
    for i in 0..n {
        unmap_4k_inner(l0_pt, v + (4096 * i)).map_err(|e| e)?;
    }
    Ok(())
}

fn map_ovwr(
    l0_pt: &mut [u64],
    v: usize,
    p: usize,
    n: usize,
    perms: u64,
) -> Result<usize, vm::Error> {
    for i in 0..n {
        match map_v2p_4k_inner(
            l0_pt,
            v + (4096 * i),
            p + (4096 * i), //
            perms,
            true,
        ) {
            Err(vm::Error::Exists) => {}
            e => return e,
        };
    }
    Ok(v)
}

fn alloc_region(r: &mut RTree, len: usize, flags: u32) -> Option<usize> {
    if let Some((k, v)) = r.last_key_value() {
        let free_begin = align_f(k + v.len, 4096);
        r.insert(free_begin, Region { len, flags });
        Some(free_begin)
    } else {
        None
    }
}

const SPEL0_SIZE: usize = 4096 * 2;

pub fn execv(path: &str, argv: &[*const u8], envp: &[*const u8]) -> Result<(), ()> {
    let mut new_regions = BTreeMap::<usize, Region>::new();
    let task = mycpu().get_task().unwrap();
    let user_pt = pm::alloc(4096).map_err(|_| ())?;

    let defer_user_pt = defer(|| {
        vm::free_pt(user_pt as u64);
    });

    let l0_pt = PmWrap::new(user_pt as usize, vm::PR_PW, true).unwrap();

    let mut elf = Elf::new(path).map_err(|_| ())?;

    let file = unsafe { (elf.file as *mut File).as_mut() }.unwrap();
    let mut phit = PhIter::new(&mut elf);
    let mut ph = Elf64Phdr::zeroed();
    while let Some(p) = phit.next((&mut ph) as *mut Elf64Phdr) {
        if p.kind as u64 != elf::PT_LOAD {
            continue;
        }

        let len = align_f((p.vaddr as usize % 4096) + p.memsz as usize, 4096);
        let vfrom = align_b(p.vaddr as usize, 4096);

        let pages = len / 4096;
        let mut wofft = p.vaddr;
        for i in 0..pages {
            let pm = pm::alloc(4096).map_err(|_| ())?;
            let defer_pm = defer(|| {
                crate::pm::free(pm as usize);
            });
            map(
                l0_pt.as_slice_mut(),
                vfrom + i * 4096,
                pm,
                1,
                if p.flags == elf::PF_R | elf::PF_X {
                    vm::PR_UR_UX
                } else if p.flags == elf::PF_R | elf::PF_W {
                    vm::PR_PW_UR_UW1
                } else if p.flags == elf::PF_R {
                    vm::PR_UR
                } else {
                    panic!("unhandled flags combo")
                },
            )
            .map_err(|_| ())?;
            forget(defer_pm);

            let vm = PmWrap::new(pm, vm::PR_PW, false).map_err(|_| ())?;
            let buf_offt = wofft as usize % 4096;
            let written = wofft - p.vaddr;
            let buf = &mut vm.as_slice_mut::<u8>()
                [buf_offt..buf_offt + min(4096 - buf_offt, p.filesz as usize - written as usize)];

            file.seek_to(p.offset as usize + written as usize);

            let n = file.read(buf).map_err(|_| ())?;
            if n != buf.len() as usize {
                return Err(());
            }

            wofft += n as u64;
        }

        new_regions.insert(
            vfrom,
            Region {
                len,
                flags: p.flags,
            },
        );
    }

    let user_sp = pm::alloc(SPEL0_SIZE).map_err(|_| ())?;
    let defer_user_sp = defer(|| {
        pm::free(user_sp as usize);
    });

    let user_sp_region = alloc_region(
        &mut new_regions,
        SPEL0_SIZE, //
        elf::PF_R | elf::PF_W,
    )
    .unwrap();

    forget(defer_user_sp);

    let sp_el0_w = PmWrap::new(user_sp + 4096, vm::PR_PW, true).map_err(|_| ())?;
    let sp_el0 = sp_el0_w.as_slice_mut::<u8>();
    let mut w_idx = 4096;
    let btm = user_sp_region + 4096;

    w_idx -= 16; //AT_RANDOM
    let at_random = btm + w_idx;

    #[repr(C)]
    struct Aux {
        k: u64,
        v: u64,
    }

    let mut aux_ptr = (&mut sp_el0[w_idx]) as *mut u8 as *mut Aux;
    let mut aux_ref = unsafe { aux_ptr.as_mut() }.unwrap();
    let _ = aux_ref;

    {
        if w_idx < 16 {
            return Err(());
        }
        unsafe {
            aux_ptr = aux_ptr.sub(1);
            aux_ref = aux_ptr.as_mut().unwrap();
            aux_ref.k = 0;
            aux_ref.v = 0;
        }
        w_idx -= 16;
    }

    {
        if w_idx < 16 {
            return Err(());
        }
        unsafe {
            aux_ptr = aux_ptr.sub(1);
            aux_ref = aux_ptr.as_mut().unwrap();
            aux_ref.k = 25;
            aux_ref.v = at_random as u64;
        }
        w_idx -= 16;
    }

    let mut s = Vec::new();
    s.push(0); // envp null term

    for i in 0..envp.len() {
        let slice = cstr_as_slice(envp[envp.len() - i - 1]);
        if slice.len() == 0 {
            break;
        }
        if slice.len() + 1 > w_idx {
            return Err(());
        }
        w_idx -= 1;
        sp_el0[w_idx] = 0;
        w_idx -= slice.len();
        sp_el0[w_idx..w_idx + slice.len()].copy_from_slice(slice);
        s.push(btm + w_idx);
    }

    s.push(0); // argv null term

    for i in 0..argv.len() {
        let slice = cstr_as_slice(argv[argv.len() - i - 1]);
        if slice.len() == 0 {
            break;
        }
        if slice.len() + 1 > w_idx {
            return Err(());
        }
        w_idx -= 1;
        sp_el0[w_idx] = 0;
        w_idx -= slice.len();
        sp_el0[w_idx..w_idx + slice.len()].copy_from_slice(slice);
        s.push(btm + w_idx);
    }

    w_idx = align_b(w_idx, 8);
    if w_idx < 8 {
        return Err(());
    }

    let ptrs_len = 8 * (s.len() + 1);
    if w_idx < ptrs_len {
        return Err(());
    }

    let ptrs = as_slice_mut(
        &sp_el0[w_idx - ptrs_len] as *const u8 as *mut usize,
        ptrs_len,
    );

    let sp_pos = (btm + 4096) - (ptrs_len + (4096 - w_idx));
    w_idx = 0;

    ptrs[w_idx] = argv.len();
    w_idx += 1;

    while let Some(ptr) = s.pop() {
        ptrs[w_idx] = ptr as usize;
        w_idx += 1;
    }

    let tf = unsafe { (task.trapframe as *mut trap::Frame).as_mut() }.unwrap();

    tf.pc = elf.header.entry;
    tf.pstate = 0x0;
    tf.sp_el0 = sp_pos as u64;

    free_pt(task.user_pt.unwrap());

    forget(defer_user_pt);

    task.regions = new_regions;
    task.user_pt = Some(user_pt as u64);
    map(
        l0_pt.as_slice_mut(),
        user_sp_region,
        user_sp,
        SPEL0_SIZE / 4096,
        vm::PR_PW_UR_UW1,
    ) //
    .map_err(|_| ())?;
    // .unwrap();
    restore_ttbr0(task.pid as usize, user_pt as usize);
    Ok(())
}

pub fn brk() -> u64 {
    let task = mycpu().get_task().unwrap();
    let tf = task.get_trap_frame().unwrap();
    let last = task.regions.last_key_value().unwrap();
    let pos = (last.0 + last.1.len) as u64;

    let new_pos = tf.regs[0];

    if new_pos == 0 {
        return pos;
    }

    if new_pos < pos {
        return !0;
    }

    if new_pos == pos {
        return pos;
    }

    let incr = align_f(new_pos as usize - pos as usize, 4096);

    let region = alloc_region(&mut task.regions, incr as usize, elf::PF_R | elf::PF_W);
    if region.is_none() {
        return !0;
    }
    let region = region.unwrap();

    let def = defer(|| {
        task.regions.pop_last();
    });

    let p = pm::alloc(incr as usize);
    if p.is_err() {
        unreachable!();
        // return !0;
    }
    let p = p.unwrap();

    let def2 = defer(|| {
        pm::free(p);
    });

    let l0_pt = PmWrap::new(task.user_pt.unwrap() as usize, vm::PR_PW, false);
    if l0_pt.is_err() {
        return !0;
    }

    let l0_pt = l0_pt.unwrap();

    return match map(
        l0_pt.as_slice_mut(),
        region,
        p,
        incr / 4096,
        vm::PR_PW_UR_UW1,
    ) {
        Ok(_) => {
            forget(def2);
            forget(def);
            new_pos
        }
        _ => !0,
    };

    // panic!("brk({});", incr);
}

pub fn settid() -> u64 {
    let task = mycpu().get_task().unwrap();
    task.pid as u64
}

pub fn set_robust_list() -> u64 {
    0
}

pub fn rseq() -> u64 {
    !0
}

pub fn prlimit64() -> u64 {
    0
}

const MAPPINGS_BEGIN: usize = GB;

pub fn mprotect() -> u64 {
    0
}

pub fn mmap() -> u64 {
    let task = mycpu().get_task().unwrap();
    let tf = task.get_trap_frame().unwrap();
    let flags = tf.regs[3];

    // TODO
    if (flags & 0x20) == 0 {
        return !0;
    }

    let len = align_f(tf.regs[1] as usize, 4096);

    // TODO
    if len.count_ones() != 1 {
        return !0;
    }

    // print!(
    //     "mmap flags = {:x} prot = {:x} len = {}\n",
    //     flags, tf.regs[2], len
    // );

    let p = pm::alloc(len as usize);
    if p.is_err() {
        return !0;
    }
    let p = p.unwrap();

    let def = defer(|| {
        pm::free(p);
    });

    let x = tf.regs[2];
    let region = alloc_region(
        &mut task.mappings, //
        len as usize,
        x as u32,
    );

    if region.is_none() {
        return !0;
    }
    let region = region.unwrap() + MAPPINGS_BEGIN;

    let def2 = defer(|| {
        task.regions.pop_last();
    });

    let perms = if tf.regs[2] == 1 {
        vm::PR_UR
    } else if tf.regs[2] == 3 {
        vm::PR_PW_UR_UW1
    } else if tf.regs[2] == 5 {
        vm::PR_UR_UX
    } else if tf.regs[2] == 0 {
        return region as u64;
    } else {
        panic!("mmap: unknown perms: {}\n", tf.regs[2]);
    };

    let l0_pt = PmWrap::new(task.user_pt.unwrap() as usize, vm::PR_PW, false);
    if l0_pt.is_err() {
        return !0;
    }

    let l0_pt = l0_pt.unwrap();

    return match map(l0_pt.as_slice_mut(), region, p, len as usize / 4096, perms) {
        Ok(_) => {
            forget(def2);
            forget(def);
            region as u64
        }
        _ => !0,
    };
    // panic!("mmap {:?}\n", tf);
}

fn clone_regions(
    from: &RTree,
    to: &mut RTree, //
    from_pt: &mut [u64],
    to_pt: &mut [u64],
) -> Result<(), ()> {
    let mut fit = from.iter();
    while let Some((k, v)) = fit.next() {
        assert!(k % 4096 == 0 && v.len % 4096 == 0);
        let flags = if v.flags == elf::PF_R | elf::PF_X {
            // 0
            vm::PR_UR_UX
        } else if v.flags == elf::PF_R | elf::PF_W {
            // 0
            vm::PR_UR
        } else if v.flags == elf::PF_R {
            // 0
            vm::PR_UR
        } else {
            panic!("unhandled flags combo")
        };

        let n = v.len / 4096;
        let mut i = 0;
        while i < n {
            let closure = |ent: *mut u64| unsafe {
                *ent = (*ent & vm::PHY_MASK as u64) | flags | 0x403;
            };
            let vm = *k + (i * 4096);
            let pm = v2p_pt(from_pt, vm, Some(closure)).map_err(|_| ())?;
            let pm_page = pm::lookup(pm);
            if pm_page.is_none() {
                return Err(());
            }
            let pm_page = pm_page.unwrap();
            if pm_page.ref_cnt == 0 {
                return Err(());
            }
            let pages = pm_page.len() / 4096;
            map(to_pt, vm, pm, pages, flags).map_err(|_| ())?;
            for j in 1..pages {
                v2p_pt(from_pt, vm + 4096 * j, Some(closure)).map_err(|_| ())?;
            }
            i += pages;
            pm_page.dup_for_cow();
        }

        to.insert(*k, *v);
    }

    Ok(())
}

pub fn fork() -> u64 {
    let task = mycpu().get_task().unwrap();
    if let Some(new_task) = alloc_task() {
        let defer = defer(|| if let Ok(_) = free_task(new_task.pid as usize) {});
        let from = PmWrap::new(task.user_pt.unwrap() as usize, vm::PR_PW, false);
        if from.is_err() {
            return !0;
        }
        let to = PmWrap::new(new_task.user_pt.unwrap() as usize, vm::PR_PW, false);
        if to.is_err() {
            return !0;
        }

        if let Err(_) = clone_regions(
            &task.regions,
            &mut new_task.regions, //
            from.unwrap().as_slice_mut(),
            to.unwrap().as_slice_mut(),
        ) {
            return !0;
        }

        for i in 0..task.files.len() {
            if let Some(f) = &mut task.files[i] {
                new_task.files[i] = f.dup();
            }
        }

        let nt = new_task.get_trap_frame().unwrap();
        let ot = task.get_trap_frame().unwrap();
        *nt = *ot;

        nt.regs[0] = 0;
        tlbi_aside1(task.pid as u64);
        dsb!();
        isb!();
        new_task.state = State::Ready;
        new_task.parent = Some(task as *const Task);
        forget(defer);
        new_task.pid as u64
    } else {
        !0
    }
}

fn free_regions(regions: &mut RTree, l0_pt: &mut [u64]) -> Result<(), vm::Error> {
    let mut rit = regions.iter();
    while let Some((k, v)) = rit.next() {
        let n = v.len / 4096;
        let mut i = 0;
        while i < n {
            let vm = *k + (i * 4096);
            let pm = v2p_pt::<fn(*mut u64)>(l0_pt, vm, None).map_err(|e| e)?;
            let pm_page = pm::lookup(pm);
            if pm_page.is_none() {
                return Err(vm::Error::Inval);
            }
            let pm_page = pm_page.unwrap();
            if pm_page.ref_cnt == 0 {
                return Err(vm::Error::Inval);
            }
            match pm_page.flags {
                pm::Flags::Used | pm::Flags::Cow => {}
                _ => return Err(vm::Error::Inval),
            }
            let pages = pm_page.len() / 4096;
            unmap(l0_pt, vm, pages).map_err(|e| e)?;
            crate::pm::free(pm);
            i += pages;
        }
    }
    Ok(())
}

fn free_task(pid: usize) -> Result<(), vm::Error> {
    let task: &mut Task = &mut TASKS.as_mut()[pid];
    let l0_pt = PmWrap::new(
        task.user_pt.unwrap() as usize, //
        vm::PR_PW,
        false,
    )
    .map_err(|e| e)?;

    free_regions(&mut task.regions, l0_pt.as_slice_mut()).map_err(|e| e)?;

    for i in 0..task.files.len() {
        if let Some(f) = &mut task.files[i] {
            let c = f.close();
            if c.is_err() {}
        }
    }

    free_pt(task.user_pt.unwrap() as u64);
    Ok(())
}

static WAIT: Lock<()> = Lock::new("wait", ());

pub fn exit() -> u64 {
    let task = mycpu().get_task().unwrap();
    if task.pid == 0 {
        panic!(
            "pid 0 tried to exit {}\n",
            task.get_trap_frame().unwrap().regs[0]
        );
    }
    let wait_lock = WAIT.acquire();

    if let Some(p) = task.parent {
        wakeup(p as u64);
    }

    let lock = task.lock.acquire();
    task.exit_code = task.get_trap_frame().unwrap().regs[0];
    free_task(task.pid as usize).unwrap();
    task.state = State::Zombie;
    drop(wait_lock);
    sched();
    let _ = lock;
    0
}

pub fn exit_group() -> u64 {
    exit()
}

pub fn getuid() -> u64 {
    1000
}

pub fn wait() -> u64 {
    let t = mycpu().get_task().unwrap();
    let tf = t.get_trap_frame().unwrap();
    let ptr = t as *const Task;
    let wait_lock = WAIT.acquire();
    let tasks = TASKS.as_mut();
    loop {
        let mut has_child = false;
        for i in 0..tasks.len() {
            let task: &mut Task = &mut tasks[i];
            let l = task.lock.acquire();
            if let Some(parent) = task.parent {
                if parent == ptr {
                    has_child = true;
                    if let State::Zombie = task.state {
                        unsafe {
                            *(tf.regs[0] as *mut u32) = task.exit_code as u32;
                        }
                        task.state = State::Free;
                        task.parent = None;
                        return task.pid as u64;
                    }
                }
            }
            let _ = l;
        }

        if !has_child {
            return !0;
        }

        sleep(ptr as u64, wait_lock.get_lock());
    }
}

fn copy_pm(from_pm: usize, to_pm: usize, n: usize) -> Result<(), ()> {
    for i in 0..n {
        let to = PmWrap::new(to_pm + (4096 * i), vm::PR_PW, true).map_err(|_| ())?;
        let from = PmWrap::new(from_pm + (4096 * i), vm::PR, false).map_err(|_| ())?;
        to.as_slice_mut::<u8>().copy_from_slice(from.as_slice());
    }
    Ok(())
}

pub fn dabt_handler() {
    let task = mycpu().get_task().unwrap();
    let vaddr = r_far_el1() as usize;

    let mut it = task.regions.iter();
    while let Some((k, v)) = it.next() {
        if vaddr >= *k && vaddr < (k + v.len) {
            if v.flags & elf::PF_W > 0 {
                let l0_pt = PmWrap::new(task.user_pt.unwrap() as usize, vm::PR_PW, false);
                if l0_pt.is_err() {
                    return;
                }
                let l0_pt = l0_pt.unwrap();
                let kpm = v2p_pt::<fn(*mut u64)>(l0_pt.as_slice_mut(), *k, None);
                if kpm.is_err() {
                    return;
                }
                let kpm = kpm.unwrap();
                let mut good = false;
                let err = v2p_pt(
                    l0_pt.as_slice_mut(),
                    vaddr,
                    Some(|ptr: *mut u64| {
                        let pm_ = unsafe { *ptr as usize & vm::PHY_MASK };
                        let page = pm::lookup(pm_);
                        if page.is_none() {
                            return;
                        }
                        let page = page.unwrap();
                        let cow = match page.flags {
                            pm::Flags::Mid => {
                                let head = page.get_head();
                                if head.is_none() {
                                    return;
                                }
                                let head = head.unwrap();
                                if let pm::Flags::Cow = head.flags {
                                    // TODO
                                    // assumption: if pm.size>4096 {one region}
                                    let kpage = pm::lookup(kpm);
                                    if kpage.is_none() {
                                        return;
                                    }
                                    let kpage = kpage.unwrap();
                                    assert!(kpage.eql(head));
                                    Some((head, kpm, *k))
                                } else {
                                    None
                                }
                            }
                            pm::Flags::Cow => Some((page, pm_, align_b(vaddr, 4096))),
                            _ => None,
                        };

                        if let Some((cow, pm, vm)) = cow {
                            if cow.ref_cnt == 1 {
                                cow.flags = pm::Flags::Used;
                                if let Err(_) = map_ovwr(
                                    l0_pt.as_slice_mut(),
                                    vm,
                                    pm,
                                    cow.len() / 4096,
                                    vm::PR_PW_UR_UW1,
                                ) {
                                    return;
                                }
                            } else {
                                let new_pm = pm::alloc(cow.len());
                                if new_pm.is_err() {
                                    return;
                                }

                                let new_pm = new_pm.unwrap();
                                let defer = defer(|| {
                                    pm::free(new_pm);
                                });

                                let n = cow.len() / 4096;
                                if copy_pm(pm, new_pm, n).is_err() {
                                    return;
                                }
                                cow.ref_cnt -= 1;
                                if let Err(_) = map_ovwr(
                                    l0_pt.as_slice_mut(),
                                    vm,
                                    new_pm,
                                    cow.len() / 4096,
                                    vm::PR_PW_UR_UW1,
                                ) {
                                    return;
                                }
                                forget(defer);
                            };
                            good = true;
                        }
                    }),
                );
                if err.is_ok() && good {
                    return;
                }
            }
        }
    }
    //TODO segfaultonomy
    print!("======================\n");
    print!("Dabt.. at {:x} pid {}\n", vaddr, task.pid);
    print!("{:?}\n", task.get_trap_frame().unwrap());
    print!("======================\n");
    loop {}
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
                    restore_ttbr0(task.pid as usize, task.user_pt.unwrap() as usize);
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
    let task = cpu.get_task().unwrap();
    assert!(cpu.int_disables == 1);
    assert!(task.lock.holding());
    if let State::Running = task.state {
        panic!("running");
    }
    // go back to sheduler()
    switch(task.ctx.as_mut_ptr(), cpu.shed_ctx.as_ptr());
    restore_ttbr0(task.pid as usize, task.user_pt.unwrap() as usize);
    mycpu().int_enable = cpu.int_enable;
}

pub fn yild() {
    if let Some(task) = mycpu().get_task() {
        let lock = task.lock.acquire(); // re-acquire one released at fork ret
        task.state = State::Ready;
        sched();
        // print!("yield: {}\n", task.pid);
        let _ = lock;
    } else {
        // print!("yield: none\n");
    }
}

pub fn getpid() -> u64 {
    let task = mycpu().get_task().unwrap();
    task.pid as u64
}

fn alloc_pid() -> Option<u16> {
    let tasks = TASKS.as_mut();
    for i in 0..tasks.len() {
        let task = &mut tasks[i];
        let lock = task.lock.acquire();
        if let State::Free = task.state {
            task.state = State::Used;
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

pub fn alloc_task() -> Option<&'static mut Task> {
    let tasks = TASKS.as_mut();
    if let Some(pid) = alloc_pid() {
        let task = &mut tasks[pid as usize];
        task.pid = pid;
        task.init_1(0);
        task.state = State::Used;
        Some(task)
    } else {
        None
    }
}

pub fn create_task(entry: u64) {
    let tasks = TASKS.as_mut();
    let pid = alloc_pid().unwrap();
    let task = &mut tasks[pid as usize];
    task.pid = pid;
    task.init_1(entry);
    task.files[0] = Some(fs::open_cons().unwrap());
    task.files[1] = Some(fs::open_cons().unwrap());
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

    w_tpidrro_el0(0xff0);
    if FIRST.swap(false, Ordering::Release) {
        print!("launching init..\n");
        execv("init", &[], &[]).unwrap();
        task.cwd = Some("/".into());
    }

    unsafe {
        asm!(
            "mov sp, {}",
            "ldp x0, x1, [sp], #16",
            "msr elr_el1, x0",
            "msr sp_el0, x1",
            "ldp x1, x0, [sp], #16",
            "msr spsr_el1, x1",
            "ldp x1, x2, [sp], #16",
            "ldp x3, x4, [sp], #16",
            "ldp x5, x6, [sp], #16",
            "ldp x7, x8, [sp], #16",
            "ldp x9, x10, [sp], #16",
            "ldp x11, x12, [sp], #16",
            "ldp x13, x14, [sp], #16",
            "ldp x15, x16, [sp], #16",
            "ldp x17, x18, [sp], #16",
            "ldp x19, x20, [sp], #16",
            "ldp x21, x22, [sp], #16",
            "ldp x23, x24, [sp], #16",
            "ldp x25, x26, [sp], #16",
            "ldp x27, x28, [sp], #16",
            "ldp x29, x30, [sp], #16",
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
