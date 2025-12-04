use core::{
    arch::asm,
    ptr::{NonNull, slice_from_raw_parts_mut},
};

use crate::{
    dsb, print,
    stuff::{BitSet128, StaticMut},
    vm::{self, map},
};

// Device ID	Virtio Device
// -----------------------------------------------
// 0	        reserved (invalid)
// 1	        network card
// 2	        block device
// 3	        console
// 4	        entropy source
// 5	        memory ballooning (traditional)
// 6	        ioMemory
// 7	        rpmsg
// 8	        SCSI host
// 9	        9P transport
// 10	        mac80211 wlan
// 11	        rproc serial
// 12	        virtio CAIF
// 13	        memory balloon
// 16	        GPU device
// 17	        Timer/Clock device
// 18	        Input device
// 19	        Socket device
// 20	        Crypto device
// 21	        Signal Distribution Module
// 22	        pstore device
// 23	        IOMMU device
// 24	        Memory device

#[derive(Debug)]
#[repr(C, align(512))]
struct Regs {
    buf: [u8; 512],
}

impl Regs {
    fn write<T>(&mut self, offt: usize, v: T) {
        unsafe { (self.buf.as_mut_ptr().add(offt) as *mut T).write_volatile(v) }
    }

    fn read<T>(&self, offt: usize) -> T {
        unsafe { self.buf.as_ptr().add(offt).cast::<T>().read_volatile() }
    }

    //R
    // Magic value
    // 0x74726976 (a Little Endian equivalent of the “virt” string).
    pub const MAGICVALUE: usize = 0x000;

    // R
    // Device version number
    // 0x2. Note: Legacy devices (see 4.2.4 Legacy interface) used 0x1.
    pub const VERSION: usize = 0x004;

    // R
    // Virtio Subsystem Device ID
    // See 5 Device Types for possible values. Value zero (0x0) is
    //  used to define a system memory map with placeholder devices
    // at static, well known addresses, assigning functions to them
    // depending on user’s needs.
    pub const DEVICEID: usize = 0x008;

    // R
    // Virtio Subsystem Vendor ID
    pub const VENDORID: usize = 0x00c;

    // R
    // Flags representing features the device supports
    // Reading from this register returns 32 consecutive
    //  flag bits, the least significant bit depending on the last
    //  value written to DeviceFeaturesSel. Access to this register
    //  returns bits DeviceFeaturesSel ∗ 32 to (DeviceFeaturesSel ∗ 32) + 31, eg.
    // feature bits 0 to 31 if DeviceFeaturesSel is set to 0 and
    // features bits 32 to 63 if DeviceFeaturesSel is set to 1.
    // Also see 2.2 Feature Bits.
    pub const DEVICEFEATURES: usize = 0x010;

    // W
    // Device (host) features word selection.
    // Writing to this register selects a set of 32 device feature
    //  bits accessible by reading from DeviceFeatures.
    pub const DEVICEFEATURESSEL: usize = 0x014;

    // W
    // Flags representing device features understood and activated by the driver
    // Writing to this register sets 32 consecutive flag bits, the
    //  least significant bit depending on the last value written
    //  to DriverFeaturesSel. Access to this register sets bits
    //  DriverFeaturesSel ∗ 32 to (DriverFeaturesSel ∗ 32) + 31, eg.
    //  feature bits 0 to 31 if DriverFeaturesSel is set to 0 and
    //  features bits 32 to 63 if DriverFeaturesSel is set to 1.
    //  Also see 2.2 Feature Bits.
    pub const DRIVERFEATURES: usize = 0x020;

    // W
    // Activated (guest) features word selection
    // Writing to this register selects a set of 32 activated
    //  feature bits accessible by writing to DriverFeatures.
    pub const DRIVERFEATURESSEL: usize = 0x024;

    // W
    // Virtual queue index
    // Writing to this register selects the virtual queue that the
    //  following operations on QueueNumMax, QueueNum, QueueReady,
    //  QueueDescLow, QueueDescHigh, QueueAvailLow, QueueAvailHigh,
    //  QueueUsedLow and QueueUsedHigh apply to. The index number of
    //  the first queue is zero (0x0).
    pub const QUEUESEL: usize = 0x030;

    // R
    // Maximum virtual queue size
    // Reading from the register returns the maximum
    //  size (number of elements) of the queue the device is ready
    // to process or zero (0x0) if the queue is not available.
    // This applies to the queue selected by writing to QueueSel.
    pub const QUEUENUMMAX: usize = 0x034;

    // W
    // Virtual queue size
    // Queue size is the number of elements in the queue. Writing to
    //  this register notifies the device what size of the queue the
    //  driver will use. This applies to the queue selected by writing
    //  to QueueSel.
    pub const QUEUENUM: usize = 0x038;

    // RW
    // Virtual queue ready bit
    // Writing one (0x1) to this register notifies the device that
    //  it can execute requests from this virtual queue.
    //  Reading from this register returns the last value written to
    //  it. Both read and write accesses apply to the queue selected
    //  by writing to QueueSel.
    pub const QUEUEREADY: usize = 0x044;

    // W
    // Queue notifier
    // Writing a value to this register notifies the device that
    // there are new buffers to process in a queue.

    // When VIRTIO_F_NOTIFICATION_DATA has not been negotiated,
    //  the value written is the queue index.

    // When VIRTIO_F_NOTIFICATION_DATA has been negotiated,
    //  the Notification data value has the following format:

    // le32 {
    //   vqn : 16;
    //   next_off : 15;
    //   next_wrap : 1;
    // };
    // See 2.7.23 Driver notifications for the definition of the components.
    pub const QUEUENOTIFY: usize = 0x050;

    // R
    // Interrupt status
    // Reading from this register returns a bit mask of events that
    //  caused the device interrupt to be asserted.
    //  The following events are possible:

    // Used Buffer Notification
    // - bit 0 - the interrupt was asserted because
    //  the device has used a buffer in at least one of the
    // active virtual queues.
    // Configuration Change Notification
    // - bit 1 - the interrupt was asserted because the configuration
    //  of the device has changed.
    pub const INTERRUPTSTATUS: usize = 0x60;

    // W
    // Interrupt acknowledge
    // Writing a value with bits set as defined in
    // InterruptStatus to this register notifies the
    //  device that events causing the interrupt have been handled.
    pub const INTERRUPTACK: usize = 0x064;

    // RW
    // Device status
    // Reading from this register returns
    //  the current device status flags.
    //  Writing non-zero values to this register sets
    //  the status flags, indicating the driver progress.
    //  Writing zero (0x0) to this register triggers a device reset. See also p. 4.2.3.1 Device Initialization.
    pub const STATUS: usize = 0x070;

    // W
    // Virtual queue’s Descriptor Area 64 bit
    //  long physical address
    // Writing to these two registers (lower 32 bits of the address
    // to QueueDescLow, higher 32 bits to QueueDescHigh) notifies
    //  the device about location of the Descriptor Area of the queue
    //  selected by writing to QueueSel register.
    pub const QUEUEDESCLOW: usize = 0x080;
    pub const QUEUEDESCHIGH: usize = 0x084;

    // W
    // Virtual queue’s Driver Area 64 bit long physical address
    // Writing to these two registers (lower 32 bits of the address
    // to QueueAvailLow, higher 32 bits to QueueAvailHigh) notifies
    // the device about location of the Driver Area of the queue
    // selected by writing to QueueSel.
    pub const QUEUEDRIVERLOW: usize = 0x090;
    pub const QUEUEDRIVERHIGH: usize = 0x094;

    // W
    // Virtual queue’s Device Area 64 bit long physical address
    // Writing to these two registers (lower 32 bits of the address
    // to QueueUsedLow, higher 32 bits to QueueUsedHigh) notifies the
    //  device about location of the Device Area of the queue
    //  selected by writing to QueueSel.
    pub const QUEUEDEVICELOW: usize = 0x0a0;
    pub const QUEUEDEVICEHIGH: usize = 0x0a4;

    // R
    pub const CONFIGGENERATION: usize = 0x0fc;
    pub const CONFIG: usize = 0x0100;
}

pub struct Status;
impl Status {
    pub const ACKNOWLEDGE: u32 = 1;
    pub const DRIVER: u32 = 2;
    pub const FAILED: u32 = 128;
    pub const FEATURES_OK: u32 = 8;
    pub const DRIVER_OK: u32 = 4;
    pub const DEVICE_NEEDS_RESET: u32 = 64;
}

#[repr(packed, C)]
#[derive(Debug, Clone, Copy)]
struct VqDesc {
    /* Address (guest-physical). */
    addr: u64,
    /* Length.*/
    len: u32,
    /* The flags as indicated above. */
    flags: u16,
    /* Next field if flags & NEXT */
    next: u16,
}

impl VqDesc {
    /* This marks a buffer as continuing via the next field. */
    pub const F_NEXT: u16 = 1;
    /* This marks a buffer as device write-only (otherwise device read-only). */
    pub const F_WRITE: u16 = 2;
    /* This means the buffer contains a list of buffer descriptors. */
    pub const F_INDIRECT: u16 = 4;

    pub fn set_next(&mut self, idx: u16) -> &mut Self {
        self.next = idx;
        self.flags |= Self::F_NEXT;
        self
    }

    pub fn get_next(&self) -> Option<u16> {
        if (self.flags & Self::F_NEXT) == 0 {
            None
        } else {
            Some(self.next)
        }
    }

    pub fn set_writable(&mut self) -> &mut Self {
        self.flags |= Self::F_WRITE;
        self
    }

    pub fn set_readable(&mut self) -> &mut Self {
        self
    }

    pub fn set_len(&mut self, len: u32) -> &mut Self {
        self.len = len;
        self
    }

    pub fn set_data(&mut self, data: u64) -> &mut Self {
        self.addr = vm::v2p(data as usize).unwrap() as u64;
        self
    }
}

impl VqDesc {
    pub const fn zeroed() -> VqDesc {
        VqDesc {
            addr: 0,
            len: 0,
            flags: 0,
            next: 0,
        }
    }
}

#[repr(packed, C)]
#[derive(Debug)]
struct VqAvail<const N: usize> {
    // #define VIRTQ_AVAIL_F_NO_INTERRUPT      1
    flags: u16,
    idx: u16,
    ring: [u16; N],
    used_event: u16, /* Only if VIRTIO_F_EVENT_IDX */
}

impl<const N: usize> VqAvail<N> {
    pub const fn zeroed() -> Self {
        Self {
            flags: 0,
            idx: 0,
            ring: [0; N],
            used_event: 0,
        }
    }
}

#[repr(packed, C)]
#[derive(Debug, Copy, Clone)]
/* le32 is used here for ids for padding reasons. */
struct VqUsedElem {
    /* Index of start of used descriptor chain. */
    id: u32,
    /* Total length of the descriptor chain which
    was used (written to) */
    len: u32,
}

#[repr(packed, C)]
#[derive(Debug)]
struct VqUsed<const N: usize> {
    // #define VIRTQ_USED_F_NO_NOTIFY  1
    flags: u16,
    idx: u16,
    ring: [VqUsedElem; N],
    avail_event: u16, /* Only if VIRTIO_F_EVENT_IDX */
}

impl<const N: usize> VqUsed<N> {
    pub const fn zeroed() -> Self {
        Self {
            flags: 0,
            idx: 0,
            ring: [VqUsedElem { id: 0, len: 0 }; N],
            avail_event: 0,
        }
    }
}

struct Q<const N: usize> {
    desc: [VqDesc; N],
    avail: VqAvail<N>,
    pub used: VqUsed<N>,
    desc_bs: BitSet128,
    pub desc_data: [u64; N],
    pub used_pos: u16,
}

impl<const N: usize> Q<N> {
    pub const fn new() -> Self {
        Self {
            desc: [VqDesc::zeroed(); N],
            avail: VqAvail::zeroed(),
            used: VqUsed::zeroed(),
            desc_bs: BitSet128::new(N as u8),
            desc_data: [0; N],
            used_pos: 0,
        }
    }

    pub fn alloc_desc(&mut self) -> Option<u16> {
        // return None;
        match self.desc_bs.first_clr() {
            Some(f) => {
                self.desc_bs.set(f);
                Some(f as u16)
            }
            _ => None,
        }
    }

    pub fn free_desc(&mut self, hidx: usize) {
        self.desc_bs.clr(hidx as u8);
        let mut d = self.get_desc(hidx);
        let mut i = 1;
        while let Some(nidx) = d.get_next() {
            assert!(i < N);
            self.desc_bs.clr(nidx as u8);
            //TODO wake
            d = self.get_desc(nidx as usize);
            i += 1;
        }
    }

    pub fn get_desc_tail(&mut self, hidx: usize) -> usize {
        let mut d = self.get_desc(hidx);
        let mut i = 1;
        let mut tail = hidx;
        while let Some(nidx) = d.get_next() {
            assert!(i < N);
            d = self.get_desc(nidx as usize);
            i += 1;
            tail = nidx as usize;
        }
        tail
    }

    pub fn get_desc(&mut self, idx: usize) -> &mut VqDesc {
        &mut self.desc[idx]
    }

    pub fn desc_area_paddr(&self) -> (u32, u32) {
        let p = vm::v2p(self.desc.as_ptr() as usize).unwrap();
        ((p & 0xffff_ffff) as u32, (p >> 32) as u32)
    }

    pub fn avail_area_paddr(&self) -> (u32, u32) {
        let p = vm::v2p(&self.avail as *const VqAvail<N> as usize).unwrap();
        ((p & 0xffff_ffff) as u32, (p >> 32) as u32)
    }

    pub fn used_area_paddr(&self) -> (u32, u32) {
        let p = vm::v2p(&self.used as *const VqUsed<N> as usize).unwrap();
        ((p & 0xffff_ffff) as u32, (p >> 32) as u32)
    }

    pub fn add_avail(&mut self, head: u16) {
        self.avail.ring[self.avail.idx as usize % N] = head;
        self.avail.idx = self.avail.idx.wrapping_add(1);
        dsb!();
    }
}

#[inline]
fn select_q(regs: &mut Regs, pos: u32) {
    regs.write(Regs::QUEUESEL, pos);
    dsb!();
}

#[inline]
fn get_qlen_max(regs: &mut Regs, qpos: u32) -> u32 {
    select_q(regs, qpos);
    regs.read(Regs::QUEUENUMMAX)
}

#[inline]
fn set_ready(regs: &mut Regs, qpos: u32) {
    select_q(regs, qpos);
    regs.write(Regs::QUEUEREADY, 1u32);
    dsb!();
}

#[inline]
fn notify_q(regs: &mut Regs, qpos: u32) {
    select_q(regs, qpos);
    regs.write(Regs::QUEUENOTIFY, qpos);
    dsb!();
}

#[inline]
fn get_status(regs: &mut Regs) -> u32 {
    regs.read(Regs::STATUS)
}

#[inline]
fn get_irq_status(regs: &mut Regs) -> u32 {
    regs.read(Regs::INTERRUPTSTATUS)
}

#[inline]
fn irq_ack(regs: &mut Regs, v: u32) {
    regs.write(Regs::INTERRUPTACK, v)
}

#[inline]
fn set_desc_area(regs: &mut Regs, paddr: (u32, u32)) {
    regs.write(Regs::QUEUEDESCLOW, paddr.0);
    regs.write(Regs::QUEUEDESCHIGH, paddr.1);
    dsb!();
}

#[inline]
fn set_used_area(regs: &mut Regs, paddr: (u32, u32)) {
    regs.write(Regs::QUEUEDEVICELOW, paddr.0);
    regs.write(Regs::QUEUEDEVICEHIGH, paddr.1);
    dsb!();
}

#[inline]
fn set_avail_area(regs: &mut Regs, paddr: (u32, u32)) {
    regs.write(Regs::QUEUEDRIVERLOW, paddr.0);
    regs.write(Regs::QUEUEDRIVERHIGH, paddr.1);
    dsb!();
}

#[inline]
fn set_q_len(regs: &mut Regs, qpos: u32, len: u32) {
    select_q(regs, qpos);
    let qlen_max = get_qlen_max(regs, qpos);
    assert!(len <= qlen_max);
    regs.write(Regs::QUEUENUM, len);
    dsb!();
}

static REGS: StaticMut<&mut [Regs]> = StaticMut::new(&mut []);

pub fn init() {
    let map = map(0xa000000, 4).unwrap();
    REGS.set(unsafe { slice_from_raw_parts_mut(map as *mut Regs, 32).as_mut() }.unwrap());
    let regs = REGS.get_mut();
    for i in 0..32 {
        let reg = &mut regs[i];
        assert!(reg.read::<u32>(Regs::MAGICVALUE) == 0x74726976);
        assert!(reg.read::<u32>(Regs::VERSION) == 2);
        let id: u32 = reg.read(Regs::DEVICEID);

        match id {
            2 => {
                // virtio-blk
                print!("virtio-blk found.\n");
                blk::init(reg);
            }
            4 => {
                // virtio-rng
                print!("virtio-rng found.\n");
            }
            9 => {
                // virtio-9p
                print!("virtio-9p found.\n");
            }
            _ => {}
        }
        // print!("id [{}] {}\n", i, reg.read::<u32>(DEVICEID));
    }
}

pub mod blk {
    use ::alloc::boxed::Box;

    use crate::{
        dsb, print,
        spin::Lock,
        virtio::{self, Q, Regs, Status, VqDesc},
        vm,
    };
    use core::{alloc, any::Any, arch::asm, fmt::Pointer, ptr::NonNull};

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

    pub fn init(reg: &mut Regs) {
        let lock = BLK.acquire();
        let blk = lock.as_mut();

        if blk.regs != NonNull::dangling() {
            /*TODO*/
            return;
        }

        fn get_config(reg: &mut Regs) -> &Config {
            unsafe { (((reg as *mut Regs as usize) + Regs::CONFIG) as *mut Config).as_ref() }
                .unwrap()
        }

        blk.regs = NonNull::new(reg as *mut Regs).unwrap();

        reg.write::<u32>(Regs::STATUS, 0);
        dsb!();
        let mut status: u32 = reg.read(Regs::STATUS);
        reg.write(Regs::STATUS, status | Status::ACKNOWLEDGE);
        dsb!();
        reg.write(Regs::STATUS, status | Status::DRIVER);
        dsb!();
        // status = reg.read(Regs::STATUS);
        reg.write(Regs::DEVICEFEATURESSEL, 0u32);
        reg.write(Regs::DRIVERFEATURESSEL, 0u32);
        dsb!();
        let device_features: u32 = reg.read(Regs::DEVICEFEATURES);
        reg.write(
            Regs::DRIVERFEATURES,
            Features::BLK_SIZE | Features::SIZE_MAX | Features::SIZE_MAX,
        );
        status = reg.read(Regs::STATUS);
        dsb!();
        reg.write(Regs::STATUS, status | Status::FEATURES_OK);
        dsb!();
        status = reg.read(Regs::STATUS);
        print!("blk: status {} features {}\n", status, device_features);

        if (status & Status::FEATURES_OK) == 0 {
            panic!("virt-blk feature not ok.");
        }

        reg.write(Regs::STATUS, status | Status::DRIVER_OK);
        dsb!();

        virtio::set_q_len(reg, 0, 4);
        virtio::set_used_area(reg, blk.vq.used_area_paddr());
        virtio::set_avail_area(reg, blk.vq.avail_area_paddr());
        virtio::set_desc_area(reg, blk.vq.desc_area_paddr());
    }

    fn rw(sect: u64, buf: *const u8, len: usize, r: bool) -> Result<(), ()> {
        if len % 512 != 0 {
            return Err(());
        }

        if len > u32::MAX as usize {
            return Err(());
        }

        let kind = if r { ReqKind::IN } else { ReqKind::OUT };
        let req = Box::new(Req::new(kind, sect));

        let lock = BLK.acquire();
        let blk = lock.as_mut();
        // assert!(blk.regs != NonNull::dangling());

        // print!("first clr.....\n");
        let d1_idx = blk.vq.alloc_desc().unwrap();
        let d2_idx = blk.vq.alloc_desc().unwrap();
        let d3_idx = blk.vq.alloc_desc().unwrap();

        let d1 = blk.vq.get_desc(d1_idx as usize);
        // let k = Box::new(0u8);

        d1.set_next(d2_idx)
            .set_len(16)
            .set_data(req.as_ref().paddr() as u64);

        let d2 = blk.vq.get_desc(d2_idx as usize);
        d2.set_next(d3_idx)
            .set_len(len as u32)
            .set_data(vm::v2p(buf as *const u8 as usize).unwrap() as u64);
        if r {
            d2.set_writable();
        }

        let d3 = blk.vq.get_desc(d3_idx as usize);

        d3.set_writable()
            .set_len(1)
            .set_data(req.as_ref().status_paddr() as u64);

        // print!("====> req before: {:?}\n", req);
        blk.vq.desc_data[d1_idx as usize] = Box::into_raw(req) as u64;

        let regs = unsafe { blk.regs.as_mut() };

        blk.vq.add_avail(d1_idx);
        virtio::set_ready(regs, 0);
        virtio::notify_q(regs, 0);
        Ok(())
    }

    pub fn read(sect: u64, buf: &mut [u8]) -> Result<(), ()> {
        let ptr = (&buf[0]) as *const u8;
        let len = buf.len();
        match rw(sect, ptr, len, true) {
            Ok(_) => Ok(()),
            Err(_) => Err(()),
        }
    }

    pub fn write(sect: u64, buf: &[u8]) -> Result<(), ()> {
        let ptr = (&buf[0]) as *const u8;
        let len = buf.len();
        match rw(sect, ptr, len, false) {
            Ok(_) => Ok(()),
            Err(_) => Err(()),
        }
    }

    pub fn read_sync(sect: u64, buf: &mut [u8]) -> Result<(), ()> {
        read(sect, buf)?;
        while !pending_irq() {}
        match irq_handle() {
            Ok(_) => Ok(()),
            _ => Err(()),
        }
    }

    pub fn write_sync(sect: u64, buf: &[u8]) -> Result<(), ()> {
        write(sect, buf)?;
        while !pending_irq() {}
        match irq_handle() {
            Ok(_) => Ok(()),
            _ => Err(()),
        }
    }

    pub fn pending_irq() -> bool {
        let lock = BLK.acquire();
        let blk = lock.as_mut();
        assert!(blk.regs != NonNull::dangling());

        let regs = unsafe { blk.regs.as_mut() };
        virtio::get_irq_status(regs) != 0
    }

    pub fn irq_handle() -> Result<(), u8> {
        let lock = BLK.acquire();
        let blk = lock.as_mut();
        assert!(blk.regs != NonNull::dangling());
        let regs = unsafe { blk.regs.as_mut() };
        let irq_status = virtio::get_irq_status(regs);

        if irq_status & 2 > 0 {
            panic!("device config changed.");
        }

        let used_idx = blk.vq.used_pos as usize % QSIZE;
        let mut req_status = ReqStatus::OK;

        if blk.vq.used.idx as usize != used_idx {
            let head = blk.vq.used.ring[used_idx as usize];
            let req_raw = blk.vq.desc_data[head.id as usize] as *mut Req;
            let req = unsafe { req_raw.as_ref() }.unwrap();
            // print!("irq: req after {:?}\n", req);
            blk.vq.free_desc(head.id as usize);
            // print!("irq end..\n");
            blk.vq.used_pos = blk.vq.used_pos.wrapping_add(1);
            req_status = req.status;
            let _ = unsafe { Box::from_raw(req_raw) };
        } else {
            unreachable!();
            //???
        }

        virtio::irq_ack(regs, irq_status);

        if req_status == ReqStatus::OK {
            Ok(())
        } else {
            Err(req_status)
        }
    }
}
