use core::cell::UnsafeCell;

use crate::{heap::SyncUnsafeCell, trap::gic_enable_intr, vm};

static MAP: SyncUnsafeCell<usize> = SyncUnsafeCell(UnsafeCell::new(0));

// static LOCK: spin::Lock<()> = spin::Lock::new("uart", ());

#[inline]
fn write_char(c: u8, map: usize) {
    let dr = map as *mut u8;
    unsafe { dr.write_volatile(c) };
}

fn write_bytes(b: &[u8], map: usize) {
    for i in 0..b.len() {
        write_char(b[i], map);
    }
}

pub struct Writer;

impl core::fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        write_bytes(s.as_bytes(), unsafe { MAP.0.get().read() });
        Ok(())
    }
}

#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        let mut stdout = $crate::uart::Writer;
        core::fmt::write(&mut stdout, format_args!($($arg)*)).unwrap();
    }};
}

pub fn init_tx() {
    let v = vm::map_4k(0x9000000, vm::PR_PW).unwrap();
    unsafe { MAP.0.get().write(v) };
    enable_tx(v);
}

pub fn init_rx() {
    enable_rx(unsafe { MAP.0.get().read() });
}

pub fn enable_rx(map: usize) {
    unsafe {
        let base = (map + 0x30) as *mut u32;
        let cr = base.read_volatile() | 1u32 << 9;
        base.write_volatile(cr);
        let base = (map + 0x38) as *mut u32;
        let cr = base.read_volatile() | 1u32 << 4;
        base.write_volatile(cr);
    }
    gic_enable_intr(33);
}

pub fn enable_tx(map: usize) {
    let base = (map + 0x30) as *mut u32;
    unsafe {
        let cr = base.read_volatile() | 0x101;
        base.write_volatile(cr);
    }
}

fn clr_rx() {
    let base = (unsafe { MAP.0.get().read() } + 0x44) as *mut u32;
    unsafe {
        let cr = 1u32 << 4;
        base.write_volatile(cr);
    }
}

fn read() -> u8 {
    let dr = (unsafe { MAP.0.get().read() }) as *const u8;
    unsafe { *dr }
}

pub fn handle_rx() {
    let c = read();
    print!("uart... {}\n", c as char);
    // print!("{:?}\n", frame);
    clr_rx();
}
