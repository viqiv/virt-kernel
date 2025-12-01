use crate::arch;
use crate::print;
use crate::timer;
use crate::uart;
use core::arch::{asm, naked_asm};

#[derive(Debug)]
#[repr(C)]
pub struct Frame {
    pc: u64,
    pstate: u64,
    regs: [u64; 31],
}

#[unsafe(no_mangle)]
pub extern "C" fn irq_handler(frame: &Frame) {
    let idx = gic_ack();
    match idx {
        30 => {
            let freq = timer::r_freq();
            print!("timer... {:x}\n", freq);
            timer::w_ptval_el0(freq / 10);
        }
        33 => {
            let c = uart::read();
            print!("uart... {} {}\n", idx, c);
            // print!("{:?}\n", frame);
            uart::clr_rx();
        }
        _ => unreachable!(),
    };
    gic_eoi(idx);
}

#[unsafe(no_mangle)]
pub extern "C" fn sync_handler(frame: &Frame) {
    // print!(
    //     "sync... 0b{:b} 0x{:x}\n",
    //     arch::r_esr_el1() >> 26,
    //     arch::r_elr_el1()
    // );
    // print!("{:?}\n", frame);
    loop {
        // unsafe {
        //     asm!("wfi");
        // }
    }
}

#[unsafe(no_mangle)]
#[unsafe(naked)]
#[allow(unused)]
pub extern "C" fn _sync_handler() {
    naked_asm!(
        "stp x1, x0, [sp, #-16]!",
        "stp x3, x2, [sp, #-16]!",
        "stp x5, x4, [sp, #-16]!",
        "stp x7, x6, [sp, #-16]!",
        "stp x9, x8, [sp, #-16]!",
        "stp x11, x10, [sp, #-16]!",
        "stp x13, x12, [sp, #-16]!",
        "stp x15, x14, [sp, #-16]!",
        "stp x17, x16, [sp, #-16]!",
        "stp x19, x18, [sp, #-16]!",
        "stp x21, x20, [sp, #-16]!",
        "stp x23, x22, [sp, #-16]!",
        "stp x25, x24, [sp, #-16]!",
        "stp x27, x26, [sp, #-16]!",
        "stp x29, x28, [sp, #-16]!",
        "mrs x0, spsr_el1",
        "stp x0, x30, [sp, #-16]!",
        "mrs x0, elr_el1",
        "str x0, [sp, #-8]!",
        "mov x0, sp",
        "bl sync_handler",
        "ldr x0, [sp], #8",
        "msr elr_el1, x0",
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
        "eret"
    );
}

#[unsafe(no_mangle)]
#[unsafe(naked)]
#[allow(unused)]
pub extern "C" fn _irq_handler() {
    naked_asm!(
        "stp x1, x0, [sp, #-16]!",
        "stp x3, x2, [sp, #-16]!",
        "stp x5, x4, [sp, #-16]!",
        "stp x7, x6, [sp, #-16]!",
        "stp x9, x8, [sp, #-16]!",
        "stp x11, x10, [sp, #-16]!",
        "stp x13, x12, [sp, #-16]!",
        "stp x15, x14, [sp, #-16]!",
        "stp x17, x16, [sp, #-16]!",
        "stp x19, x18, [sp, #-16]!",
        "stp x21, x20, [sp, #-16]!",
        "stp x23, x22, [sp, #-16]!",
        "stp x25, x24, [sp, #-16]!",
        "stp x27, x26, [sp, #-16]!",
        "stp x29, x28, [sp, #-16]!",
        "mrs x0, spsr_el1",
        "stp x0, x30, [sp, #-16]!",
        "mrs x0, elr_el1",
        "str x0, [sp, #-8]!",
        "mov x0, sp",
        "bl irq_handler",
        "ldr x0, [sp], #8",
        "msr elr_el1, x0",
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
    )
}

const GIC_DIST: usize = 0x8000000;
const GIC_CPU: usize = 0x8010000;

#[allow(unused)]
pub fn gic_enable() {
    unsafe {
        let x = (GIC_CPU + 4) as *mut u32;
        x.write_volatile(0xff);

        let x = GIC_DIST as *mut u32;
        x.write_volatile(1);

        let x = GIC_CPU as *mut u32;
        x.write_volatile(1);
    };
}

#[allow(unused)]
#[inline]
pub fn gic_ack() -> u32 {
    let ptr = (GIC_CPU + 0xc) as *const u32;
    unsafe { ptr.read_volatile() }
}

#[allow(unused)]
#[inline]
pub fn gic_eoi(idx: u32) {
    let ptr = (GIC_CPU + 0x10) as *mut u32;
    unsafe {
        ptr.write_volatile(idx);
    }
}

#[allow(unused)]
pub fn gic_enable_intr(idx: usize) {
    let back = idx / 32;
    let bit = idx % 32;
    let back_ptr = (GIC_DIST + 0x100) as *mut u32;
    unsafe {
        let v = back_ptr.add(back).read_volatile() | (1u32 << bit);
        back_ptr.add(back).write_volatile(v);
    }
}

#[allow(unused)]
pub fn gic_disable_intr(idx: usize) {
    let back = idx / 32;
    let bit = idx % 32;
    let back_ptr = (GIC_DIST + 0x180) as *mut u32;
    unsafe {
        let v = back_ptr.add(back).read_volatile() | (1u32 << bit);
        back_ptr.add(back).write_volatile(v);
    }
}
