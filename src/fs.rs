use core::{
    cmp::min,
    ffi::{c_int, c_long, c_uint, c_ulong},
    marker::PhantomData,
    sync::atomic::{AtomicU16, Ordering},
};

use alloc::{str, string::String, vec::Vec};

use crate::{
    cons::{self},
    heap::SyncUnsafeCell,
    p9, print, ptr2mut,
    sched::mycpu,
    spin::Lock,
    stuff::{as_slice, as_slice_mut, cstr_as_slice},
    tty::{self, Termios, Winsize},
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
    offt: u64,
    path: Option<String>,
}

impl File {
    pub const fn zeroed() -> File {
        File {
            kind: FileKind::None,
            rc: AtomicU16::new(0),
            offt: 0,
            path: None,
        }
    }

    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        if self.rc.load(Ordering::Acquire) == 0 {
            return Err(());
        }
        match &mut self.kind {
            FileKind::P9(p9f) => {
                if let Ok(n) = p9f.read(buf, self.offt as usize) {
                    self.offt = self.offt.wrapping_add(n as u64);
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
                if let Ok(n) = p9f.write(buf, self.offt as usize) {
                    self.offt = self.offt.wrapping_add(n as u64);
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

        if let Ok(1) = self.rc.compare_exchange(
            1,
            0,
            Ordering::AcqRel, //
            Ordering::Relaxed,
        ) {
            match &self.kind {
                FileKind::P9(p9f) => {
                    return if let Ok(_) = p9f.close() {
                        print!(
                            "CLOSE: {} {:?} {}\n",
                            self.rc.load(Ordering::Acquire),
                            self.path,
                            p9f.fid
                        );
                        self.kind = FileKind::None;
                        self.path = None;
                        Ok(())
                    } else {
                        self.rc.fetch_add(1, Ordering::Release);
                        Err(())
                    };
                }
                FileKind::Cons(cons) => {}
                _ => panic!("write: unhandled file kind."),
            }
        } else {
            self.rc.fetch_sub(1, Ordering::Release);
        }

        Ok(())
    }

    pub fn seek_to(&mut self, offt: usize) {
        self.offt = offt as u64;
    }

    pub fn seek_by(&mut self, offt: i32) {
        self.offt = if offt > 0 {
            self.offt.wrapping_add(offt as u64)
        } else {
            self.offt.wrapping_sub(offt as u64)
        };
    }

    pub fn dup(&mut self) -> Option<&'static mut Self> {
        self.rc.fetch_add(1, Ordering::Release);
        print!(
            "DUP: {:?} rc {}\n",
            self.path,
            self.rc.load(Ordering::Relaxed)
        );
        unsafe { (self as *const Self as *mut Self).as_mut() }
    }

    pub fn read_all(&mut self, mut buf: &mut [u8]) -> Result<(), ()> {
        while buf.len() > 0 {
            let n = self.read(buf).map_err(|_| ())?;
            if n == 0 {
                break;
            }
            buf = &mut buf[n..];
        }
        Ok(())
    }

    pub fn write_all(&mut self, mut buf: &[u8]) -> Result<(), ()> {
        while buf.len() > 0 {
            let n = self.write(buf).map_err(|_| ())?;
            buf = &buf[n..];
        }
        Ok(())
    }

    pub fn fstat(&self, stat: &mut Stat) -> Result<(), ()> {
        match &self.kind {
            FileKind::P9(p9) => p9.stat(stat),
            FileKind::Cons(c) => c.stat(stat),
            FileKind::None => panic!("fstat: none"),
            FileKind::Used => panic!("fstat: used"),
            _ => panic!("fstat: unhandled file kind."),
        }
    }

    pub fn getdents64(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        match &self.kind {
            FileKind::P9(p9) => {
                if let Ok((n, offt)) = p9.getdents64(buf, self.offt) {
                    self.offt = offt as u64;
                    Ok(n)
                } else {
                    Err(())
                }
            }
            _ => panic!("fstat: unhandled file kind."),
        }
    }

    pub fn send(&mut self, to: &mut File, n: usize) -> Result<usize, ()> {
        let vec = Vec::<u8>::with_capacity(4096);
        let buf = ptr2mut!(vec.as_ptr(), [u8; 4096]);

        let mut rem = n;
        while rem > 0 {
            let amt = min(rem, 4096);
            let r = self.read(&mut buf[0..amt]).map_err(|_| ())?;
            if r == 0 {
                break;
            }
            to.write_all(&buf[0..r]).map_err(|_| ())?;
            rem -= r;
        }

        Ok(n - rem)
    }
}

const NFILES: usize = 128;

struct Fs {
    files: [File; NFILES],
}

pub fn open(path: &str, flags: u32, _: u32) -> Result<&'static mut File, ()> {
    if let Some((idx, file)) = alloc_file() {
        return if let Ok(p9file) = p9::open(path, flags) {
            print!("OPEN: path {} fid = {}\n", path, p9file.fid);
            file.kind = FileKind::P9(p9file);
            file.rc = AtomicU16::new(1);
            file.path = Some(String::from(path));
            file.offt = 0;
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

struct IOvec {
    ptr: *mut u8,
    len: usize,
}

pub fn sys_writev() -> u64 {
    let task = mycpu().get_task().unwrap();
    let tf = task.get_trap_frame().unwrap();
    let fd = tf.regs[0] as usize;
    if fd >= task.files.len() {
        return !0;
    }

    if task.files[fd].is_none() {
        return !0;
    }

    let iovec_len = tf.regs[2] as usize;
    let ptr = tf.regs[1];

    let file = task.files[fd].as_mut().unwrap();

    if ptr == 0 {
        return !0;
    }

    let iovec_buf = as_slice(ptr as *const IOvec, iovec_len);

    let mut written = 0;
    for i in 0..iovec_len {
        let iovec = &iovec_buf[i];
        let buf = as_slice(iovec.ptr, iovec.len);
        if let Ok(n) = file.write(buf) {
            written += n as u64
        } else {
            return !0;
        }
    }
    written
}

pub fn getdents64() -> u64 {
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

    let buf = as_slice_mut(ptr as *mut u8, len);
    if let Ok(n) = file.getdents64(buf) {
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

pub fn lseek() -> u64 {
    !0
}

pub struct T;
impl T {
    pub const CGETS: u64 = 0x5401;
    pub const CSETS: u64 = 0x5402;
    pub const CSETSW: u64 = 0x5403;
    pub const CSETSF: u64 = 0x5404;
    pub const CGETA: u64 = 0x5405;
    pub const CSETA: u64 = 0x5406;
    pub const CSETAW: u64 = 0x5407;
    pub const CSETAF: u64 = 0x5408;
    pub const CSBRK: u64 = 0x5409;
    pub const CXONC: u64 = 0x540a;
    pub const CFLSH: u64 = 0x540b;
    pub const IOCEXCL: u64 = 0x540c;
    pub const IOCNXCL: u64 = 0x540d;
    pub const IOCSCTTY: u64 = 0x540e;
    pub const IOCGPGRP: u64 = 0x540f;
    pub const IOCSPGRP: u64 = 0x5410;
    pub const IOCOUTQ: u64 = 0x5411;
    pub const IOCSTI: u64 = 0x5412;
    pub const IOCGWINSZ: u64 = 0x5413;
    pub const IOCSWINSZ: u64 = 0x5414;
    pub const IOCMGET: u64 = 0x5415;
    pub const IOCMBIS: u64 = 0x5416;
    pub const IOCMBIC: u64 = 0x5417;
    pub const IOCMSET: u64 = 0x5418;
    pub const IOCGSOFTCAR: u64 = 0x5419;
    pub const IOCSSOFTCAR: u64 = 0x541a;
    pub const FIONREAD: u64 = 0x541b;
    pub const IOCINQ: u64 = T::FIONREAD;
    pub const IOCLINUX: u64 = 0x541c;
    pub const IOCCONS: u64 = 0x541d;
    pub const IOCGSERIAL: u64 = 0x541e;
    pub const IOCSSERIAL: u64 = 0x541f;
    pub const IOCPKT: u64 = 0x5420;
    pub const FIONBIO: u64 = 0x5421;
    pub const IOCNOTTY: u64 = 0x5422;
    pub const IOCSETD: u64 = 0x5423;
    pub const IOCGETD: u64 = 0x5424;
    pub const CSBRKP: u64 = 0x5425;
    pub const IOCSBRK: u64 = 0x5427;
    pub const IOCCBRK: u64 = 0x5428;
    pub const IOCGSID: u64 = 0x5429;
}

pub fn ioctl() -> u64 {
    let task = mycpu().get_task().unwrap();
    let tf = task.get_trap_frame().unwrap();

    print!("IOCTL {:x} 0x{:x}\n", tf.regs[0], tf.regs[1]);

    match (tf.regs[1]) {
        T::CGETS => tty::get_termios(tf.regs[2] as *mut Termios),
        T::CSETS => tty::set_termios(tf.regs[2] as *const Termios),
        T::IOCGWINSZ => tty::get_winsz(tf.regs[2] as *mut Winsize),
        T::IOCGPGRP => {
            unsafe { *(tf.regs[2] as *mut u32) = task.pid as u32 };
            0
        }
        T::IOCSPGRP => 0,
        x => panic!("unimplemented ioctl 0x{:x}", x),
    }
}

pub struct O;
impl O {
    pub const RDONLY: u32 = 0;
    pub const WRONLY: u32 = 1 << 0;
    pub const RDWR: u32 = 1 << 1;
    pub const CREAT: u32 = 1 << 6;
    pub const EXCL: u32 = 1 << 7;
    pub const NOCTTY: u32 = 1 << 8;
    pub const TRUNC: u32 = 1 << 9;
    pub const APPEND: u32 = 1 << 10;
    pub const NONBLOCK: u32 = 1 << 11;
    pub const DSYNC: u32 = 1 << 12;
    pub const ASYNC: u32 = 1 << 13;
    pub const DIRECTORY: u32 = 1 << 14;
    pub const NOFOLLOW: u32 = 1 << 15;
    pub const DIRECT: u32 = 1 << 16;
    pub const LARGEFILE: u32 = 1 << 17;
    pub const NOATIME: u32 = 1 << 18;
    pub const CLOEXEC: u32 = 1 << 19;
    pub const SYNC: u32 = 1 << 20;
    pub const PATH: u32 = 1 << 21;
    pub const TMPFILE: u32 = 1 << 22;
}

fn exists(path: &str) -> bool {
    p9::exists(path)
}

fn remove(path: &str) -> Result<(), ()> {
    p9::remove(path)
}

pub fn unlinkat() -> u64 {
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

    if let Ok(_) = remove(&real_path) {
        0
    } else {
        -2i64 as u64
    }
}

pub fn utimensat() -> u64 {
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

    if exists(&real_path) { 0 } else { -2i64 as u64 }
}

pub fn faccessat() -> u64 {
    utimensat()
}

pub fn openat() -> u64 {
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

    print!("OPEN: path {} by {}\n", real_path, task.pid);

    let mut idx = None;
    for i in 0..task.files.len() {
        if (task.files[i].is_none()) {
            idx = Some(i);
            break;
        }
    }

    if let Some(idx) = idx {
        if let Ok(f) = open(&real_path, tf.regs[2] as u32, tf.regs[3] as u32) {
            task.files[idx] = Some(f);
            return idx as u64;
        } else {
            print!("FAILED TO OPEN: {}\n", real_path);
        }
    }

    !0
}

pub fn fcntl() -> u64 {
    0
}

pub fn ppoll() -> u64 {
    !0
}

pub fn close() -> u64 {
    let task = mycpu().get_task().unwrap();
    let tf = task.get_trap_frame().unwrap();

    let fd = tf.regs[0] as usize;
    print!("CLOSE FD {}\n", fd);

    if task.files[fd].is_none() {
        return !0;
    }

    let file = task.files[fd].as_mut().unwrap();
    print!("CLOSING {:?} fd: {} BY {}\n", file.path, fd, task.pid);
    if let Ok(_) = file.close() {
        task.files[fd] = None;
        0
    } else {
        !0
    }
}

pub fn dup3() -> u64 {
    let task = mycpu().get_task().unwrap();
    let tf = task.get_trap_frame().unwrap();

    let old_fd = tf.regs[0] as usize;
    let new_fd = tf.regs[1] as usize;

    print!("DUP3 old {} new {}\n", old_fd, new_fd);

    if task.files[old_fd].is_none() {
        return !0;
    }

    if task.files[new_fd].is_some() {
        task.files[new_fd].as_mut().unwrap().close().unwrap();
    }

    let file = task.files[old_fd].as_mut().unwrap();
    task.files[new_fd] = file.dup();

    new_fd as u64
}

pub fn sendfile64() -> u64 {
    let task = mycpu().get_task().unwrap();
    let tf = task.get_trap_frame().unwrap();

    let out_fd = tf.regs[0] as usize;
    let in_fd = tf.regs[1] as usize;
    let offt = tf.regs[2] as *mut u64;
    let cnt = tf.regs[3] as usize;

    if task.files[in_fd].is_none() {
        return !0;
    }

    if task.files[out_fd].is_none() {
        return !0;
    }

    print!("SENDFILE: {} {} {:?} {}\n", in_fd, out_fd, offt, cnt);

    let ifile = task.get_file(in_fd).unwrap();
    let ofile = task.get_file(out_fd).unwrap();

    if !offt.is_null() {
        ifile.seek_to(unsafe { offt.read() as usize })
    }

    if let Ok(n) = ifile.send(ofile, cnt) {
        if !offt.is_null() {
            unsafe { offt.write(ifile.offt) }
        }
        n as u64
    } else {
        !0
    }
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

    print!("NEWFSTAT: {}\n", real_path);

    let stat = unsafe { (tf.regs[2] as *mut Stat).as_mut() }.unwrap();
    match p9::stat(
        &real_path,
        stat,
        tf.regs[3] as u32 & AT_SYMLINK_NOFOLLOW > 0,
    ) {
        Ok(_) => {
            return 0;
        }
        _ => return !0,
    }
}

pub fn newfstat() -> u64 {
    let task = mycpu().get_task().unwrap();
    let tf = task.get_trap_frame().unwrap();
    let fd = tf.regs[0] as usize;

    if task.files[fd].is_none() {
        return !0;
    }

    let file = task.files[fd].as_ref().unwrap();

    if let Ok(_) = file.fstat(unsafe { (tf.regs[1] as *mut Stat).as_mut() }.unwrap()) {
        return 0;
    }

    !0
}
pub const AT_FDCWD: i32 = -100;

#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct Stat {
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
