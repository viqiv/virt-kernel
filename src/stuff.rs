use core::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
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
