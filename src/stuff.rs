use core::{
    cell::UnsafeCell,
    ops::{Deref, DerefMut},
};

use crate::print;

pub struct StaticMut<T> {
    data: UnsafeCell<T>,
}

unsafe impl<T> Sync for StaticMut<T> {}

impl<T> StaticMut<T> {
    pub const fn new(data: T) -> StaticMut<T> {
        StaticMut {
            data: UnsafeCell::new(data),
        }
    }

    pub fn get(&self) -> &T {
        unsafe { self.data.get().as_ref() }.unwrap()
    }

    pub fn set(&self, v: T) {
        unsafe {
            self.data.get().write(v);
        }
    }

    pub fn get_mut(&self) -> &mut T {
        unsafe { self.data.get().as_mut() }.unwrap()
    }
}

impl<T> Deref for StaticMut<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.get()
    }
}

impl<T> DerefMut for StaticMut<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.get_mut()
    }
}

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
