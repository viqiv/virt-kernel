use crate::{
    dsb, print,
    spin::Lock,
    virtio::{self, Q, Regs, Status, init_dev_common},
    vm,
};
use core::{arch::asm, ptr::NonNull};

const QSIZE: usize = 4;

struct VirtioBlk {
    regs: NonNull<Regs>,
    vq: Q<QSIZE>,
}

// static REGS: StaticMut<Option<&mut Regs>> = StaticMut::new(None);
// static VQ: StaticMut<Q<4>> = StaticMut::new(Q::new());
static BLK: Lock<VirtioBlk> = Lock::new(
    "virtio-blk",
    VirtioBlk {
        regs: NonNull::dangling(),
        vq: Q::new(),
    },
);

struct Features;

impl Features {
    // Maximum size of any single segment is in size_max.
    const SIZE_MAX: u32 = 1;
    // Maximum number of segments in a request is in seg_max.
    const SEG_MAX: u32 = 2;
    // Disk-style geometry specified in geometry.
    const GEOMETRY: u32 = 4;
    // Device is read-only.
    const RO: u32 = 5;
    // Block size of disk is in blk_size.
    const BLK_SIZE: u32 = 6;
    // Cache flush command support.
    const FLUSH: u32 = 9;
    // Device exports information on optimal I/O alignment.
    const TOPOLOGY: u32 = 10;
    // Device can toggle its cache between writeback and writethrough modes.
    const CONFIG_WCE: u32 = 11;
    // Device can support discard command, maximum discard sectors size in max_discard_sectors and maximum discard segment number in max_discard_seg.
    const DISCARD: u32 = 13;
    // Device can support write zeroes command, maximum write zeroes sectors size in max_write_zeroes_sectors and maximum write zeroes segment number in max_write_zeroes_seg.
    const WRITE_ZEROES: u32 = 14;
}

struct ReqKind;
impl ReqKind {
    const IN: u32 = 0;
    const OUT: u32 = 1;
    const FLUSH: u32 = 4;
    const DISCARD: u32 = 11;
    const WRITE_ZEROES: u32 = 13;
}

struct ReqStatus;
impl ReqStatus {
    const OK: u8 = 0;
    const IOERR: u8 = 1;
    const UNSUPP: u8 = 2;
}

#[repr(packed, C)]
#[derive(Debug)]
struct Req {
    kind: u32,
    reserved: u32,
    sector: u64,
    // data: *mut [u8; 512],
    status: u8,
}

impl Req {
    const fn new(kind: u32, sector: u64) -> Self {
        Self {
            kind,
            reserved: 0,
            sector,
            status: 0,
        }
    }

    #[inline]
    fn paddr(&self) -> usize {
        vm::v2p(self as *const Req as usize).unwrap()
    }

    #[inline]
    fn status_paddr(&self) -> usize {
        vm::v2p(self as *const Req as usize).unwrap() + 16
    }
}

#[repr(packed, C)]
#[derive(Debug)]
struct Geometry {
    cylinders: u16,
    heads: u8,
    sectors: u8,
}

#[repr(packed, C)]
#[derive(Debug)]
struct Topology {
    // # of logical blocks per physical block (log2)
    physical_block_exp: u8,
    // offset of first aligned logical block
    alignment_offset: u8,
    // suggested minimum I/O size in blocks
    min_io_size: u16,
    // optimal (suggested maximum) I/O size in blocks
    opt_io_size: u16,
}

#[repr(packed, C)]
// #[derive(Debug)]
struct Config {
    capacity: u64,
    size_max: u32,
    seg_max: u32,
    geometry: Geometry,
    blk_size: u32,
    topology: Topology,
    writeback: u8,
    unused0: [u8; 3],
    max_discard_sectors: u32,
    max_discard_seg: u32,
    discard_sector_alignment: u32,
    max_write_zeroes_sectors: u32,
    max_write_zeroes_seg: u32,
    write_zeroes_may_unmap: u8,
    unused1: [u8; 3],
}

fn get_config(reg: &mut Regs) -> &Config {
    unsafe { (((reg as *mut Regs as usize) + Regs::CONFIG) as *mut Config).as_ref() }.unwrap()
}

pub fn init(reg: &mut Regs) {
    let lock = BLK.acquire();
    let blk = lock.as_mut();

    if blk.regs != NonNull::dangling() {
        /*TODO*/
        return;
    }

    blk.regs = NonNull::new(reg as *mut Regs).unwrap();

    init_dev_common(reg, 0);
    let status: u32 = reg.read(Regs::STATUS);
    reg.write(Regs::STATUS, status | Status::DRIVER_OK);
    dsb!();

    virtio::set_q_len(reg, 0, blk.vq.len());
    virtio::set_used_area(reg, blk.vq.used_area_paddr());
    virtio::set_avail_area(reg, blk.vq.avail_area_paddr());
    virtio::set_desc_area(reg, blk.vq.desc_area_paddr());
    dsb!();
}

fn rw(sect: u64, buf: *const u8, len: usize, r: bool, sync: bool) -> Result<(), ()> {
    if len % 512 != 0 {
        return Err(());
    }

    if len > u32::MAX as usize {
        return Err(());
    }

    let lock = BLK.acquire();
    let blk = lock.as_mut();
    assert!(blk.regs != NonNull::dangling());

    let kind = if r { ReqKind::IN } else { ReqKind::OUT };
    let req = Req::new(kind, sect);

    // print!("first clr.....\n");
    let d1_idx = blk.vq.alloc_desc().unwrap();
    let d2_idx = blk.vq.alloc_desc().unwrap();
    let d3_idx = blk.vq.alloc_desc().unwrap();

    let d1 = blk.vq.get_desc_mut(d1_idx as usize);
    // let k = Box::new(0u8);

    d1.set_next(d2_idx).set_len(16).set_data(req.paddr() as u64);

    let d2 = blk.vq.get_desc_mut(d2_idx as usize);
    d2.set_next(d3_idx)
        .set_len(len as u32)
        .set_data(vm::v2p(buf as *const u8 as usize).unwrap() as u64);
    if r {
        d2.set_writable();
    }

    let d3 = blk.vq.get_desc_mut(d3_idx as usize);

    d3.set_writable()
        .set_len(1)
        .set_data(req.status_paddr() as u64);

    // print!("====> req before: {:?}\n", req)
    let req_ptr = &req as *const Req as u64;
    blk.vq.desc_data[d1_idx as usize] = if sync { 0 } else { req_ptr };

    let regs = unsafe { blk.regs.as_mut() };

    let old = blk.vq.add_avail(d1_idx);
    virtio::set_ready(regs, 0);
    virtio::notify_q(regs, 0);

    if sync {
        blk.vq.wait_use(old);
        drop(lock);
        irq_handle();
    } else {
        //TODO sleep on req_ptr here
    }

    if req.status == ReqStatus::OK {
        Ok(())
    } else {
        Err(())
    }
}

pub fn read(sect: u64, buf: &mut [u8]) -> Result<(), ()> {
    let ptr = (&buf[0]) as *const u8;
    let len = buf.len();
    rw(sect, ptr, len, true, false)
}

pub fn write(sect: u64, buf: &[u8]) -> Result<(), ()> {
    let ptr = (&buf[0]) as *const u8;
    let len = buf.len();
    rw(sect, ptr, len, false, false)
}

pub fn read_sync(sect: u64, buf: &mut [u8]) -> Result<(), ()> {
    let ptr = (&buf[0]) as *const u8;
    let len = buf.len();
    rw(sect, ptr, len, true, true)
}

pub fn write_sync(sect: u64, buf: &[u8]) -> Result<(), ()> {
    let ptr = (&buf[0]) as *const u8;
    let len = buf.len();
    rw(sect, ptr, len, false, true)
}

pub fn pending_irq() -> bool {
    let lock = BLK.acquire();
    let blk = lock.as_mut();
    assert!(blk.regs != NonNull::dangling());

    let regs = unsafe { blk.regs.as_mut() };
    virtio::get_irq_status(regs) != 0
}

pub fn irq_handle() {
    let lock = BLK.acquire();
    let blk = lock.as_mut();
    assert!(blk.regs != NonNull::dangling());
    let regs = unsafe { blk.regs.as_mut() };
    let irq_status = virtio::get_irq_status(regs);

    if irq_status & 2 > 0 {
        panic!("device config changed.");
    }

    while let Some((_, data)) = blk.vq.peek_used() {
        if data != 0 {
            //TODO wake on data here
        }
        blk.vq.pop_used();
    }

    virtio::irq_ack(regs, irq_status);
}
