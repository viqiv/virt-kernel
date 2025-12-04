#![no_std]
#![no_main]

extern crate alloc;

use core::arch::{asm, naked_asm};

use alloc::boxed::Box;

use crate::{arch::r_sp, stuff::StaticMut};

mod arch;
mod heap;
mod pm;
mod spin;
mod stuff;
mod timer;
mod trap;
mod uart;
mod virtio;
mod vm;

static BUF: StaticMut<[u8; 512]> = StaticMut::new([0; 512]);

#[unsafe(no_mangle)]
fn main(b: usize, e: usize) {
    pm::init(b, e);
    vm::init(b, e);
    heap::init();
    uart::init_tx();
    trap::init();
    uart::init_rx();
    timer::init();
    virtio::init();

    print!(
        "kernel stack top 0x{:x} bottom 0x{:x} current sp 0x{:x}\n",
        unsafe { (&_boot_stack) as *const u64 as usize },
        unsafe { (&_boot_stack_btm) as *const u64 as usize },
        r_sp()
    );
    // arch::pstate_i_clr();

    virtio::blk::read_sync(0, BUF.get_mut()).unwrap();
    for i in 0..512 {
        let c = BUF.get()[i] as char;
        print!("{}.", c);
    }
    // virtio::blk::write_sync(0, &mut buf).unwrap();
    // virtio::blk::write_sync(1, &mut buf).unwrap();
    // virtio::blk::write_sync(2, &mut buf).unwrap();
    // virtio::blk::write_sync(3, &mut buf).unwrap();
    // virtio::blk::write_sync(4, &mut buf).unwrap();
    // virtio::blk::write_sync(5, &mut buf).unwrap();
    // virtio::blk::write_sync(6, &mut buf).unwrap();
    // virtio::blk::write_sync(7, &mut buf).unwrap();
    // virtio::blk::write_sync(8, &mut buf).unwrap();
    // virtio::blk::write_sync(9, &mut buf).unwrap();
    // virtio::blk::write_sync(10, &mut buf).unwrap();

    loop {
        wfi!();
    }
}

unsafe extern "C" {
    pub static _boot_stack: u64;
    pub static _boot_stack_btm: u64;
    pub static _trap_vec: u64;
    pub static _kernel_begin: u64;
    pub static _kernel_end: u64;
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
fn panic(info: &core::panic::PanicInfo) -> ! {
    print!("{}\n", info);
    loop {
        wfi!();
    }
}

// MSR <Special-purpose_register>, Xt ; Write to Special-purpose register
// MRS Xt, <Special-purpose_register> ; Read from Special-purpose register
