# virt-kernel
kernel for virt aarch64 board on qemu.

```bash
m@127:~/Desktop/aarch64/xyz$ cargo run --target aarch64-unknown-none-softfloat --release
    Finished `release` profile [optimized + debuginfo] target(s) in 0.03s
     Running `/home/m/Desktop/aarch64/xyz/./run.sh target/aarch64-unknown-none-softfloat/release/xyz`
/ # ls
bin        build.zig  busybox    disas.txt  etc        init.c     main.zig
/ # cat main.zig 
const std = @import("std");

std.os.linux.Stat {}
/ # stat busybox
  File: busybox
  Size: 2237080   	Blocks: 547        IO Block: 4096   regular file
Device: 0h/0d	Inode: 3399988123389603631  Links: 1
Access: (0775/-rwxrwxr-x)  Uid: ( 1000/ UNKNOWN)   Gid: ( 1000/ UNKNOWN)
Access: 2026-01-02 18:09:17.858993459 +0000
Modify: 2025-12-16 12:09:31.000000000 +0000
Change: 1970-01-01 00:00:00.000000000 +0000
/ # 
/ # 
```
