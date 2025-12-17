use core::{
    ffi::{c_int, c_long, c_uint, c_ulong},
    marker::PhantomData,
    sync::atomic::{AtomicU16, Ordering},
};

use alloc::{str, string::String};

use crate::{
    cons::{self},
    heap::SyncUnsafeCell,
    p9, print,
    sched::mycpu,
    spin::Lock,
    stuff::{as_slice, as_slice_mut, cstr_as_slice},
};

pub enum FileKind {
    None,
    Used,
    P9(&'static mut p9::File),
    Cons(&'static mut cons::File),
}

pub struct File {
    kind: FileKind,
    rc: AtomicU16,
    offt: usize,
}

impl File {
    pub const fn zeroed() -> File {
        File {
            kind: FileKind::None,
            rc: AtomicU16::new(0),
            offt: 0,
        }
    }

    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        if self.rc.load(Ordering::Acquire) == 0 {
            return Err(());
        }
        match &mut self.kind {
            FileKind::P9(p9f) => {
                if let Ok(n) = p9f.read(buf, self.offt) {
                    self.offt = self.offt.wrapping_add(n);
                    Ok(n)
                } else {
                    Err(())
                }
            }
            FileKind::Cons(c) => c.read(buf),
            _ => {
                panic!("read: unhandled file kind.")
            }
        }
    }

    pub fn write(&mut self, buf: &[u8]) -> Result<usize, ()> {
        if self.rc.load(Ordering::Acquire) == 0 {
            return Err(());
        }
        match &mut self.kind {
            FileKind::P9(p9f) => {
                if let Ok(n) = p9f.write(buf, self.offt) {
                    self.offt = self.offt.wrapping_add(n);
                    Ok(n)
                } else {
                    Err(())
                }
            }
            FileKind::Cons(c) => c.write(buf),
            _ => {
                panic!("write: unhandled file kind.")
            }
        }
    }

    pub fn close(&mut self) -> Result<(), ()> {
        if self.rc.load(Ordering::Acquire) == 0 {
            return Ok(());
        }

        if let Ok(0) = self.rc.compare_exchange(
            1,
            0,
            Ordering::AcqRel, //
            Ordering::Relaxed,
        ) {
            match &self.kind {
                FileKind::P9(p9f) => {
                    return if let Ok(_) = p9f.close() {
                        self.kind = FileKind::None;
                        Ok(())
                    } else {
                        self.rc.fetch_add(1, Ordering::Release);
                        Err(())
                    };
                }
                _ => panic!("write: unhandled file kind."),
            }
        } else {
            self.rc.fetch_sub(1, Ordering::Release);
        }

        Ok(())
    }

    pub fn seek_to(&mut self, offt: usize) {
        self.offt = offt;
    }

    pub fn seek_by(&mut self, offt: i32) {
        self.offt = if offt > 0 {
            self.offt.wrapping_add(offt as usize)
        } else {
            self.offt.wrapping_sub(offt as usize)
        };
    }

    pub fn dup(&mut self) -> Option<&'static mut Self> {
        self.rc.fetch_add(1, Ordering::Release);
        unsafe { (self as *const Self as *mut Self).as_mut() }
    }

    pub fn read_all(&mut self, mut buf: &mut [u8]) -> Result<(), ()> {
        while buf.len() > 0 {
            let n = self.read(buf).map_err(|_| ())?;
            buf = &mut buf[n..];
        }
        Ok(())
    }
}

const NFILES: usize = 128;

struct Fs {
    files: [File; NFILES],
}

pub fn open(path: &str, mode: u32) -> Result<&'static mut File, ()> {
    if let Some((idx, file)) = alloc_file() {
        return if let Ok(p9file) = p9::open(path, mode) {
            file.kind = FileKind::P9(p9file);
            file.rc = AtomicU16::new(1);
            Ok(file)
        } else {
            free_file(idx);
            Err(())
        };
    }

    Err(())
}

pub fn open_cons() -> Result<&'static mut File, ()> {
    if let Some((_, file)) = alloc_file() {
        file.kind = FileKind::Cons(cons::open());
        file.rc = AtomicU16::new(1);
        Ok(file)
    } else {
        Err(())
    }
}

pub fn sys_write() -> u64 {
    let task = mycpu().get_task().unwrap();
    let tf = task.get_trap_frame().unwrap();
    let fd = tf.regs[0] as usize;
    if fd >= task.files.len() {
        return !0;
    }

    if task.files[fd].is_none() {
        return !0;
    }

    let len = tf.regs[2] as usize;
    let ptr = tf.regs[1];

    let file = task.files[fd].as_mut().unwrap();

    if ptr == 0 {
        return !0;
    }
    // i trust you user
    let buf = as_slice(ptr as *const u8, len);
    if let Ok(n) = file.write(buf) {
        n as u64
    } else {
        !0
    }
}

pub fn sys_read() -> u64 {
    let task = mycpu().get_task().unwrap();
    let tf = task.get_trap_frame().unwrap();
    let fd = tf.regs[0] as usize;
    if fd >= task.files.len() {
        return !0;
    }

    if task.files[fd].is_none() {
        return !0;
    }

    let len = tf.regs[2] as usize;
    let ptr = tf.regs[1];

    let file = task.files[fd].as_mut().unwrap();

    if ptr == 0 {
        return !0;
    }
    // i trust you user
    let buf = as_slice_mut(ptr as *mut u8, len);
    if let Ok(n) = file.read(buf) {
        n as u64
    } else {
        !0
    }
}

pub fn readlinkat() -> u64 {
    let task = mycpu().get_task().unwrap();
    let tf = task.get_trap_frame().unwrap();

    let fd = tf.regs[0];

    let path = cstr_as_slice(tf.regs[1] as *const u8);
    let path_str = String::from(str::from_utf8(path).unwrap());

    let real_path = if fd == AT_FDCWD as u64 && !path_str.starts_with("/") {
        let mut cwd = task.cwd.as_ref().unwrap().clone();
        cwd.push_str(&path_str);
        cwd
    } else {
        path_str
    };

    !0
}

pub fn getrandom() -> u64 {
    let task = mycpu().get_task().unwrap();
    let tf = task.get_trap_frame().unwrap();
    tf.regs[1]
}

pub fn fstat() -> u64 {
    !0
}

pub fn lseek() -> u64 {
    !0
}

pub fn ioctl() -> u64 {
    !0
}

pub fn openat() -> u64 {
    !0
}

pub fn fcntl() -> u64 {
    0
}

pub fn ppoll() -> u64 {
    !0
}

pub fn close() -> u64 {
    !0
}

pub fn dup3() -> u64 {
    let task = mycpu().get_task().unwrap();
    let tf = task.get_trap_frame().unwrap();

    let old_fd = tf.regs[0] as usize;
    let new_fd = tf.regs[1] as usize;

    if task.files[old_fd].is_none() {
        return !0;
    }

    if task.files[new_fd].is_some() {
        task.files[old_fd].as_mut().unwrap().close().unwrap();
    }

    let file = task.files[old_fd].as_mut().unwrap();
    task.files[new_fd] = file.dup();

    0
}

pub const AT_SYMLINK_NOFOLLOW: u32 = 256;

pub fn newfsstatat() -> u64 {
    let task = mycpu().get_task().unwrap();
    let tf = task.get_trap_frame().unwrap();

    let fd = tf.regs[0];

    let path = cstr_as_slice(tf.regs[1] as *const u8);
    let path_str = String::from(str::from_utf8(path).unwrap());

    let real_path = if fd == AT_FDCWD as u64 && !path_str.starts_with("/") {
        let mut cwd = task.cwd.as_ref().unwrap().clone();
        cwd.push_str(&path_str);
        cwd
    } else {
        path_str
    };

    // print!("=====================fstat {} {}\n", real_path, tf.regs[3]);
    match p9::stat(&real_path, tf.regs[3] as u32 & AT_SYMLINK_NOFOLLOW > 0) {
        Ok(s) => {
            let stat = unsafe { (tf.regs[2] as *mut stat).as_mut() }.unwrap();
            stat.st_dev = s.dev as u64;
            stat.st_size = s.len as i64;
            stat.st_atime = s.atime as i64;
            stat.st_mtime = s.mtime as i64;
            stat.st_mode = s.mode;
            stat.st_uid = 0;
            stat.st_gid = 0;
            stat.st_nlink = 2;
            stat.st_blocks = 8;
            stat.st_blksize = 4096;
            stat.st_ctime = 0;
            return 0;
        }
        _ => return !0,
    }

    // panic!("fd = {}\n", fd);
}

pub const AT_FDCWD: i32 = -100;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct stat {
    pub st_dev: c_ulong,
    pub st_ino: c_ulong,
    pub st_mode: c_uint,
    pub st_nlink: c_uint,
    pub st_uid: c_uint,
    pub st_gid: c_uint,
    pub st_rdev: c_ulong,
    pub __pad1: c_ulong,
    pub st_size: c_long,
    pub st_blksize: c_int,
    pub __pad2: c_int,
    pub st_blocks: c_long,
    pub st_atime: c_long,
    pub st_atime_nsec: c_ulong,
    pub st_mtime: c_long,
    pub st_mtime_nsec: c_ulong,
    pub st_ctime: c_long,
    pub st_ctime_nsec: c_ulong,
    pub __unused4: c_uint,
    pub __unused5: c_uint,
}

static FS: Lock<Fs> = Lock::new(
    "fs",
    Fs {
        files: [
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
        ],
    },
);

fn alloc_file() -> Option<(usize, &'static mut File)> {
    let lock = FS.acquire();
    let fs = lock.as_mut();

    for i in 0..fs.files.len() {
        let file = &mut fs.files[i];
        if let FileKind::None = file.kind {
            file.kind = FileKind::Used;
            let steal = unsafe { (file as *mut File).as_mut() }.unwrap();
            return Some((i, steal));
        }
    }

    None
}

fn free_file(idx: usize) {
    let lock = FS.acquire();
    let fs = lock.as_mut();
    fs.files[idx].kind = FileKind::None;
}
