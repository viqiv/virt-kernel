use core::{
    arch::asm,
    cmp::{max, min},
    hint::spin_loop,
    ops::BitOr,
    ptr::NonNull,
};

use alloc::{collections::btree_set::SymmetricDifference, str, string::String, vec::Vec};
use hashbrown::HashMap;

use crate::{
    dsb,
    heap::SyncUnsafeCell,
    print,
    sched::wakeup,
    spin::Lock,
    stuff::BitSet128,
    trap::gic_enable_intr,
    virtio::{self, Q, Regs, Status, get_irq_status, init_dev_common, irq_ack},
};

struct Msg {
    buf: Vec<u8>,
    pos: usize,
}

impl Msg {
    fn new(capacity: usize) -> Msg {
        let mut v = Vec::new();
        v.resize(capacity, 0x69);
        Msg { buf: v, pos: 0 }
    }

    pub fn get_buf(&self) -> &[u8] {
        &self.buf.as_slice()
    }

    fn get_buf_ptr(&self) -> *const u8 {
        self.buf.as_ptr()
    }

    fn get_self_ptr(&self) -> u64 {
        self as *const Msg as u64
    }

    pub fn read_u8(&mut self) -> Option<u8> {
        let buf = self.get_buf();
        if self.pos + 1 > buf.len() {
            return None;
        }
        let b = buf[self.pos];
        self.pos += 1;
        Some(b)
    }

    pub fn read_u16(&mut self) -> Option<u16> {
        let buf = self.get_buf();
        if self.pos + 2 > buf.len() {
            return None;
        }
        let w = u16::from_le_bytes(buf[self.pos..self.pos + 2].try_into().unwrap());
        self.pos += 2;
        Some(w)
    }

    pub fn read_u32(&mut self) -> Option<u32> {
        let buf = self.get_buf();
        if self.pos + 4 > buf.len() {
            return None;
        }
        let d = u32::from_le_bytes(buf[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        Some(d)
    }

    pub fn read_u64(&mut self) -> Option<u64> {
        let buf = self.get_buf();
        if self.pos + 8 > buf.len() {
            return None;
        }
        let q = u64::from_le_bytes(buf[self.pos..self.pos + 8].try_into().unwrap());
        self.pos += 8;
        Some(q)
    }

    pub fn read_str(&mut self) -> Option<&str> {
        let len = self.read_u16().unwrap() as usize;
        let buf = self.get_buf();
        if self.pos + len > buf.len() {
            return None;
        }
        match str::from_utf8(&buf[self.pos..self.pos + len as usize]) {
            Ok(s) => Some(s),
            Err(_) => None,
        }
    }

    pub fn write_slice(&mut self, slice: &[u8]) {
        let pos = self.pos;
        let vec = &mut self.buf;
        if pos + slice.len() > vec.len() {
            panic!(
                "slice write out of bounds. pos: {} cap {} len {} slice.len {}\n",
                self.pos,
                vec.capacity(),
                vec.len(),
                slice.len()
            );
        }
        vec[pos..self.pos + slice.len()].copy_from_slice(slice);
        self.pos += slice.len();
    }

    pub fn write_u8(&mut self, v: u8) {
        self.write_slice(&[v]);
    }

    pub fn write_u16(&mut self, v: u16) {
        self.write_slice(&v.to_le_bytes());
    }

    pub fn write_u32(&mut self, v: u32) {
        self.write_slice(&v.to_le_bytes());
    }

    pub fn write_u64(&mut self, v: u64) {
        self.write_slice(&v.to_le_bytes());
    }

    pub fn write_str(&mut self, s: &str) {
        self.write_u16(s.as_bytes().len() as u16);
        self.write_slice(s.as_bytes());
    }

    pub fn seek(&mut self, pos: usize) {
        assert!(pos <= self.get_buf().len());
        self.pos = pos
    }

    pub fn tell(&self) -> usize {
        self.pos
    }

    pub fn skip(&mut self, n: usize) {
        self.seek(self.pos + n);
    }
}

const QSIZE: usize = 8;

#[repr(u8)]
#[derive(Debug, Clone, Copy, Default)]
pub enum QIDKind {
    #[default]
    DIR = 0x80,
    APPEND = 0x40,
    EXCL = 0x20,
    MOUNT = 0x10,
    AUTH = 0x08,
    TMP = 0x04,
    SYMLINK = 0x02,
    LINK = 0x01,
    FILE = 0x00,
}

impl TryFrom<u8> for QIDKind {
    type Error = ();
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x0 => Ok(QIDKind::FILE),
            0x01 => Ok(QIDKind::LINK),
            0x02 => Ok(QIDKind::SYMLINK),
            0x04 => Ok(QIDKind::TMP),
            0x08 => Ok(QIDKind::AUTH),
            0x10 => Ok(QIDKind::MOUNT),
            0x20 => Ok(QIDKind::EXCL),
            0x40 => Ok(QIDKind::APPEND),
            0x80 => Ok(QIDKind::DIR),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct QID {
    pub kind: QIDKind,
    pub version: u32,
    pub path: u64,
}

impl QID {
    const fn new() -> QID {
        QID {
            kind: QIDKind::DIR,
            version: 0,
            path: 0,
        }
    }
}

pub struct P9 {
    q: Q<QSIZE>,
    fid_bs: BitSet128,
    tag: u16,
    qid: QID,
    regs: Option<NonNull<Regs>>,
}

impl P9 {
    fn alloc_fid(&mut self) -> Option<u32> {
        match self.fid_bs.first_clr() {
            Some(i) => {
                self.fid_bs.set(i);
                Some(i as u32)
            }
            _ => None,
        }
    }

    fn free_fid(&mut self, fid: u32) {
        assert!(self.fid_is_ok(fid));
        self.fid_bs.clr(fid as u8);
    }

    fn next_tag(&mut self) -> u16 {
        let tag = self.tag;
        self.tag = tag.wrapping_add(1);
        tag
    }

    fn fid_is_ok(&self, fid: u32) -> bool {
        fid < self.fid_bs.len() as u32 && self.fid_bs.tst(fid as u8)
    }
}

#[repr(u8)]
#[allow(non_camel_case_types)]
enum Op {
    TLERROR = 6,
    RLERROR,
    TSTATFS = 8,
    RSTATFS,
    TLOPEN = 12,
    RLOPEN,
    TLCREATE = 14,
    RLCREATE,
    TSYMLINK = 16,
    RSYMLINK,
    TMKNOD = 18,
    RMKNOD,
    TRENAME = 20,
    RRENAME,
    TREADLINK = 22,
    RREADLINK,
    TGETATTR = 24,
    RGETATTR,
    TSETATTR = 26,
    RSETATTR,
    TXATTRWALK = 30,
    RXATTRWALK,
    TXATTRCREATE = 32,
    RXATTRCREATE,
    TREADDIR = 40,
    RREADDIR,
    TFSYNC = 50,
    RFSYNC,
    TLOCK = 52,
    RLOCK,
    TGETLOCK = 54,
    RGETLOCK,
    TLINK = 70,
    RLINK,
    TMKDIR = 72,
    RMKDIR,
    TRENAMEAT = 74,
    RRENAMEAT,
    TUNLINKAT = 76,
    RUNLINKAT,
    TVERSION = 100,
    RVERSION,
    TAUTH = 102,
    RAUTH,
    TATTACH = 104,
    RATTACH,
    TERROR = 106,
    RERROR,
    TFLUSH = 108,
    RFLUSH,
    TWALK = 110,
    RWALK,
    TOPEN = 112,
    ROPEN,
    TCREATE = 114,
    RCREATE,
    TREAD = 116,
    RREAD,
    TWRITE = 118,
    RWRITE,
    TCLUNK = 120,
    RCLUNK,
    TREMOVE = 122,
    RREMOVE,
    TSTAT = 124,
    RSTAT,
    TWSTAT = 126,
    RWSTAT,
}

#[repr(u32)]
#[allow(non_camel_case_types)]
pub enum O {
    RDONLY = 00000000,
    WRONLY = 00000001,
    RDWR = 00000002,
    NOACCESS = 00000003,
    CREATE = 00000100,
    EXCL = 00000200,
    NOCTTY = 00000400,
    TRUNC = 00001000,
    APPEND = 00002000,
    NONBLOCK = 00004000,
    DSYNC = 00010000,
    FASYNC = 00020000,
    DIRECT = 00040000,
    LARGEFILE = 00100000,
    DIRECTORY = 00200000,
    NOFOLLOW = 00400000,
    NOATIME = 01000000,
    CLOEXEC = 02000000,
    SYNC = 04000000,
}

impl BitOr for O {
    type Output = u32;
    fn bitor(self, rhs: Self) -> Self::Output {
        self as u32 | rhs as u32
    }
}

const VERSION: &'static str = "9P2000.L";

static P9L: Lock<P9> = Lock::new(
    "9p",
    P9 {
        q: Q::new(),
        fid_bs: BitSet128::new(128),
        tag: 0,
        qid: QID::new(),
        regs: None,
    },
);

#[derive(Default, Debug)]
pub struct Stat {
    pub kind: u16,
    pub dev: u32,
    pub qid: QID,
    pub mode: u32,
    pub atime: u32,
    pub mtime: u32,
    pub len: u64,
}

impl Stat {
    const fn zeroed() -> Stat {
        Stat {
            kind: 0,
            dev: 0,
            qid: QID::new(),
            mode: 0,
            atime: 0,
            mtime: 0,
            len: 0,
        }
    }
}

mod ops {
    use core::{cmp::max, hint::spin_loop, sync::atomic::spin_loop_hint};

    use alloc::vec::Vec;

    use crate::{
        p9::{Msg, Op, P9, P9L, QID, Stat, VERSION},
        print,
        sched::sleep,
        virtio::{self, get_irq_status, irq_ack},
    };

    pub fn set_version(p9: &mut P9) {
        //size[4] Tversion tag[2] msize[4] version[s]
        let msg_len = 4 + 1 + 2 + 4 + 2 + VERSION.len();
        let mut msg = Msg::new(msg_len);

        msg.write_u32(msg_len as u32);
        msg.write_u8(Op::TVERSION as u8);
        msg.write_u16(0);
        msg.write_u32(u16::MAX as u32);
        let vpos = msg.tell();
        msg.write_str(VERSION);

        let d1 = p9.q.alloc_desc().unwrap();
        let d2 = p9.q.alloc_desc().unwrap();

        let desc1 = p9.q.get_desc_mut(d1 as usize);
        desc1
            .set_next(d2)
            .set_data(msg.get_buf_ptr() as u64)
            .set_len(msg_len as u32);

        let desc2 = p9.q.get_desc_mut(d2 as usize);

        desc2
            .set_writable()
            .set_len(msg_len as u32)
            .set_data(msg.get_buf_ptr() as u64);

        let regs = unsafe { p9.regs.unwrap().as_mut() };
        let old = p9.q.add_avail(d1);
        virtio::set_ready(regs, 0);
        virtio::notify_q(regs, 0);

        p9.q.wait_use(old);
        p9.q.pop_used();
        let irq_s = get_irq_status(regs);
        irq_ack(regs, irq_s);

        msg.seek(4);
        let resp_kind = msg.read_u8().unwrap();
        msg.seek(vpos);
        let rv = msg.read_str().unwrap();
        assert!(resp_kind == Op::RVERSION as u8 && rv == VERSION);
    }

    pub fn attach(p9: &mut P9) {
        // size[4] Tattach tag[2] fid[4] afid[4] uname[s] aname[s] n_uname[4]
        // size[4] Rattach tag[2] qid[13]
        let mut msg = Msg::new(4 + 1 + 2 + 4 + 4 + 2 + 2 + 4 + 4);
        msg.write_u32(0);
        msg.write_u8(Op::TATTACH as u8);
        msg.write_u16(p9.next_tag());
        msg.write_u32(0);
        msg.write_u32(!0u32);
        msg.write_str("root");
        msg.write_str("");
        msg.write_u32(0);
        let len = msg.tell();
        msg.seek(0);
        msg.write_u32(len as u32);

        let d1 = p9.q.alloc_desc().unwrap();
        let d2 = p9.q.alloc_desc().unwrap();

        let desc1 = p9.q.get_desc_mut(d1 as usize);
        desc1
            .set_next(d2)
            .set_data(msg.get_buf_ptr() as u64)
            .set_len(len as u32);

        let desc2 = p9.q.get_desc_mut(d2 as usize);

        desc2
            .set_writable()
            .set_len(20)
            .set_data(msg.get_buf_ptr() as u64);

        let old = p9.q.add_avail(d1);

        let regs = unsafe { p9.regs.unwrap().as_mut() };
        virtio::set_ready(regs, 0);
        virtio::notify_q(regs, 0);

        p9.q.wait_use(old);
        p9.q.pop_used();
        let irq_s = get_irq_status(regs);
        irq_ack(regs, irq_s);

        msg.seek(4);
        let resp_kind = msg.read_u8().unwrap();
        assert!(resp_kind == Op::RATTACH as u8);
        msg.seek(7);
        p9.qid.kind = msg.read_u8().unwrap().try_into().unwrap();
        p9.qid.version = msg.read_u32().unwrap();
        p9.qid.path = msg.read_u64().unwrap();
    }

    fn path_to_wnames(path: &str) -> Vec<&str> {
        path.split('/').filter(|s| !s.is_empty()).collect()
    }

    pub fn walk(path: &str) -> Result<(u32, QID), ()> {
        if path.is_empty() {
            return Err(());
        }

        let lock = P9L.acquire();
        let p9 = lock.as_mut();

        if path == "/" {
            return Ok((0, p9.qid));
        }

        let wnames = path_to_wnames(path);
        if wnames.len() > u16::MAX as usize {
            return Err(());
        }
        // print!("wnames {:?}\n", wnames);

        // size[4] Twalk tag[2] fid[4] newfid[4] nwname[2] nwname*(wname[s])
        // size[4] Rwalk tag[2] nwqid[2] nwqid*(wqid[13])
        let resp_len = 22 + 13 * wnames.len();
        let mut msg = Msg::new(resp_len);
        msg.write_u32(0);
        msg.write_u8(Op::TWALK as u8);
        msg.write_u16(p9.next_tag());
        msg.write_u32(0);
        let fid = p9.alloc_fid().unwrap();
        msg.write_u32(fid);
        msg.write_u16(wnames.len() as u16);
        for i in 0..wnames.len() {
            msg.write_str(wnames[i]);
        }
        let len = msg.tell();
        msg.seek(0);
        msg.write_u32(len as u32);

        let d1 = p9.q.alloc_desc().unwrap();
        let d2 = p9.q.alloc_desc().unwrap();

        let desc1 = p9.q.get_desc_mut(d1 as usize);
        desc1
            .set_next(d2)
            .set_data(msg.get_buf_ptr() as u64)
            .set_len(len as u32);

        let desc2 = p9.q.get_desc_mut(d2 as usize);

        desc2
            .set_writable()
            .set_len(resp_len as u32)
            .set_data(msg.get_buf_ptr() as u64);

        p9.q.set_desc_data(d1 as usize, msg.get_self_ptr());
        let _ = p9.q.add_avail(d1);

        let regs = unsafe { p9.regs.unwrap().as_mut() };
        virtio::set_ready(regs, 0);
        virtio::notify_q(regs, 0);

        // {
        //     p9.q.wait_use(old);
        //     p9.q.pop_used();
        //     let irq_s = get_irq_status(regs);
        //     irq_ack(regs, irq_s);
        // }

        // print!("Data: {:x} {}\n", msg.get_self_ptr(), d1);
        sleep(msg.get_self_ptr(), lock.get_lock());

        msg.seek(4);
        let resp_kind = msg.read_u8().unwrap();
        if resp_kind != Op::RWALK as u8 {
            return Err(());
        }
        msg.seek(7);
        let qid_len = msg.read_u16().unwrap() as usize;
        if qid_len != wnames.len() {
            return Err(());
        }

        for _ in 0..qid_len - 1 {
            msg.skip(13);
        }

        let mut qid = super::QID::new();

        qid.kind = msg.read_u8().unwrap().try_into().unwrap();
        qid.version = msg.read_u32().unwrap();
        qid.path = msg.read_u64().unwrap();

        Ok((fid, qid))
    }

    pub fn open(fid: u32, mode: u32) -> Result<(QID, u32), ()> {
        let lock = P9L.acquire();
        let p9 = lock.as_mut();

        if !p9.fid_is_ok(fid) {
            return Err(());
        }

        // size[4] Topen tag[2] fid[4] mode[4]
        // size[4] Ropen tag[2] qid[13] iounit[4]
        let resp_len = 4 + 1 + 2 + 13 + 4;
        let mut msg = Msg::new(resp_len);
        msg.write_u32(0);
        msg.write_u8(Op::TOPEN as u8);
        msg.write_u16(p9.next_tag());
        msg.write_u32(fid);
        msg.write_u32(mode as u32);
        let len = msg.tell();
        msg.seek(0);
        msg.write_u32(len as u32);

        let d1 = p9.q.alloc_desc().unwrap();
        let d2 = p9.q.alloc_desc().unwrap();

        let desc1 = p9.q.get_desc_mut(d1 as usize);
        desc1
            .set_next(d2)
            .set_data(msg.get_buf_ptr() as u64)
            .set_len(len as u32);

        let desc2 = p9.q.get_desc_mut(d2 as usize);

        desc2
            .set_writable()
            .set_len(resp_len as u32)
            .set_data(msg.get_buf_ptr() as u64);

        p9.q.set_desc_data(d1 as usize, msg.get_self_ptr());
        let _ = p9.q.add_avail(d1);

        let regs = unsafe { p9.regs.unwrap().as_mut() };
        virtio::set_ready(regs, 0);
        virtio::notify_q(regs, 0);

        sleep(msg.get_self_ptr(), lock.get_lock());

        msg.seek(4);
        let resp_kind = msg.read_u8().unwrap();
        if resp_kind != Op::ROPEN as u8 {
            return Err(());
        }

        msg.seek(7);
        let mut qid = QID::new();

        qid.kind = msg.read_u8().unwrap().try_into().unwrap();
        qid.version = msg.read_u32().unwrap();
        qid.path = msg.read_u64().unwrap();

        Ok((qid, msg.read_u32().unwrap()))
    }

    pub fn remove(fid: u32) -> Result<(), ()> {
        let lock = P9L.acquire();
        let p9 = lock.as_mut();

        if !p9.fid_is_ok(fid) {
            return Err(());
        }

        // size[4] Tremove tag[2] fid[4]
        // size[4] Rremove tag[2]
        let resp_len = 4 + 1 + 2;
        let mut msg = Msg::new(resp_len + 4);
        msg.write_u32(resp_len as u32 + 4);
        msg.write_u8(Op::TREMOVE as u8);
        msg.write_u16(p9.next_tag());
        msg.write_u32(fid);

        let d1 = p9.q.alloc_desc().unwrap();
        let d2 = p9.q.alloc_desc().unwrap();

        let desc1 = p9.q.get_desc_mut(d1 as usize);
        desc1
            .set_next(d2)
            .set_data(msg.get_buf_ptr() as u64)
            .set_len(resp_len as u32 + 4);

        let desc2 = p9.q.get_desc_mut(d2 as usize);

        desc2
            .set_writable()
            .set_len(resp_len as u32 + 4)
            .set_data(msg.get_buf_ptr() as u64);

        p9.q.set_desc_data(d1 as usize, msg.get_self_ptr());
        let old = p9.q.add_avail(d1);

        let regs = unsafe { p9.regs.unwrap().as_mut() };
        virtio::set_ready(regs, 0);
        virtio::notify_q(regs, 0);

        sleep(msg.get_self_ptr(), lock.get_lock());

        msg.seek(4);
        let resp_kind = msg.read_u8().unwrap();
        msg.seek(resp_len + 4);
        if resp_kind != Op::RREMOVE as u8 {
            return Err(());
        }
        Ok(())
    }

    enum RWBuf<'a> {
        R(&'a mut [u8]),
        W(&'a [u8]),
    }

    impl<'a> RWBuf<'a> {
        fn len(&self) -> usize {
            match self {
                Self::R(b) => b.len(),
                Self::W(b) => b.len(),
            }
        }

        fn is_r(&self) -> bool {
            match self {
                Self::R(_) => true,
                Self::W(_) => false,
            }
        }

        fn buf_mut(&mut self) -> &mut [u8] {
            match self {
                Self::R(b) => b,
                Self::W(_) => panic!("read only"),
            }
        }

        fn buf(&self) -> &[u8] {
            match self {
                Self::R(b) => b,
                Self::W(b) => b,
            }
        }
    }

    fn rw(fid: u32, mut buf: RWBuf, offt: usize) -> Result<usize, ()> {
        let lock = P9L.acquire();
        let p9 = lock.as_mut();

        if !p9.fid_is_ok(fid) {
            return Err(());
        }

        if buf.len() > u16::MAX as usize {
            return Err(());
        }

        let r = buf.is_r();

        // size[4] Tread tag[2] fid[4] offset[8] count[4]
        // size[4] Rread tag[2] count[4] data[count]
        //
        // size[4] Twrite tag[2] fid[4] offset[8] count[4] data[count]
        // size[4] Rwrite tag[2] count[4]

        let resp_len = 4 + 1 + 2 + 4 + 8 + 4 + buf.len();

        let mut msg = Msg::new(resp_len);

        msg.write_u32(0);
        msg.write_u8(if r { Op::TREAD } else { Op::TWRITE } as u8);
        msg.write_u16(p9.next_tag());
        msg.write_u32(fid);
        msg.write_u64(offt as u64);
        msg.write_u32(buf.len() as u32);
        if !r {
            print!("resp len = {}\n", resp_len);
            msg.write_slice(buf.buf());
        }
        let len = msg.tell();
        msg.seek(0);
        msg.write_u32(len as u32);

        let d1 = p9.q.alloc_desc().unwrap();
        let d2 = p9.q.alloc_desc().unwrap();

        let desc1 = p9.q.get_desc_mut(d1 as usize);
        desc1
            .set_next(d2)
            .set_data(msg.get_buf_ptr() as u64)
            .set_len(len as u32);

        let desc2 = p9.q.get_desc_mut(d2 as usize);

        desc2
            .set_writable()
            .set_len(resp_len as u32)
            .set_data(msg.get_buf_ptr() as u64);

        p9.q.set_desc_data(d1 as usize, msg.get_self_ptr());
        let old = p9.q.add_avail(d1);

        let regs = unsafe { p9.regs.unwrap().as_mut() };
        virtio::set_ready(regs, 0);
        virtio::notify_q(regs, 0);

        sleep(msg.get_self_ptr(), lock.get_lock());

        msg.seek(4);
        let resp_kind = msg.read_u8().unwrap();
        if resp_kind != if r { Op::RREAD } else { Op::RWRITE } as u8 {
            return Err(());
        }

        msg.seek(7);
        let n = msg.read_u32().unwrap() as usize;
        // print!("N = {}\n", n);
        if r {
            buf.buf_mut()[0..n].copy_from_slice(&msg.get_buf()[msg.pos..][0..n]);
        }
        Ok(n)
    }

    pub fn read(fid: u32, buf: &mut [u8], offt: usize) -> Result<usize, ()> {
        rw(fid, RWBuf::R(buf), offt)
    }

    pub fn write(fid: u32, buf: &[u8], offt: usize) -> Result<usize, ()> {
        rw(fid, RWBuf::W(buf), offt)
    }

    pub fn clunk(fid: u32) -> Result<(), ()> {
        // size[4] Tclunk tag[2] fid[4]
        // size[4] Rclunk tag[2]
        let lock = P9L.acquire();
        let p9 = lock.as_mut();

        if !p9.fid_is_ok(fid) {
            return Err(());
        }

        let mut msg = Msg::new(11);
        msg.write_u32(11);
        msg.write_u8(Op::TCLUNK as u8);
        msg.write_u16(p9.next_tag());
        msg.write_u32(fid);

        let d1 = p9.q.alloc_desc().unwrap();
        let d2 = p9.q.alloc_desc().unwrap();

        let desc1 = p9.q.get_desc_mut(d1 as usize);
        desc1
            .set_next(d2)
            .set_data(msg.get_buf_ptr() as u64)
            .set_len(11);

        let desc2 = p9.q.get_desc_mut(d2 as usize);

        desc2
            .set_writable()
            .set_len(7)
            .set_data(msg.get_buf_ptr() as u64);

        p9.q.set_desc_data(d1 as usize, msg.get_self_ptr());
        let old = p9.q.add_avail(d1);

        let regs = unsafe { p9.regs.unwrap().as_mut() };
        virtio::set_ready(regs, 0);
        virtio::notify_q(regs, 0);

        sleep(msg.get_self_ptr(), lock.get_lock());

        msg.seek(4);
        let resp_kind = msg.read_u8().unwrap();
        if resp_kind != Op::RCLUNK as u8 {
            return Err(());
        }

        p9.free_fid(fid);

        Ok(())
    }

    pub fn create(fid: u32, name: &str, perm: u32, mode: u32, gid: u32) -> Result<(QID, u32), ()> {
        let lock = P9L.acquire();
        let p9 = lock.as_mut();

        if !p9.fid_is_ok(fid) {
            return Err(());
        }

        // size[4] Tcreate tag[2] fid[4] name[s] perm[4] mode[4] gid [4]
        // size[4] Rcreate tag[2] qid[13] iounit[4]
        let tlen = 4 + 1 + 2 + 4 + 2 + name.as_bytes().len() + 4 + 4 + 4;
        let rlen = 4 + 1 + 2 + 13 + 4;
        let mut msg = Msg::new(max(tlen, rlen));
        msg.write_u32(tlen as u32);
        msg.write_u8(Op::TCREATE as u8);
        msg.write_u16(p9.next_tag());
        msg.write_u32(fid);
        msg.write_str(name);
        msg.write_u32(perm);
        msg.write_u32(mode as u32);
        msg.write_u32(gid);

        let d1 = p9.q.alloc_desc().unwrap();
        let d2 = p9.q.alloc_desc().unwrap();

        let desc1 = p9.q.get_desc_mut(d1 as usize);
        desc1
            .set_next(d2)
            .set_data(msg.get_buf_ptr() as u64)
            .set_len(tlen as u32);

        let desc2 = p9.q.get_desc_mut(d2 as usize);

        desc2
            .set_writable()
            .set_len(rlen as u32)
            .set_data(msg.get_buf_ptr() as u64);

        p9.q.set_desc_data(d1 as usize, msg.get_self_ptr());
        let old = p9.q.add_avail(d1);

        let regs = unsafe { p9.regs.unwrap().as_mut() };
        virtio::set_ready(regs, 0);
        virtio::notify_q(regs, 0);

        sleep(msg.get_self_ptr(), lock.get_lock());

        msg.seek(4);
        let resp_kind = msg.read_u8().unwrap();
        if resp_kind != Op::RCREATE as u8 {
            return Err(());
        }

        msg.seek(7);
        let mut qid = QID::new();

        qid.kind = msg.read_u8().unwrap().try_into().unwrap();
        qid.version = msg.read_u32().unwrap();
        qid.path = msg.read_u64().unwrap();

        Ok((qid, msg.read_u32().unwrap()))
    }

    pub fn mkdir(fid: u32, name: &str, mode: u32, gid: u32) -> Result<QID, ()> {
        let lock = P9L.acquire();
        let p9 = lock.as_mut();

        if !p9.fid_is_ok(fid) {
            return Err(());
        }

        // size[4] Tcreate tag[2] fid[4] name[s] mode[4] gid [4]
        // size[4] Rcreate tag[2] qid[13] iounit[4]
        let tlen = 4 + 1 + 2 + 4 + 2 + name.as_bytes().len() + 4 + 4;
        let rlen = 4 + 1 + 2 + 13;
        let mut msg = Msg::new(max(tlen, rlen));
        msg.write_u32(tlen as u32);
        msg.write_u8(Op::TMKDIR as u8);
        msg.write_u16(p9.next_tag());
        msg.write_u32(fid);
        msg.write_str(name);
        msg.write_u32(mode as u32);
        msg.write_u32(gid);

        let d1 = p9.q.alloc_desc().unwrap();
        let d2 = p9.q.alloc_desc().unwrap();

        let desc1 = p9.q.get_desc_mut(d1 as usize);
        desc1
            .set_next(d2)
            .set_data(msg.get_buf_ptr() as u64)
            .set_len(tlen as u32);

        let desc2 = p9.q.get_desc_mut(d2 as usize);

        desc2
            .set_writable()
            .set_len(rlen as u32)
            .set_data(msg.get_buf_ptr() as u64);

        p9.q.set_desc_data(d1 as usize, msg.get_self_ptr());
        let old = p9.q.add_avail(d1);

        let regs = unsafe { p9.regs.unwrap().as_mut() };
        virtio::set_ready(regs, 0);
        virtio::notify_q(regs, 0);

        sleep(msg.get_self_ptr(), lock.get_lock());

        msg.seek(4);
        let resp_kind = msg.read_u8().unwrap();
        if resp_kind != Op::RMKDIR as u8 {
            return Err(());
        }

        msg.seek(7);
        let mut qid = QID::new();

        qid.kind = msg.read_u8().unwrap().try_into().unwrap();
        qid.version = msg.read_u32().unwrap();
        qid.path = msg.read_u64().unwrap();

        Ok(qid)
    }

    pub fn readdir(fid: u32, buf: &mut [u8], offt: u64) -> Result<u32, ()> {
        let lock = P9L.acquire();
        let p9 = lock.as_mut();

        if !p9.fid_is_ok(fid) {
            return Err(());
        }

        if buf.len() > u16::MAX as usize {
            return Err(());
        }

        // size[4] Treaddir tag[2] fid[4] offt [8] count [4]
        // size[4] Rreaddir tag[2] count[4]
        let tlen = 4 + 1 + 2 + 4 + 8 + 4;
        let rlen = 4 + 1 + 2 + 4 + buf.len();
        let mut msg = Msg::new(max(tlen, rlen));
        msg.write_u32(tlen as u32);
        msg.write_u8(Op::TREADDIR as u8);
        msg.write_u16(p9.next_tag());
        msg.write_u32(fid);
        msg.write_u64(offt);
        msg.write_u32(buf.len() as u32);

        let d1 = p9.q.alloc_desc().unwrap();
        let d2 = p9.q.alloc_desc().unwrap();

        let desc1 = p9.q.get_desc_mut(d1 as usize);
        desc1
            .set_next(d2)
            .set_data(msg.get_buf_ptr() as u64)
            .set_len(tlen as u32);

        let desc2 = p9.q.get_desc_mut(d2 as usize);

        desc2
            .set_writable()
            .set_len(rlen as u32)
            .set_data(msg.get_buf_ptr() as u64);

        p9.q.set_desc_data(d1 as usize, msg.get_self_ptr());
        let old = p9.q.add_avail(d1);

        let regs = unsafe { p9.regs.unwrap().as_mut() };
        virtio::set_ready(regs, 0);
        virtio::notify_q(regs, 0);

        sleep(msg.get_self_ptr(), lock.get_lock());

        msg.seek(4);
        let resp_kind = msg.read_u8().unwrap();
        if resp_kind != Op::RREADDIR as u8 {
            return Err(());
        }
        msg.seek(7);
        let count = msg.read_u32().unwrap();
        buf[0..count as usize].copy_from_slice(&msg.get_buf()[msg.pos..msg.pos + count as usize]);
        Ok(count)
    }

    pub fn stat(fid: u32) -> Result<Stat, ()> {
        let lock = P9L.acquire();
        let p9 = lock.as_mut();

        if !p9.fid_is_ok(fid) {
            return Err(());
        }

        // size[4] Treaddir tag[2] fid[4]
        // size[4] Rreaddir tag[2]
        // [2] zero
        // [2] size
        // [2] type
        // [4] dev
        // [13] qid
        // [4] mode
        // [4] atime
        // [4] mtime
        // [8] length
        // [s] name
        // [s] uid
        // [s] gid
        // [s] muid

        let tlen = 4 + 1 + 2 + 4;
        let rlen = 4 + 1 + 2 + 2 + 2 + 2 + 4 + 13 + 4 + 4 + 4 + 8 + 256;
        let mut msg = Msg::new(max(tlen, rlen));
        msg.write_u32(tlen as u32);
        msg.write_u8(Op::TSTAT as u8);
        msg.write_u16(p9.next_tag());
        msg.write_u32(fid);

        let d1 = p9.q.alloc_desc().unwrap();
        let d2 = p9.q.alloc_desc().unwrap();

        let desc1 = p9.q.get_desc_mut(d1 as usize);
        desc1
            .set_next(d2)
            .set_data(msg.get_buf_ptr() as u64)
            .set_len(tlen as u32);

        let desc2 = p9.q.get_desc_mut(d2 as usize);

        desc2
            .set_writable()
            .set_len(rlen as u32)
            .set_data(msg.get_buf_ptr() as u64);

        p9.q.set_desc_data(d1 as usize, msg.get_self_ptr());
        let old = p9.q.add_avail(d1);

        let regs = unsafe { p9.regs.unwrap().as_mut() };
        virtio::set_ready(regs, 0);
        virtio::notify_q(regs, 0);
        let _ = old;

        sleep(msg.get_self_ptr(), lock.get_lock());

        msg.seek(4);
        let resp_kind = msg.read_u8().unwrap();
        if resp_kind != Op::RSTAT as u8 {
            return Err(());
        }
        msg.seek(11);
        let mut stat = Stat::default();
        stat.kind = msg.read_u16().unwrap();
        stat.dev = msg.read_u32().unwrap();

        stat.qid.kind = msg.read_u8().unwrap().try_into().unwrap();
        stat.qid.version = msg.read_u32().unwrap();
        stat.qid.path = msg.read_u64().unwrap();

        stat.mode = msg.read_u32().unwrap();
        stat.atime = msg.read_u32().unwrap();
        stat.mtime = msg.read_u32().unwrap();
        stat.len = msg.read_u64().unwrap();
        Ok(stat)
    }
}

pub fn irq_handle() {
    let lock = P9L.acquire();
    let p9 = lock.as_mut();
    assert!(p9.regs.is_some());
    let regs = unsafe { p9.regs.unwrap().as_mut() };
    let irq_status = virtio::get_irq_status(regs);

    if irq_status & 2 > 0 {
        panic!("device config changed.");
    }

    while let Some((_, data)) = p9.q.peek_used() {
        if data != 0 {
            wakeup(data);
        }
        p9.q.pop_used();
    }
    virtio::irq_ack(regs, irq_status);
}

pub fn init(regs: &mut Regs, irq: u32) {
    let lock = P9L.acquire();
    let p9 = lock.as_mut();

    if p9.regs.is_some() {
        // TODO
        return;
    }

    p9.regs = NonNull::new(regs as *mut Regs);
    p9.alloc_fid().unwrap(); // waste fid 0

    init_dev_common(regs, 0);

    virtio::set_q_len(regs, 0, p9.q.len());
    virtio::set_used_area(regs, p9.q.used_area_paddr());
    virtio::set_avail_area(regs, p9.q.avail_area_paddr());
    virtio::set_desc_area(regs, p9.q.desc_area_paddr());
    dsb!();

    let status: u32 = regs.read(Regs::STATUS);
    regs.write(Regs::STATUS, status | Status::DRIVER_OK);
    dsb!();
    ops::set_version(p9);
    ops::attach(p9);

    let root = &mut FILES.as_mut()[0];
    root.fid = 0;
    root.qid = p9.qid;
    root.iou = u16::MAX as u32;

    gic_enable_intr(irq as usize);
}

pub struct File {
    pub fid: u32,
    pub iou: u32,
    pub qid: QID,
}

impl File {
    const fn zeroed() -> File {
        File {
            fid: 0,
            iou: 0,
            qid: QID::new(),
        }
    }
}

impl File {
    pub fn read(&self, buf: &mut [u8], offt: usize) -> Result<usize, ()> {
        let len = min(self.iou as usize, buf.len());
        ops::read(self.fid, &mut buf[0..len], offt)
    }

    pub fn write(&self, buf: &[u8], offt: usize) -> Result<usize, ()> {
        let len = min(self.iou as usize, buf.len());
        ops::write(self.fid, &buf[0..len], offt)
    }

    pub fn close(&self) -> Result<(), ()> {
        ops::clunk(self.fid)
    }
}

pub fn open(path: &str, mode: u32) -> Result<&'static mut File, ()> {
    if let Ok((fid, _)) = ops::walk(path) {
        if let Ok((qid, iou)) = ops::open(fid, mode) {
            let file = &mut FILES.as_mut()[fid as usize];
            file.fid = fid;
            file.iou = iou;
            file.qid = qid;
            return Ok(file);
        } else {
            return Err(());
        }
    };

    Err(())
}

pub fn stat(path: &str) -> Result<Stat, ()> {
    if let Ok((fid, _)) = ops::walk(path) {
        if let Ok(s) = ops::stat(fid) {
            return Ok(s);
        } else {
            return Err(());
        }
    };

    Err(())
}

static FILES: SyncUnsafeCell<[File; 128]> = SyncUnsafeCell::new([
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
]);
