use core::{arch::asm, hint::spin_loop, ptr::slice_from_raw_parts_mut};

use alloc::vec::Vec;

use crate::{
    blk, dsb, p9, print, rng,
    stuff::BitSet128,
    vm::{self, map, map2},
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

#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct Volatile<T> {
    v: T,
}

impl<T> Volatile<T> {
    pub const fn new(v: T) -> Volatile<T> {
        Volatile { v }
    }

    pub fn read(&self) -> T {
        unsafe { (&self.v as *const T).read_volatile() }
    }

    pub fn write(&mut self, v: T) {
        unsafe { (&mut self.v as *mut T).write_volatile(v) }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VqDesc {
    /* Address (guest-physical). */
    addr: Volatile<u64>,
    /* Length.*/
    len: Volatile<u32>,
    /* The flags as indicated above. */
    flags: Volatile<u16>,
    /* Next field if flags & NEXT */
    next: Volatile<u16>,
}

impl VqDesc {
    /* This marks a buffer as continuing via the next field. */
    pub const F_NEXT: u16 = 1;
    /* This marks a buffer as device write-only (otherwise device read-only). */
    pub const F_WRITE: u16 = 2;
    /* This means the buffer contains a list of buffer descriptors. */
    pub const F_INDIRECT: u16 = 4;

    pub fn set_next(&mut self, idx: u16) -> &mut Self {
        self.next.write(idx);
        self.flags.write(self.flags.read() | Self::F_NEXT);
        self
    }

    pub fn get_next(&self) -> Option<u16> {
        if (self.flags.read() & Self::F_NEXT) == 0 {
            None
        } else {
            Some(self.next.read())
        }
    }

    pub fn set_writable(&mut self) -> &mut Self {
        self.flags.write(self.flags.read() | Self::F_WRITE);
        self
    }

    pub fn set_readable(&mut self) -> &mut Self {
        self
    }

    pub fn set_len(&mut self, len: u32) -> &mut Self {
        self.len.write(len);
        self
    }

    pub fn set_data(&mut self, data: u64) -> &mut Self {
        self.addr.write(vm::v2p(data as usize).unwrap() as u64);
        self
    }
}

impl VqDesc {
    pub const fn zeroed() -> VqDesc {
        VqDesc {
            addr: Volatile::new(0),
            len: Volatile::new(0),
            flags: Volatile::new(0),
            next: Volatile::new(0),
        }
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct VqAvail<const N: usize> {
    // #define VIRTQ_AVAIL_F_NO_INTERRUPT      1
    pub flags: Volatile<u16>,
    pub idx: Volatile<u16>,
    pub ring: [Volatile<u16>; N],
    used_event: Volatile<u16>, /* Only if VIRTIO_F_EVENT_IDX */
}

impl<const N: usize> VqAvail<N> {
    pub const fn zeroed() -> Self {
        Self {
            flags: Volatile::new(0),
            idx: Volatile::new(0),
            ring: [Volatile::new(0); N],
            used_event: Volatile::new(0),
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

#[repr(C)]
#[derive(Debug)]
pub struct VqUsed<const N: usize> {
    // #define VIRTQ_USED_F_NO_NOTIFY  1
    flags: Volatile<u16>,
    pub idx: Volatile<u16>,
    pub ring: [Volatile<VqUsedElem>; N],
    avail_event: Volatile<u16>, /* Only if VIRTIO_F_EVENT_IDX */
}

impl<const N: usize> VqUsed<N> {
    pub const fn zeroed() -> Self {
        Self {
            flags: Volatile::new(0),
            idx: Volatile::new(0),
            ring: [Volatile::new(VqUsedElem { id: 0, len: 0 }); N],
            avail_event: Volatile::new(0),
        }
    }
}

pub struct Q<const N: usize> {
    desc: [VqDesc; N],
    avail: VqAvail<N>,
    pub used: VqUsed<N>,
    // data
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
            assert!(self.desc_bs.tst(nidx as u8));
            self.desc_bs.clr(nidx as u8);
            d = self.get_desc(nidx as usize);
            i += 1;
        }
    }

    pub fn pop_used(&mut self) {
        let used = (&self.used.ring[self.used_pos as usize % N]).read();
        self.free_desc(used.id as usize);
        self.used_pos = self.used_pos.wrapping_add(1);
    }

    pub fn peek_used(&self) -> Option<(&VqDesc, u64)> {
        if self.used_pos == self.used.idx.read() {
            return None;
        }
        let used = (&self.used.ring[self.used_pos as usize % N]).read();
        let data = self.desc_data[used.id as usize % N];
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
        let used_idx = self.used.idx.read();
        self.avail.ring[self.avail.idx.read() as usize % N].write(head);
        self.avail.idx.write(self.avail.idx.read().wrapping_add(1));
        dsb!();
        used_idx
    }

    pub fn len(&self) -> u32 {
        N as u32
    }

    pub fn wait_use(&self, old_use: u16) {
        while self.used.idx.read() == old_use {
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

// static REGS: StaticMut<&mut [Regs]> = StaticMut::new(&mut []);

pub fn init() {
    let perm = vm::PR_PW;
    let maps = [
        map(0xa000000, 1, perm).unwrap(),
        map(0xa000000 + 4096, 1, perm).unwrap(),
        map(0xa000000 + 4096 * 2, 1, perm).unwrap(),
        map(0xa000000 + 4096 * 3, 1, perm).unwrap(),
    ];
    let mut irq_n = 0x10 + 32;
    for m in 0..4 {
        let regs = unsafe { slice_from_raw_parts_mut(maps[m] as *mut Regs, 8).as_mut() }.unwrap();
        for i in 0..8 {
            let reg = &mut regs[i];

            assert!(reg.read::<u32>(Regs::MAGICVALUE) == 0x74726976);
            assert!(reg.read::<u32>(Regs::VERSION) == 2);
            let id: u32 = reg.read(Regs::DEVICEID);

            match id {
                2 => {
                    // virtio-blk
                    print!("virtio-blk found.\n");
                    // blk::init(reg);
                }
                4 => {
                    // virtio-rng
                    print!("virtio-rng found.\n");
                    // rng::init(reg);
                }
                9 => {
                    // virtio-9p
                    print!("virtio-9p found.\n");
                    p9::init(reg, irq_n);
                }
                _ => {}
            }
            irq_n += 1;
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
