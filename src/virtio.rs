use core::{
    arch::asm,
    hint::spin_loop,
    ptr::{NonNull, slice_from_raw_parts_mut},
};

use crate::{
    blk, dsb, print, rng,
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
pub struct Regs {
    buf: [u8; 512],
}

impl Regs {
    pub fn write<T>(&mut self, offt: usize, v: T) {
        unsafe { (self.buf.as_mut_ptr().add(offt) as *mut T).write_volatile(v) }
    }

    pub fn read<T>(&self, offt: usize) -> T {
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
pub struct VqDesc {
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
pub struct VqAvail<const N: usize> {
    // #define VIRTQ_AVAIL_F_NO_INTERRUPT      1
    pub flags: u16,
    pub idx: u16,
    pub ring: [u16; N],
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
pub struct VqUsedElem {
    /* Index of start of used descriptor chain. */
    pub id: u32,
    /* Total length of the descriptor chain which
    was used (written to) */
    pub len: u32,
}

#[repr(packed, C)]
#[derive(Debug)]
pub struct VqUsed<const N: usize> {
    // #define VIRTQ_USED_F_NO_NOTIFY  1
    flags: u16,
    pub idx: u16,
    pub ring: [VqUsedElem; N],
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

pub struct Q<const N: usize> {
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
                self.desc_data[f as usize] = 0;
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

    pub fn pop_used(&mut self) {
        let used = self.used.ring[self.used_pos as usize % N];
        self.free_desc(used.id as usize);
        self.used_pos = self.used_pos.wrapping_add(1);
    }

    pub fn peek_used(&self) -> Option<(&VqDesc, u64)> {
        if self.used_pos == self.used.idx {
            return None;
        }
        let used = self.used.ring[self.used_pos as usize % N];
        let data = self.desc_data[self.used_pos as usize % N];
        Some((self.get_desc(used.id as usize), data))
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

    pub fn get_desc_mut(&mut self, idx: usize) -> &mut VqDesc {
        &mut self.desc[idx]
    }

    pub fn get_desc(&self, idx: usize) -> &VqDesc {
        &self.desc[idx]
    }

    pub fn set_desc_data(&mut self, idx: usize, data: u64) {
        self.desc_data[idx] = data;
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

    pub fn add_avail(&mut self, head: u16) -> u16 {
        let used_idx = self.used.idx;
        self.avail.ring[self.avail.idx as usize % N] = head;
        self.avail.idx = self.avail.idx.wrapping_add(1);
        dsb!();
        used_idx
    }

    pub fn len(&self) -> u32 {
        N as u32
    }

    pub fn wait_use(&self, old_use: u16) {
        while self.used.idx == old_use {
            spin_loop();
        }
    }
}

#[inline]
pub fn select_q(regs: &mut Regs, pos: u32) {
    regs.write(Regs::QUEUESEL, pos);
    dsb!();
}

#[inline]
pub fn get_qlen_max(regs: &mut Regs, qpos: u32) -> u32 {
    select_q(regs, qpos);
    regs.read(Regs::QUEUENUMMAX)
}

#[inline]
pub fn set_ready(regs: &mut Regs, qpos: u32) {
    select_q(regs, qpos);
    regs.write(Regs::QUEUEREADY, 1u32);
    dsb!();
}

#[inline]
pub fn notify_q(regs: &mut Regs, qpos: u32) {
    select_q(regs, qpos);
    regs.write(Regs::QUEUENOTIFY, qpos);
    dsb!();
}

#[inline]
pub fn get_status(regs: &mut Regs) -> u32 {
    regs.read(Regs::STATUS)
}

#[inline]
pub fn get_irq_status(regs: &mut Regs) -> u32 {
    regs.read(Regs::INTERRUPTSTATUS)
}

#[inline]
pub fn irq_ack(regs: &mut Regs, v: u32) {
    regs.write(Regs::INTERRUPTACK, v)
}

#[inline]
pub fn set_desc_area(regs: &mut Regs, paddr: (u32, u32)) {
    regs.write(Regs::QUEUEDESCLOW, paddr.0);
    regs.write(Regs::QUEUEDESCHIGH, paddr.1);
    dsb!();
}

#[inline]
pub fn set_used_area(regs: &mut Regs, paddr: (u32, u32)) {
    regs.write(Regs::QUEUEDEVICELOW, paddr.0);
    regs.write(Regs::QUEUEDEVICEHIGH, paddr.1);
    dsb!();
}

#[inline]
pub fn set_avail_area(regs: &mut Regs, paddr: (u32, u32)) {
    regs.write(Regs::QUEUEDRIVERLOW, paddr.0);
    regs.write(Regs::QUEUEDRIVERHIGH, paddr.1);
    dsb!();
}

#[inline]
pub fn set_q_len(regs: &mut Regs, qpos: u32, len: u32) {
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
                rng::init(reg);
            }
            9 => {
                // virtio-9p
                print!("virtio-9p found.\n");
                p9::init(reg);
            }
            _ => {}
        }
    }
}

pub fn init_dev_common(reg: &mut Regs, features: u32) {
    reg.write::<u32>(Regs::STATUS, 0);
    dsb!();
    let mut status: u32 = reg.read(Regs::STATUS);
    reg.write(Regs::STATUS, status | Status::ACKNOWLEDGE);
    dsb!();
    reg.write(Regs::STATUS, status | Status::DRIVER);
    dsb!();
    reg.write(Regs::DEVICEFEATURESSEL, 0u32);
    reg.write(Regs::DRIVERFEATURESSEL, 0u32);
    dsb!();
    // let device_features: u32 = reg.read(Regs::DEVICEFEATURES);
    reg.write(Regs::DRIVERFEATURES, features);
    status = reg.read(Regs::STATUS);
    dsb!();
    reg.write(Regs::STATUS, status | Status::FEATURES_OK);
    dsb!();
    status = reg.read(Regs::STATUS);
    if (status & Status::FEATURES_OK) == 0 {
        panic!("virt feature not ok.");
    }
}

pub mod p9 {
    use core::{
        arch::asm,
        hint::spin_loop,
        mem::{ManuallyDrop, forget},
        ptr::NonNull,
    };

    use alloc::{str, vec::Vec};

    use crate::{
        dsb, print,
        rng::irq_pending,
        spin::Lock,
        stuff::BitSet128,
        virtio::{self, Q, Regs, Status, get_irq_status, init_dev_common, irq_ack},
    };

    enum Buf<'a> {
        Own(Vec<u8>),
        Borrowed(&'a [u8]),
    }

    struct Msg<'a> {
        buf: Buf<'a>,
        pos: usize,
    }

    impl<'a> Msg<'a> {
        fn new_own() -> Msg<'a> {
            Msg {
                buf: Buf::Own(Vec::new()),
                pos: 0,
            }
        }

        fn new_borrowed(buf: &'a [u8]) -> Msg<'a> {
            Msg {
                buf: Buf::Borrowed(buf),
                pos: 0,
            }
        }

        pub fn get_buf(&self) -> &[u8] {
            match &self.buf {
                Buf::Own(v) => v.as_slice(),
                Buf::Borrowed(s) => s,
            }
        }

        fn get_vec(&mut self) -> &mut Vec<u8> {
            match &mut self.buf {
                Buf::Own(v) => v,
                _ => panic!("read only"),
            }
        }

        fn get_buf_ptr(&self) -> *const u8 {
            (&self.get_buf()[0]) as *const u8
        }

        fn get_self_ptr(&self) -> u64 {
            self as *const Msg as u64
        }

        pub fn read_u8(&mut self) -> Option<u8> {
            let buf = self.get_buf();
            if buf.len() < self.pos {
                return None;
            }
            let b = buf[self.pos];
            self.pos += 1;
            Some(b)
        }

        pub fn read_u16(&mut self) -> Option<u16> {
            let buf = self.get_buf();
            if buf.len() < self.pos + 1 {
                return None;
            }
            let w = u16::from_le_bytes(buf[self.pos..][0..2].try_into().unwrap());
            self.pos += 2;
            Some(w)
        }

        pub fn read_u32(&mut self) -> Option<u32> {
            let buf = self.get_buf();
            if buf.len() < self.pos + 3 {
                return None;
            }
            let d = u32::from_le_bytes(buf[self.pos..][0..4].try_into().unwrap());
            self.pos += 4;
            Some(d)
        }

        pub fn read_u64(&mut self) -> Option<u64> {
            let buf = self.get_buf();
            if buf.len() < self.pos + 7 {
                return None;
            }
            let q = u64::from_le_bytes(buf[self.pos..][0..8].try_into().unwrap());
            self.pos += 8;
            Some(q)
        }

        pub fn read_str(&mut self) -> Option<&str> {
            let len = self.read_u16().unwrap() as usize;
            let buf = self.get_buf();
            if self.pos + len < buf.len() {
                return None;
            }
            match str::from_utf8(&buf[self.pos..][0..len as usize]) {
                Ok(s) => Some(s),
                Err(_) => None,
            }
        }

        pub fn write_slice(&mut self, slice: &[u8]) {
            let pos = self.pos;
            let vec = self.get_vec();
            if pos + slice.len() > vec.len() {
                vec.resize(pos + slice.len(), 0);
            }
            vec[pos..][0..slice.len()].copy_from_slice(slice);
            self.pos += slice.len();
        }

        pub fn ensure(&mut self, ns: usize) {
            let vec = self.get_vec();
            if ns > vec.len() {
                vec.resize(ns, 0);
            }
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
    #[derive(Debug, Clone, Copy)]
    pub enum QIDKind {
        QTDIR = 0x80,
        QTAPPEND = 0x40,
        QTEXCL = 0x20,
        QTMOUNT = 0x10,
        QTAUTH = 0x08,
        QTTMP = 0x04,
        QTSYMLINK = 0x02,
        QTLINK = 0x01,
        QTFILE = 0x00,
    }

    impl TryFrom<u8> for QIDKind {
        type Error = ();
        fn try_from(value: u8) -> Result<Self, Self::Error> {
            match value {
                0x0 => Ok(QIDKind::QTFILE),
                0x01 => Ok(QIDKind::QTLINK),
                0x02 => Ok(QIDKind::QTSYMLINK),
                0x04 => Ok(QIDKind::QTTMP),
                0x08 => Ok(QIDKind::QTAUTH),
                0x10 => Ok(QIDKind::QTMOUNT),
                0x20 => Ok(QIDKind::QTEXCL),
                0x40 => Ok(QIDKind::QTAPPEND),
                0x80 => Ok(QIDKind::QTDIR),
                _ => Err(()),
            }
        }
    }

    #[derive(Clone, Copy, Debug)]
    pub struct QID {
        pub kind: QIDKind,
        pub version: u32,
        pub path: u64,
    }

    impl QID {
        const fn new() -> QID {
            QID {
                kind: QIDKind::QTDIR,
                version: 0,
                path: 0,
            }
        }
    }

    struct P9 {
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
            assert!(fid < self.fid_bs.len() as u32);
            assert!(self.fid_bs.tst(fid as u8));
            self.fid_bs.clr(fid as u8);
        }

        pub fn next_tag(&mut self) -> u16 {
            let tag = self.tag;
            self.tag = tag.wrapping_add(1);
            tag
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

    fn set_version(p9: &mut P9) {
        //size[4] Tversion tag[2] msize[4] version[s]
        let mut msg = Msg::new_own();
        msg.write_u32(0);
        msg.write_u8(Op::TVERSION as u8);
        msg.write_u16(0);
        msg.write_u32(u16::MAX as u32);
        let vpos = msg.tell();
        msg.write_str(VERSION);
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
            .set_len(len as u32)
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
        // print!("rv: {}\n", rv);
    }

    pub fn attach(p9: &mut P9) {
        // size[4] Tattach tag[2] fid[4] afid[4] uname[s] aname[s] n_uname[4]
        // size[4] Rattach tag[2] qid[13]
        let mut msg = Msg::new_own();
        msg.ensure(20);
        msg.write_u32(0);
        msg.write_u8(Op::TATTACH as u8);
        msg.write_u16(p9.next_tag());
        msg.write_u32(p9.alloc_fid().unwrap());
        msg.write_u32(!0u32);
        msg.write_str("root");
        msg.write_str("");
        msg.write_u32(0);
        let len = msg.tell();
        msg.seek(0);
        msg.write_u32(len as u32);
        // print!("len = {}\n", len);

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

    pub fn walk(path: &'static str) -> Result<(u32, QID), ()> {
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
        print!("wnames {:?}\n", wnames);
        // size[4] Twalk tag[2] fid[4] newfid[4] nwname[2] nwname*(wname[s])
        // size[4] Rwalk tag[2] nwqid[2] nwqid*(wqid[13])
        let mut msg = Msg::new_own();
        let resp_len = 22 + 13 * wnames.len();
        msg.ensure(resp_len);
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
        let old = p9.q.add_avail(d1);

        let regs = unsafe { p9.regs.unwrap().as_mut() };
        virtio::set_ready(regs, 0);
        virtio::notify_q(regs, 0);

        {
            p9.q.wait_use(old);
            p9.q.pop_used();
            let irq_s = get_irq_status(regs);
            irq_ack(regs, irq_s);
        }
        // TODO sleep

        msg.seek(4);
        let resp_kind = msg.read_u8().unwrap();
        print!("RESP: {}\n", resp_kind);
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

        let mut qid = QID::new();

        qid.kind = msg.read_u8().unwrap().try_into().unwrap();
        qid.version = msg.read_u32().unwrap();
        qid.path = msg.read_u64().unwrap();

        Ok((fid, qid))
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
                //TODO wake
            }
            p9.q.pop_used();
        }
        virtio::irq_ack(regs, irq_status);
    }

    pub fn init(regs: &mut Regs) {
        let lock = P9L.acquire();
        let p9 = lock.as_mut();

        if p9.regs.is_some() {
            // TODO
            return;
        }

        p9.regs = NonNull::new(regs as *mut Regs);

        init_dev_common(regs, 0);

        let status: u32 = regs.read(Regs::STATUS);
        regs.write(Regs::STATUS, status | Status::DRIVER_OK);
        dsb!();

        virtio::set_q_len(regs, 0, p9.q.len());
        virtio::set_used_area(regs, p9.q.used_area_paddr());
        virtio::set_avail_area(regs, p9.q.avail_area_paddr());
        virtio::set_desc_area(regs, p9.q.desc_area_paddr());
        dsb!();

        set_version(p9);
        attach(p9);
    }
}
