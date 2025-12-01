use crate::{spin, vm};

struct Data {
    map: usize,
}

static LOCK: spin::Lock<Data> = spin::Lock::new("uart", Data { map: 0 });

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
        let lock = LOCK.acquire();
        write_bytes(s.as_bytes(), lock.as_ref().map);
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

pub fn init() {
    let v = vm::map_4k(0x9000000).unwrap();
    let lock = LOCK.acquire();
    lock.as_mut().map = v;
    enable_tx(v);
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
}
pub fn enable_tx(map: usize) {
    let base = (map + 0x30) as *mut u32;
    unsafe {
        let cr = base.read_volatile() | 0x101;
        base.write_volatile(cr);
    }
}

pub fn clr_rx() {
    let base = (0x9000000usize + 0x44) as *mut u32;
    unsafe {
        let cr = 1u32 << 4;
        base.write_volatile(cr);
    }
}

pub fn read() -> u8 {
    let dr = 0x9000000usize as *const u8;
    unsafe { *dr }
}
