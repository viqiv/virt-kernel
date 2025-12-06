#![no_std]
#![no_main]

extern crate alloc;

use core::{
    arch::{asm, naked_asm},
    cell::UnsafeCell,
};

use crate::{heap::SyncUnsafeCell, virtio::p9};

mod arch;
mod blk;
mod heap;
mod pm;
mod rng;
mod spin;
mod stuff;
mod timer;
mod trap;
mod uart;
mod virtio;
mod vm;

static BUF: SyncUnsafeCell<[u8; 512]> = SyncUnsafeCell(UnsafeCell::new([0; 512]));

#[unsafe(no_mangle)]
fn main(b: usize, e: usize) {
    pm::init(b, e);
    vm::init(b, e);
    uart::init_tx();
    heap::init();
    trap::init();
    uart::init_rx();
    timer::init();
    // arch::pstate_i_clr();
    virtio::init();

    let (fid, _) = virtio::p9::walk("/disas.txt").unwrap();
    // print!("FID = {:?}\n", fid);
    let qid = virtio::p9::open(fid, p9::O::RDWR as u32).unwrap();
    print!("QID = {:?}\n", qid);
    let n = p9::write(fid, "12345678910\n".as_bytes(), 0).unwrap();
    print!("N = {}\n", n);
    let n = p9::write(fid, "qweertyuiop".as_bytes(), 11).unwrap();
    print!("N = {}\n", n);

    // p9::remove(fid).unwrap();
    // p9::clunk(fid).unwrap();
    // p9::create(fid, "foxx", 0, p9::O::RDWR as u32, 1000).unwrap();
    // p9::mkdir(fid, "foxxx", p9::O::RDWR as u32, 1000).unwrap();
    // let n = p9::write(fid, "chapa ilale".as_bytes(), 0).unwrap();
    // print!("N = {}\n", n);
    let buf = unsafe { BUF.0.get().as_mut() }.unwrap();
    let n = p9::readdir(fid, buf, 0).unwrap();
    print!("N = {}\n", n);

    // print!(
    //     "kernel stack top 0x{:x} bottom 0x{:x} current sp 0x{:x}\n",
    //     unsafe { (&_boot_stack) as *const u64 as usize },
    //     unsafe { (&_boot_stack_btm) as *const u64 as usize },
    //     r_sp()
    // );

    // blk::read_sync(0, BUF.get_mut()).unwrap();
    // blk::read_sync(1, BUF.get_mut()).unwrap();
    // blk::read_sync(2, BUF.get_mut()).unwrap();
    // blk::read_sync(3, BUF.get_mut()).unwrap();
    // blk::read_sync(4, BUF.get_mut()).unwrap();
    // blk::read_sync(5, BUF.get_mut()).unwrap();
    // blk::read_sync(6, BUF.get_mut()).unwrap();
    // blk::read_sync(7, BUF.get_mut()).unwrap();
    // blk::read_sync(8, BUF.get_mut()).unwrap();
    // blk::read_sync(10, BUF.get_mut()).unwrap();
    // blk::read_sync(0, BUF.get_mut()).unwrap();
    // rng::read_sync(BUF.get_mut()).unwrap();
    // rng::read_sync(BUF.get_mut()).unwrap();
    // rng::read_sync(BUF.get_mut()).unwrap();
    // rng::read_sync(BUF.get_mut()).unwrap();
    // rng::read_sync(BUF.get_mut()).unwrap();
    // rng::read_sync(BUF.get_mut()).unwrap();
    // rng::read_sync(BUF.get_mut()).unwrap();
    // rng::read_sync(BUF.get_mut()).unwrap();

    for i in 0..n as usize {
        print!("{}", buf[i] as char);
    }

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
