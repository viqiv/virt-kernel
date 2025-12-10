use core::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
    ptr::{slice_from_raw_parts, slice_from_raw_parts_mut},
};

pub struct BitSet128 {
    back: u128,
    len: u8,
}

impl BitSet128 {
    pub const fn new(len: u8) -> Self {
        assert!(len <= 128);
        Self {
            back: if len == 128 { 0 } else { (!0u128) << len },
            len,
        }
    }

    pub fn len(&self) -> u8 {
        self.len
    }

    #[inline]
    pub fn set(&mut self, bit: u8) {
        assert!(bit < self.len);
        self.back |= 1u128 << bit
    }

    #[inline]
    pub fn clr(&mut self, bit: u8) {
        assert!(bit < self.len);
        self.back &= !(1u128 << bit)
    }

    #[inline]
    pub fn tst(&self, bit: u8) -> bool {
        assert!(bit < self.len);
        (self.back & 1u128 << bit) != 0
    }

    #[inline]
    pub fn full(&self) -> bool {
        self.back == !0u128
    }

    #[inline]
    pub fn first_clr(&self) -> Option<u8> {
        if self.full() {
            return None;
        }

        for i in 0..self.len {
            if !self.tst(i) {
                return Some(i);
            }
        }

        None
    }
}

pub fn as_slice_mut<'a, T>(ptr: *mut T, len: usize) -> &'a mut [T] {
    unsafe { slice_from_raw_parts_mut(ptr, len).as_mut() }.unwrap()
}

pub fn as_slice<'a, T>(ptr: *const T, len: usize) -> &'a [T] {
    unsafe { slice_from_raw_parts(ptr, len).as_ref() }.unwrap()
}

pub fn strlen(ptr: *const u8) -> usize {
    for i in 0.. {
        if unsafe { ptr.add(i).read() == 0 } {
            return i;
        }
    }
    return 0;
}

pub fn cstr_as_slice<'a>(ptr: *const u8) -> &'a [u8] {
    let len = strlen(ptr);
    as_slice(ptr, len)
}

pub struct Defer<F: FnOnce()> {
    f: Option<F>,
}

impl<F: FnOnce()> Drop for Defer<F> {
    fn drop(&mut self) {
        (self.f.take().unwrap())();
    }
}

pub fn defer<F: FnOnce()>(f: F) -> Defer<F> {
    Defer { f: Some(f) }
}
