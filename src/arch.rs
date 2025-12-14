#[allow(dead_code, unused)]
use core::arch::asm;
// use core::mem::transmute;
#[inline]
pub fn r_pstate_nzcv() -> u64 {
    let mut res = 0i64;
    unsafe {
        asm!("mrs {}, nzcv", out(reg) res);
    }
    res.cast_unsigned() >> 28
}

#[inline]
pub fn r_pstate_daif() -> u64 {
    let mut res = 0i64;
    unsafe {
        asm!("mrs {}, daif", out(reg) res);
    }
    res.cast_unsigned() >> 6
}

#[inline]
pub fn r_cpacr_el1() -> u64 {
    let mut res = 0i64;
    unsafe {
        asm!("mrs {}, cpacr_el1", out(reg) res);
    }
    res.cast_unsigned() >> 28
}

#[inline]
pub fn w_cpacr_el1(v: u64) {
    unsafe {
        asm!("msr cpacr_el1, {}", in(reg) v);
    }
}

pub fn enable_fp() {
    let cpacr = r_cpacr_el1();
    w_cpacr_el1(cpacr | (3u64 << 20));
}

#[inline]
pub fn r_pstate_cur_el() -> u64 {
    let mut res = 0i64;
    unsafe {
        asm!("mrs {}, currentel", out(reg) res);
    }
    res.cast_unsigned() >> 2
}

#[inline]
pub fn r_pstate_sp_sel() -> u64 {
    let mut res = 0i64;
    unsafe {
        asm!("mrs {}, spsel", out(reg) res);
    }
    res.cast_unsigned()
}

#[inline]
pub fn pstate_i_set() {
    unsafe {
        asm!("msr daifset, #0b10", options(nomem));
    }
}

#[inline]
pub fn pstate_i_clr() {
    unsafe {
        asm!("msr daifclr, #0b10", options(nomem));
    }
}

#[inline]
pub fn r_elr_el1() -> u64 {
    let mut res = 0i64;
    unsafe {
        asm!("mrs {}, elr_el1", out(reg) res);
    }
    res.cast_unsigned()
}

#[inline]
pub fn w_elr_el1(r: u64) {
    unsafe {
        asm!("msr elr_el1, {}", in(reg) r);
    }
}

#[inline]
pub fn r_spsr_el1() -> u64 {
    let mut res = 0i64;
    unsafe {
        asm!("mrs {}, spsr_el1", out(reg) res);
    }
    res.cast_unsigned()
}

#[inline]
pub fn w_spsr_el1(r: u64) {
    unsafe {
        asm!("msr spsr_el1, {}", in(reg) r);
    }
}

#[inline]
pub fn r_vbar_el1() -> u64 {
    let mut res = 0i64;
    unsafe {
        asm!("mrs {}, vbar_el1", out(reg) res);
    }
    res.cast_unsigned()
}

#[inline]
pub fn w_vbar_el1(r: u64) {
    unsafe {
        asm!("msr vbar_el1, {}", in(reg) r);
    }
}

#[inline]
pub fn w_mair_el1(r: u64) {
    unsafe {
        asm!("msr mair_el1, {}", in(reg) r);
    }
}

#[inline]
pub fn r_esr_el1() -> u64 {
    let mut res = 0i64;
    unsafe {
        asm!("mrs {}, esr_el1", out(reg) res);
    }
    res.cast_unsigned()
}

#[inline]
pub fn r_sctlr_el1() -> u64 {
    let mut res = 0i64;
    unsafe {
        asm!("mrs {}, sctlr_el1", out(reg) res);
    }
    res.cast_unsigned()
}

#[inline]
pub fn w_sctlr_el1(r: u64) {
    unsafe {
        asm!("msr sctlr_el1, {}", in(reg) r);
    }
}

#[inline]
pub fn r_tcr_el1() -> u64 {
    let mut res = 0i64;
    unsafe {
        asm!("mrs {}, tcr_el1", out(reg) res);
    }
    res.cast_unsigned()
}

#[inline]
pub fn w_tcr_el1(r: u64) {
    unsafe {
        asm!("msr tcr_el1, {}", in(reg) r);
    }
}

#[inline]
pub fn r_tpidrro_el0() -> u64 {
    let mut res = 0i64;
    unsafe {
        asm!("mrs {}, tpidrro_el0", out(reg) res);
    }
    res.cast_unsigned()
}

#[inline]
pub fn w_tpidrro_el0(r: u64) {
    unsafe {
        asm!("msr tpidrro_el0, {}", in(reg) r);
    }
}

#[inline]
pub fn r_ttbr0_el1() -> u64 {
    let mut res = 0i64;
    unsafe {
        asm!("mrs {}, ttbr0_el1", out(reg) res);
    }
    res.cast_unsigned()
}

#[inline]
pub fn w_ttbr0_el1(r: u64) {
    unsafe {
        asm!("msr ttbr0_el1, {}", in(reg) r);
    }
}

#[inline]
pub fn r_ttbr1_el1() -> u64 {
    let mut res = 0i64;
    unsafe {
        asm!("mrs {}, ttbr1_el1", out(reg) res);
    }
    res.cast_unsigned()
}

#[inline]
pub fn r_far_el1() -> u64 {
    let mut res = 0i64;
    unsafe {
        asm!("mrs {}, far_el1", out(reg) res);
    }
    res.cast_unsigned()
}

#[inline]
pub fn r_sp() -> u64 {
    let mut res = 0i64;
    unsafe {
        asm!("mov {}, sp", out(reg) res);
    }
    res.cast_unsigned()
}

#[inline]
pub fn w_ttbr1_el1(r: u64) {
    unsafe {
        asm!("msr ttbr1_el1, {}", in(reg) r);
    }
}

#[inline]
pub fn tlbi_aside1(asid: u64) {
    unsafe {
        asm!("mov x0, {}",
             "lsl x0, x0, #48",
            "tlbi aside1 , x0", in(reg) asid);
    }
}

#[inline]
pub fn tlbi_vaee1(v: u64) {
    unsafe {
        asm!("tlbi vaae1 , {}", in(reg) v>>12);
    }
}

#[macro_export]
macro_rules! dsb {
    () => {
        unsafe { asm!("dsb sy") }
    };
}

#[macro_export]
macro_rules! dmb {
    () => {
        unsafe { asm!("dmb sy") }
    };
}

#[macro_export]
macro_rules! isb {
    () => {
        unsafe { asm!("isb sy") }
    };
}

#[macro_export]
macro_rules! tlbi_vmalle1 {
    () => {
        unsafe { asm!("tlbi VMALLE1") }
    };
}

#[macro_export]
macro_rules! wfi {
    () => {
        unsafe { asm!("wfi") }
    };
}

#[macro_export]
macro_rules! wfe {
    () => {
        unsafe { asm!("wfe") }
    };
}

#[macro_export]
macro_rules! udf {
    () => {
        unsafe { asm!("udf #0") }
    };
}
