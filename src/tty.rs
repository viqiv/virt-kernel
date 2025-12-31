use crate::{heap::SyncUnsafeCell, print};

#[allow(non_camel_case_types)]
pub struct TC_IFLAGS;
impl TC_IFLAGS {
    pub const IGNBRK: u32 = 1 << 0;
    pub const BRKINT: u32 = 1 << 1;
    pub const IGNPAR: u32 = 1 << 2;
    pub const PARMRK: u32 = 1 << 3;
    pub const INPCK: u32 = 1 << 4;
    pub const ISTRIP: u32 = 1 << 5;
    pub const INLCR: u32 = 1 << 6;
    pub const IGNCR: u32 = 1 << 7;
    pub const ICRNL: u32 = 1 << 8;
    pub const IUCLC: u32 = 1 << 9;
    pub const IXON: u32 = 1 << 10;
    pub const IXANY: u32 = 1 << 11;
    pub const IXOFF: u32 = 1 << 12;
    pub const IMAXBEL: u32 = 1 << 13;
    pub const IUTF8: u32 = 1 << 14;
}

#[allow(non_camel_case_types)]
pub struct TC_OFLAGS;
impl TC_OFLAGS {
    pub const OPOST: u32 = 1 << 0;
    pub const OLCUC: u32 = 1 << 1;
    pub const ONLCR: u32 = 1 << 2;
    pub const OCRNL: u32 = 1 << 3;
    pub const ONOCR: u32 = 1 << 4;
    pub const ONLRET: u32 = 1 << 5;
    pub const OFILL: u32 = 1 << 6;
    pub const OFDEL: u32 = 1 << 7;
}

#[allow(non_camel_case_types)]
pub struct TC_LFLAGS;
impl TC_LFLAGS {
    pub const ISIG: u32 = 1 << 0;
    pub const ICANON: u32 = 1 << 1;
    pub const XCASE: u32 = 1 << 2;
    pub const ECHO: u32 = 1 << 3;
    pub const ECHOE: u32 = 1 << 4;
    pub const ECHOK: u32 = 1 << 5;
    pub const ECHONL: u32 = 1 << 6;
    pub const NOFLSH: u32 = 1 << 7;
    pub const TOSTOP: u32 = 1 << 8;
    pub const ECHOCTL: u32 = 1 << 9;
    pub const ECHOPRT: u32 = 1 << 10;
    pub const ECHOKE: u32 = 1 << 11;
    pub const FLUSHO: u32 = 1 << 12;
}

pub struct V;
impl V {
    pub const INTR: usize = 0;
    pub const QUIT: usize = 1;
    pub const ERASE: usize = 2;
    pub const KILL: usize = 3;
    pub const EOF: usize = 4;
    pub const TIME: usize = 5;
    pub const MIN: usize = 6;
    pub const SWTC: usize = 7;
    pub const START: usize = 8;
    pub const STOP: usize = 9;
    pub const SUSP: usize = 10;
    pub const EOL: usize = 11;
    pub const REPRINT: usize = 12;
    pub const DISCARD: usize = 13;
    pub const WERASE: usize = 14;
    pub const LNEXT: usize = 15;
    pub const EOL2: usize = 16;
}

pub const NCCS: usize = 32;
#[allow(non_camel_case_types)]
pub type cc_t = u8;
#[allow(non_camel_case_types)]
pub type speed_t = u32;

#[repr(C)]
#[derive(Debug)]
pub struct Termios {
    i: u32,
    o: u32,
    c: u32,
    l: u32,
    line: cc_t,
    cc: [cc_t; NCCS],
    ispeed: u32,
    ospeed: u32,
}

#[repr(C)]
#[derive(Debug)]
pub struct Winsize {
    row: u16,
    col: u16,
    xpixel: u16,
    ypixel: u16,
}

// c_iflag = ICRNL
// c_oflag = OPOST | ONLCR
// c_lflag = ICANON | ECHO | ISIG

const fn make_cc() -> [u8; 32] {
    let mut arr = [0; 32];
    arr[V::INTR] = 3;
    arr[V::ERASE] = 127;
    arr[V::KILL] = 21;
    arr[V::EOF] = 4;
    arr
}

static TERMIOS: SyncUnsafeCell<Termios> = SyncUnsafeCell::new(Termios {
    i: TC_IFLAGS::ICRNL,
    o: TC_OFLAGS::OPOST | TC_OFLAGS::ONLCR,
    c: 0,
    l: TC_LFLAGS::ICANON | TC_LFLAGS::ECHO | TC_LFLAGS::ISIG,
    line: 0,
    cc: make_cc(),
    ispeed: 9600,
    ospeed: 9600,
});

pub fn get_termios(ptr: *mut Termios) -> u64 {
    let from = TERMIOS.as_ref();
    let to = unsafe { ptr.as_mut().unwrap() };

    to.i = from.i;
    to.o = from.o;
    to.c = from.c;
    to.l = from.l;

    to.line = from.line;
    // TODO stack smash
    // to.cc = from.cc;
    0
}

pub fn set_termios(ptr: *const Termios) -> u64 {
    let to = TERMIOS.as_mut();
    let from = unsafe { ptr.as_ref().unwrap() };

    to.i = from.i;
    to.o = from.o;
    to.c = from.c;
    to.l = from.l;
    to.line = from.line;
    to.cc = from.cc;

    0
}

pub fn get_winsz(ptr: *mut Winsize) -> u64 {
    let w = unsafe { ptr.as_mut().unwrap() };
    w.row = 24;
    w.col = 80;
    w.xpixel = 0;
    w.ypixel = 0;
    0
}

pub fn echo() -> bool {
    let t = TERMIOS.as_ref();
    t.l & TC_LFLAGS::ECHO != 0
}

pub fn icanon() -> bool {
    let t = TERMIOS.as_ref();
    t.l & TC_LFLAGS::ICANON != 0
}

pub fn opost() -> bool {
    let t = TERMIOS.as_ref();
    t.o & TC_OFLAGS::OPOST != 0
}

pub fn onlcr() -> bool {
    let t = TERMIOS.as_ref();
    t.o & TC_OFLAGS::ONLCR != 0
}
