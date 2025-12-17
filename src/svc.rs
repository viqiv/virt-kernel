use crate::{
    fs, print,
    sched::{self, mycpu},
};

pub fn handle() {
    let task = mycpu().get_task().unwrap();
    let tf = task.get_trap_frame().unwrap();
    print!("++ svc {}\n", tf.regs[8]);
    tf.regs[0] = match tf.regs[8] {
        17 => sched::getcwd(),
        24 => fs::dup3(),
        25 => fs::fcntl(),
        29 => fs::ioctl(),
        56 => fs::openat(),
        57 => fs::close(),
        62 => fs::lseek(),
        63 => fs::sys_read(),
        64 => fs::sys_write(),
        73 => fs::ppoll(),
        78 => fs::readlinkat(),
        79 => fs::newfsstatat(),
        80 => fs::fstat(),
        94 => sched::exit_group(),
        96 => sched::settid(),
        99 => sched::set_robust_list(),
        129 => sched::kill(),
        134 => sched::rt_sigaction(),
        154 => sched::setpgid(),
        155 => sched::getgid(),
        160 => sched::uname(),
        174 => sched::getuid(),
        172 => sched::getpid(),
        173 => sched::getppid(),
        176 => sched::getgid(),
        214 => sched::brk(),
        215 => sched::munmap(),
        220 => sched::fork(),
        221 => sched::execve(),
        222 => sched::mmap(),
        226 => sched::mprotect(),
        260 => sched::wait4(),
        261 => sched::prlimit64(),
        278 => fs::getrandom(),
        293 => sched::rseq(),
        // 93 => sched::exit(),
        // 95 => sched::wait(),
        _ => panic!("unimplemented syscall {}\n", tf.regs[8]),
    }
}
