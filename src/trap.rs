use crate::{
    _boot_stack, _boot_stack_btm, arch,
    heap::SyncUnsafeCell,
    p9, print,
    sched::{self, mycpu},
    svc, timer, uart,
    vm::{self},
    wfi,
};
use core::{
    arch::{asm, naked_asm},
    cell::UnsafeCell,
    fmt::{LowerHex, Write},
};

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Frame {
    pub pc: u64,
    pub sp_el0: u64,
    pub pstate: u64,
    pub regs: [u64; 31],
}

impl Frame {
    pub fn zero(&mut self) {
        self.pc = 0;
        self.sp_el0 = 0;
        self.pstate = 0;
        for i in 0..self.regs.len() {
            self.regs[i] = 0;
        }
    }

    pub fn el(&self) -> u8 {
        ((self.pstate >> 2) & 3) as u8
    }
}

impl LowerHex for Frame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("TF {\n").unwrap();
        f.write_fmt(format_args!("pc: {:x},\n", self.pc)).unwrap();
        f.write_fmt(format_args!("sp_el0: {:x},\n", self.sp_el0))
            .unwrap();
        f.write_fmt(format_args!("pstate: {:x},\n", self.pstate))
            .unwrap();
        for i in 0..self.regs.len() {
            f.write_fmt(format_args!("r{}: {:x},\n", i, self.regs[i]))
                .unwrap();
        }
        f.write_str("}")
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn irq_handler(frame: &Frame) {
    let idx = gic_ack();
    gic_eoi(idx);
    match idx {
        30 => timer::handle_tik(frame.el()),
        33 => uart::handle_rx(),
        78 => p9::irq_handle(),
        _ => {
            print!("unhandled irq: {}\n", idx);
            loop {
                wfi!();
            }
        }
    };
}

#[unsafe(no_mangle)]
pub extern "C" fn sync_handler(frame: &Frame) {
    let task = mycpu().get_task().unwrap();
    task.trapframe = frame as *const Frame as u64;
    let esr = arch::r_esr_el1();
    if esr >> 26 == 0b010101 {
        return svc::handle();
    }
    if esr >> 26 == 0x24 || esr >> 26 == 0x25 {
        return sched::dabt_handler();
    }
    let far = arch::r_far_el1();
    let elr = arch::r_elr_el1();
    print!(
        "sync... pid {} far = 0x{:x} erl: 0x{:x} ret pc: 0x{:x} esr: {:x}\n",
        task.pid,
        far,
        elr,
        frame.pc,
        esr >> 26
    );
    let sp = arch::r_sp();
    let btm = unsafe { (&_boot_stack_btm) as *const u64 as u64 };
    let depth = sp.wrapping_sub(btm);
    print!("kernel stack overflow =  {} depth: {}\n", sp <= btm, depth);
    print!("{:?}\n", frame);
    loop {
        wfi!();
    }
}

#[unsafe(no_mangle)]
#[unsafe(naked)]
#[allow(unused)]
pub extern "C" fn _sync_handler() {
    naked_asm!(
        "stp x29, x30, [sp, #-16]!",
        "stp x27, x28, [sp, #-16]!",
        "stp x25, x26, [sp, #-16]!",
        "stp x23, x24, [sp, #-16]!",
        "stp x21, x22, [sp, #-16]!",
        "stp x19, x20, [sp, #-16]!",
        "stp x17, x18, [sp, #-16]!",
        "stp x15, x16, [sp, #-16]!",
        "stp x13, x14, [sp, #-16]!",
        "stp x11, x12, [sp, #-16]!",
        "stp x9, x10, [sp, #-16]!",
        "stp x7, x8, [sp, #-16]!",
        "stp x5, x6, [sp, #-16]!",
        "stp x3, x4, [sp, #-16]!",
        "stp x1, x2, [sp, #-16]!",
        "mrs x1, spsr_el1",
        "stp x1, x0, [sp, #-16]!",
        "mrs x0, elr_el1",
        "mrs x1, sp_el0",
        "stp x0, x1, [sp, #-16]!",
        "mov x0, sp",
        "bl sync_handler",
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
        "eret"
    );
}

#[unsafe(no_mangle)]
#[unsafe(naked)]
#[allow(unused)]
pub extern "C" fn _irq_handler() {
    naked_asm!(
        "stp x29, x30, [sp, #-16]!",
        "stp x27, x28, [sp, #-16]!",
        "stp x25, x26, [sp, #-16]!",
        "stp x23, x24, [sp, #-16]!",
        "stp x21, x22, [sp, #-16]!",
        "stp x19, x20, [sp, #-16]!",
        "stp x17, x18, [sp, #-16]!",
        "stp x15, x16, [sp, #-16]!",
        "stp x13, x14, [sp, #-16]!",
        "stp x11, x12, [sp, #-16]!",
        "stp x9, x10, [sp, #-16]!",
        "stp x7, x8, [sp, #-16]!",
        "stp x5, x6, [sp, #-16]!",
        "stp x3, x4, [sp, #-16]!",
        "stp x1, x2, [sp, #-16]!",
        "mrs x1, spsr_el1",
        "stp x1, x0, [sp, #-16]!",
        "mrs x0, elr_el1",
        "mrs x1, sp_el0",
        "stp x0, x1, [sp, #-16]!",
        "mov x0, sp",
        "bl irq_handler",
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
        "eret"
    );
}

#[unsafe(no_mangle)]
#[unsafe(naked)]
#[allow(unused)]
pub extern "C" fn _other_handler() {
    naked_asm!("1:", "wfi", "b 1b");
}

#[unsafe(no_mangle)]
#[unsafe(naked)]
#[allow(unused)]
#[unsafe(link_section = ".text.vector")]
pub extern "C" fn trap_vector() {
    naked_asm!(
        "b _sync_handler",
        ".rep 31",
        "nop",
        ".endr",
        "b _irq_handler",
        ".rep 31",
        "nop",
        ".endr",
        "b _other_handler",
        ".rep 31",
        "nop",
        ".endr",
        "b _other_handler",
        ".align 8", //el0
        "b _sync_handler",
        ".rep 31",
        "nop",
        ".endr",
        "b _irq_handler",
        ".rep 31",
        "nop",
        ".endr",
        "b _other_handler",
        ".rep 31",
        "nop",
        ".endr",
        "b _other_handler",
    )
}

static GIC_DIST: SyncUnsafeCell<usize> = SyncUnsafeCell(UnsafeCell::new(0));
//0x8000000;
static GIC_CPU: SyncUnsafeCell<usize> = SyncUnsafeCell(UnsafeCell::new(0));
//0x8010000;

fn gic_cpu() -> usize {
    unsafe { GIC_CPU.0.get().read() }
}

fn gic_dist() -> usize {
    unsafe { GIC_DIST.0.get().read() }
}

#[allow(unused)]
#[unsafe(no_mangle)]
pub fn gic_enable() {
    unsafe {
        let x = (gic_cpu() + 4) as *mut u32;
        x.write_volatile(0xff);

        let x = gic_dist() as *mut u32;
        x.write_volatile(1);

        let x = gic_cpu() as *mut u32;
        x.write_volatile(1);
    };
}

#[allow(unused)]
#[inline]
pub fn gic_ack() -> u32 {
    let ptr = (gic_cpu() + 0xc) as *const u32;
    unsafe { ptr.read_volatile() }
}

#[allow(unused)]
#[inline]
pub fn gic_eoi(idx: u32) {
    let ptr = (gic_cpu() + 0x10) as *mut u32;
    unsafe {
        ptr.write_volatile(idx);
    }
}

#[allow(unused)]
pub fn gic_enable_intr(idx: usize) {
    let back = idx / 32;
    let bit = idx % 32;
    let back_ptr = (gic_dist() + 0x100) as *mut u32;
    unsafe {
        let v = back_ptr.add(back).read_volatile() | (1u32 << bit);
        back_ptr.add(back).write_volatile(v);
    }
}

#[allow(unused)]
pub fn gic_disable_intr(idx: usize) {
    let back = idx / 32;
    let bit = idx % 32;
    let back_ptr = (gic_dist() + 0x180) as *mut u32;
    unsafe {
        let v = back_ptr.add(back).read_volatile() | (1u32 << bit);
        back_ptr.add(back).write_volatile(v);
    }
}

pub fn init() {
    let map = vm::map(0x8000000, 1, vm::PR_PW).unwrap();
    unsafe { GIC_DIST.0.get().write(map) };
    let map = vm::map(0x8010000, 1, vm::PR_PW).unwrap();
    unsafe { GIC_CPU.0.get().write(map) };
    gic_enable();
}
