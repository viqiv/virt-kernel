ENTRY(_start)

SECTIONS {
    . = 0x40100000;
    
    .boot.text :  
        {
        KEEP(*(.boot.text.*))
        KEEP(*(.boot.text))
    }
    
    .boot.bss : 
        {
        KEEP(*(.boot.bss))
        KEEP(*(.boot.bss.*))
    }
    

    .boot.rodata : 
        {
        KEEP(*(.boot.rodata.*))
        KEEP(*(.boot.rodata))
    }

    
    .boot.data : 
        {
        KEEP(*(.boot.data))
        KEEP(*(.boot.data.*))
    }

    . += 0xffff000000000000;
    .text.vector ALIGN(4K) :
        AT(ADDR(.text.vector) - 0xffff000000000000) 
         {
        _kernel_begin = . - 0xffff000000000000;
        _trap_vec = .;
        . += 0x200;
        KEEP(*(.text.vector));
    }
    

    .text ALIGN(4K) : 
        AT(ADDR(.text) - 0xffff000000000000) 
        {
        *(.text)
        *(.text.*)
    }

    .data : AT(ADDR(.data) - 0xffff000000000000) {
        *(.data)
        *(.data.*)
    }

    .rodata : AT(ADDR(.rodata) - 0xffff000000000000) {
        *(.rodata)
        *(.rodata.*)
    }

    .bss : AT(ADDR(.bss) - 0xffff000000000000) 
    {
        . = ALIGN(4096);
        _boot_stack_btm = .;
        . += (4096);
        _boot_stack = .;
        *(.bss.*)
        *(.bss)
    }

    _kernel_end = . - 0xffff000000000000;
}
