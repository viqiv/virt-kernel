#![no_std]
#![no_main]

use core::arch::{asm, naked_asm};

mod arch;
mod heap;
mod pm;
mod spin;
mod timer;
mod trap;
mod uart;
mod vm;

#[unsafe(no_mangle)]
fn main(b: usize, e: usize) {
    pm::init(b, e);
    vm::init(b, e);
    heap::init();
    uart::init();

    print!("Hello, World!!\n");

    loop {
        wfi!();
    }
}

unsafe extern "C" {
    static _boot_stack: u64;
    static _trap_vec: u64;
    static _kernel_begin: u64;
    static _kernel_end: u64;
}

#[unsafe(no_mangle)]
#[unsafe(link_section = ".boot.data")]
#[unsafe(naked)]
pub extern "C" fn _data() {
    naked_asm!(
        ".align 12",
        "l0_id: .8byte 0",
        ".align 12",
        "l0_h: .8byte 0",
        ".align 12",
        "l1_id0: .8byte 0",
        "l1_id1: .8byte 0",
        ".align 12",
        "l1_h0: .8byte 0",
        "l1_h1: .8byte 0",
    )
}

#[unsafe(no_mangle)]
#[unsafe(link_section = ".boot.text")]
#[unsafe(naked)]
pub extern "C" fn _start() {
    naked_asm!(
       "ldr x0, =0x5b0103210",
       "msr tcr_el1, x0",
       "ldr x0, =l0_id",
       "msr ttbr0_el1, x0",
       "ldr x1, =l0_h",
       "msr ttbr1_el1, x1",
       "ldr x2, =l1_id0",
       "ldr x3, =0x40000401",
       "str x3, [x2, #8]",
       "orr x2, x2, #3",
       "str x2, [x0]",
       "ldr x2, =l1_h0",
       "str x3, [x2, #8]",
       "orr x2, x2, #3",
       "str x2, [x1]",
       "mov x0, #0xff",
       "msr mair_el1, x0",
       "dsb sy",
       "isb sy",
       "mrs x0, sctlr_el1",
       "orr x0, x0, #1",
       "msr sctlr_el1, x0",
        "isb sy",
       "ldr x0, ={stack}",
       "mov sp, x0",
       "ldr x0, ={trap}",
       "msr vbar_el1, x0",
       "ldr x0, ={begin}",
       "ldr x1, ={end}",
       "bl main",
       "1:",
       "wfi",
       "b 1b",
        stack = sym _boot_stack,
        trap = sym _trap_vec,
        begin = sym _kernel_begin,
        end = sym _kernel_end,
    );
}

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

// MSR <Special-purpose_register>, Xt ; Write to Special-purpose register
// MRS Xt, <Special-purpose_register> ; Read from Special-purpose register
