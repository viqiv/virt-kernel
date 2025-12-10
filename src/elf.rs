/* 64-bit ELF base types. */
type Elf64Addr = u64;
type Elf64Half = u16;
type Elf64SHalf = i16;
type Elf64Off = u64;
type Elf64Sword = i32;
type Elf64Word = u32;
type Elf64Xword = u64;
type Elf64Sxword = i64;
type Elf64Versym = u16;

/* These constants are for the segment types stored in the image headers */
const PT_NULL: u64 = 0;
const PT_LOAD: u64 = 1;
const PT_DYNAMIC: u64 = 2;
const PT_INTERP: u64 = 3;
const PT_NOTE: u64 = 4;
const PT_SHLIB: u64 = 5;
const PT_PHDR: u64 = 6;
const PT_TLS: u64 = 7; /* Thread local storage segment */
const PT_LOOS: u64 = 0x60000000; /* OS-specific */
const PT_HIOS: u64 = 0x6fffffff; /* OS-specific */
const PT_LOPROC: u64 = 0x70000000;
const PT_HIPROC: u64 = 0x7fffffff;
const PT_GNU_EH_FRAME: u64 = PT_LOOS + 0x474e550;
const PT_GNU_STACK: u64 = PT_LOOS + 0x474e551;
const PT_GNU_RELRO: u64 = PT_LOOS + 0x474e552;
const PT_GNU_PROPERTY: u64 = PT_LOOS + 0x474e553;

/* ARM MTE memory tag segment type */
const PT_AARCH64_MEMTAG_MTE: u64 = PT_LOPROC + 0x2;

/* These constants define the different elf file types */
const ET_NONE: u64 = 0;
const ET_REL: u64 = 1;
const ET_EXEC: u64 = 2;
const ET_DYN: u64 = 3;
const ET_CORE: u64 = 4;
const ET_LOPROC: u64 = 0xff00;
const ET_HIPROC: u64 = 0xffff;

const EI_NIDENT: usize = 16;

#[repr(C)]
struct Elf64hdr {
    ident: [u8; EI_NIDENT], /* ELF "magic number" */
    kind: Elf64Half,
    machine: Elf64Half,
    version: Elf64Word,
    entry: Elf64Addr, /* Entry point virtual address */
    phoff: Elf64Off,  /* Program header table file offset */
    shoff: Elf64Off,  /* Section header table file offset */
    flags: Elf64Word,
    ehsize: Elf64Half,
    phentsize: Elf64Half,
    phnum: Elf64Half,
    shentsize: Elf64Half,
    shnum: Elf64Half,
    shstrndx: Elf64Half,
}

/* These constants define the permissions on sections in the program
header, p_flags. */
const PF_R: u64 = 0x4;
const PF_W: u64 = 0x2;
const PF_X: u64 = 0x1;

#[repr(C)]
struct Elf64phdr {
    kind: Elf64Word,
    flags: Elf64Word,
    offset: Elf64Off,   /* Segment file offset */
    vaddr: Elf64Addr,   /* Segment virtual address */
    paddr: Elf64Addr,   /* Segment physical address */
    filesz: Elf64Xword, /* Segment size in file */
    memsz: Elf64Xword,  /* Segment size in memory */
    align: Elf64Xword,  /* Segment alignment, file & memory */
}
