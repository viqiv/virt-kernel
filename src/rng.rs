use crate::{
    dsb, print,
    spin::Lock,
    virtio::{self, Q, Regs, Status, get_irq_status, init_dev_common},
};
use core::{arch::asm, hint::spin_loop, ptr::NonNull};

const QSIZE: usize = 2;

struct VirtioRng {
    regs: NonNull<Regs>,
    vq: Q<QSIZE>,
}
static RNG: Lock<VirtioRng> = Lock::new(
    "virtio-rng",
    VirtioRng {
        regs: NonNull::dangling(),
        vq: Q::new(),
    },
);

pub fn init(reg: &mut Regs) {
    let lock = RNG.acquire();
    let rng = lock.as_mut();

    if rng.regs != NonNull::dangling() {
        /*TODO*/
        return;
    }

    rng.regs = NonNull::new(reg as *mut Regs).unwrap();

    init_dev_common(reg, 0);

    let status: u32 = reg.read(Regs::STATUS);
    reg.write(Regs::STATUS, status | Status::DRIVER_OK);
    dsb!();

    virtio::set_q_len(reg, 0, rng.vq.len());
    virtio::set_used_area(reg, rng.vq.used_area_paddr());
    virtio::set_avail_area(reg, rng.vq.avail_area_paddr());
    virtio::set_desc_area(reg, rng.vq.desc_area_paddr());
    dsb!();
}

pub fn read_inner(buf: &mut [u8], sync: bool) -> Result<usize, ()> {
    let lock = RNG.acquire();
    let rng = lock.as_mut();
    let d = rng.vq.alloc_desc().unwrap();
    let desc = rng.vq.get_desc_mut(d as usize);

    let ptr = (&buf[0]) as *const u8;

    desc.set_writable()
        .set_data(ptr as u64)
        .set_len(buf.len() as u32);

    rng.vq.desc_data[d as usize] = if sync { 0 } else { ptr as u64 };
    let regs = unsafe { rng.regs.as_mut() };

    let old = rng.vq.add_avail(d);
    virtio::set_ready(regs, 0);
    virtio::notify_q(regs, 0);

    if sync {
        rng.vq.wait_use(old);
        drop(lock);
        irq_handle();
    } else {
        //TODO sleep on ptr here
    }

    Ok(buf.len())
}

pub fn read(buf: &mut [u8]) -> Result<usize, ()> {
    read_inner(buf, false)
}

pub fn read_sync(buf: &mut [u8]) -> Result<usize, ()> {
    read_inner(buf, true)
}

pub fn irq_pending() -> bool {
    let lock = RNG.acquire();
    let rng = lock.as_mut();
    get_irq_status(unsafe { rng.regs.as_mut() }) != 0
}

pub fn irq_handle() {
    let lock = RNG.acquire();
    let rng = lock.as_mut();
    assert!(rng.regs != NonNull::dangling());
    let regs = unsafe { rng.regs.as_mut() };
    let irq_status = virtio::get_irq_status(regs);

    if irq_status & 2 > 0 {
        panic!("device config changed.");
    }

    while let Some((_, data)) = rng.vq.peek_used() {
        if data != 0 {
            //TODO wake on data here
        }
        rng.vq.pop_used();
    }

    virtio::irq_ack(regs, irq_status);
}
