use alloc::collections::vec_deque::VecDeque;

use crate::{
    elf::PT_LOOS,
    heap::SyncUnsafeCell,
    sched::{sleep, wakeup},
    spin::Lock,
    uart::{self, putc},
};

pub struct File {}

impl File {
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        Ok(read_line(buf))
    }

    pub fn write(&mut self, buf: &[u8]) -> Result<usize, ()> {
        uart::Writer::write_bytes(buf);
        Ok(buf.len())
    }
}

static FILE: SyncUnsafeCell<File> = SyncUnsafeCell::new(File {});

pub fn open() -> &'static mut File {
    FILE.as_mut()
}

static BUF: Lock<VecDeque<u8>> = Lock::new("cons buf", VecDeque::new());

fn put_backspace() {
    putc(8);
    putc(32);
    putc(8);
}

pub fn push_char(c: u8) {
    let lock = BUF.acquire();
    let buf = lock.as_mut();
    match c {
        127 => {
            buf.pop_back();
            put_backspace();
        }
        _ => {
            let c = if c == 13 { 10 } else { c };
            putc(c);
            buf.push_back(c);

            if c == 10 {
                wakeup(&BUF as *const Lock<VecDeque<u8>> as u64);
            }
        }
    }
}

pub fn read_line(buf: &mut [u8]) -> usize {
    let lock = BUF.acquire();
    if buf.len() == 0 {
        return 0;
    }
    let mut i = 0;

    'outer: loop {
        while let Some(c) = lock.as_mut().pop_front() {
            buf[i] = c;
            i += 1;
            if c == 10 || i == buf.len() {
                break 'outer;
            }
        }

        sleep(&BUF as *const Lock<VecDeque<u8>> as u64, lock.get_lock());
    }

    return i;
}
