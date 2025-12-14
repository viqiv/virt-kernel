use crate::{
    fs, print,
    sched::{self, mycpu},
};

pub fn handle() {
    let task = mycpu().get_task().unwrap();
    let tf = task.get_trap_frame().unwrap();
    tf.regs[0] = match tf.regs[8] {
        62 => fs::lseek(),
        63 => fs::sys_read(),
        64 => fs::sys_write(),
        78 => fs::readlinkat(),
        80 => fs::fstat(),
        94 => sched::exit_group(),
        96 => sched::settid(),
        99 => sched::set_robust_list(),
        214 => sched::brk(),
        222 => sched::mmap(),
        226 => sched::mprotect(),
        261 => sched::prlimit64(),
        278 => fs::getrandom(),
        293 => sched::rseq(),
        // 93 => sched::exit(),
        // 95 => sched::wait(),
        // 172 => sched::getpid(),
        // 220 => sched::fork(),
        _ => panic!("unimplemented syscall {}\n", tf.regs[8]),
    }
}
