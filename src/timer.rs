use core::arch::asm;

#[allow(unused)]
#[inline]
pub fn r_freq() -> u64 {
    let mut r = 0u64;
    unsafe { asm!("MRS {}, CNTFRQ_EL0", out(reg) r) };
    r
}

#[allow(unused)]
#[inline]
pub fn w_freq(r: u64) {
    unsafe { asm!("MSR CNTFRQ_EL0, {}", in(reg) r) };
}

#[allow(unused)]
#[inline]
pub fn r_pctl_el0() -> u64 {
    let mut r = 0u64;
    unsafe { asm!("MRS {}, CNTP_CTL_EL0", out(reg) r) };
    r
}

#[allow(unused)]
#[inline]
pub fn w_pctl_el0(r: u64) {
    unsafe { asm!("MSR CNTP_CTL_EL0, {}", in(reg) r) };
}

#[allow(unused)]
#[inline]
pub fn r_pct_el0() -> u64 {
    let mut r = 0u64;
    unsafe { asm!("MRS {}, CNTPCT_EL0", out(reg) r) };
    r
}

#[allow(unused)]
#[inline]
pub fn r_ptval_el0() -> u64 {
    let mut r = 0u64;
    unsafe { asm!("MRS {}, CNTP_TVAL_EL0", out(reg) r) };
    r
}

#[allow(unused)]
#[inline]
pub fn w_ptval_el0(r: u64) {
    unsafe { asm!("MSR CNTP_TVAL_EL0, {}", in(reg) r) };
}

#[allow(unused)]
#[inline]
pub fn r_pcval_el0() -> u64 {
    let mut r = 0u64;
    unsafe { asm!("MRS {}, CNTP_CVAL_EL0", out(reg) r) };
    r
}

#[allow(unused)]
#[inline]
pub fn w_pcval_el0(r: u64) {
    unsafe { asm!("MSR CNTP_CVAL_EL0, {}", in(reg) r) };
}

pub fn init() {
    // trap::gic_enable_intr(30);
    // w_pctl_el0(1);
}
