use core::sync::atomic::{AtomicU16, Ordering};

use crate::{heap::SyncUnsafeCell, p9, spin::Lock};

pub enum FileKind {
    None,
    Used,
    P9(&'static p9::File),
}

pub struct File {
    kind: FileKind,
    rc: AtomicU16,
    offt: usize,
}

impl File {
    pub const fn zeroed() -> File {
        File {
            kind: FileKind::None,
            rc: AtomicU16::new(0),
            offt: 0,
        }
    }

    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, ()> {
        if self.rc.load(Ordering::Acquire) == 0 {
            return Err(());
        }
        match self.kind {
            FileKind::P9(p9f) => {
                if let Ok(n) = p9f.read(buf, self.offt) {
                    self.offt = self.offt.wrapping_add(n);
                    Ok(n)
                } else {
                    Err(())
                }
            }
            _ => {
                panic!("read: unhandled file kind.")
            }
        }
    }

    pub fn write(&mut self, buf: &[u8]) -> Result<usize, ()> {
        if self.rc.load(Ordering::Acquire) == 0 {
            return Err(());
        }
        match self.kind {
            FileKind::P9(p9f) => {
                if let Ok(n) = p9f.write(buf, self.offt) {
                    self.offt = self.offt.wrapping_add(n);
                    Ok(n)
                } else {
                    Err(())
                }
            }
            _ => {
                panic!("write: unhandled file kind.")
            }
        }
    }

    pub fn close(&mut self) -> Result<(), ()> {
        if self.rc.load(Ordering::Acquire) == 0 {
            return Ok(());
        }

        if let Ok(0) = self.rc.compare_exchange(
            1,
            0,
            Ordering::AcqRel, //
            Ordering::Relaxed,
        ) {
            match self.kind {
                FileKind::P9(p9f) => {
                    return if let Ok(_) = p9f.close() {
                        self.kind = FileKind::None;
                        Ok(())
                    } else {
                        self.rc.fetch_add(1, Ordering::Release);
                        Err(())
                    };
                }
                _ => panic!("write: unhandled file kind."),
            }
        } else {
            self.rc.fetch_sub(1, Ordering::Release);
        }

        Ok(())
    }

    pub fn seek_to(&mut self, offt: usize) {
        self.offt = offt;
    }

    pub fn seek_by(&mut self, offt: i32) {
        self.offt = if offt > 0 {
            self.offt.wrapping_add(offt as usize)
        } else {
            self.offt.wrapping_sub(offt as usize)
        };
    }
}

const NFILES: usize = 128;

struct Fs {
    files: [File; NFILES],
}

pub fn open(path: &str, mode: u32) -> Result<&'static mut File, ()> {
    if let Some((idx, file)) = alloc_file() {
        return if let Ok(p9file) = p9::open(path, mode) {
            file.kind = FileKind::P9(p9file);
            file.rc = AtomicU16::new(1);
            Ok(file)
        } else {
            free_file(idx);
            Err(())
        };
    }

    Err(())
}

static FS: Lock<Fs> = Lock::new(
    "fs",
    Fs {
        files: [
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
            File::zeroed(),
        ],
    },
);

fn alloc_file() -> Option<(usize, &'static mut File)> {
    let lock = FS.acquire();
    let fs = lock.as_mut();

    for i in 0..fs.files.len() {
        let file = &mut fs.files[i];
        if let FileKind::None = file.kind {
            file.kind = FileKind::Used;
            let steal = unsafe { (file as *mut File).as_mut() }.unwrap();
            return Some((i, steal));
        }
    }

    None
}

fn free_file(idx: usize) {
    let lock = FS.acquire();
    let fs = lock.as_mut();
    fs.files[idx].kind = FileKind::None;
}
