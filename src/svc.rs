use crate::{
    fs, print,
    sched::{self, mycpu},
};

pub fn handle() {
    let task = mycpu().get_task().unwrap();
    let tf = task.get_trap_frame().unwrap();
    tf.regs[0] = match tf.regs[8] {
        63 => fs::sys_read(),
        64 => fs::sys_write(),
        93 => sched::exit(),
        95 => sched::wait(),
        172 => sched::getpid(),
        220 => sched::fork(),
        _ => panic!("unimplemented syscall {}\n", tf.regs[8]),
    }
}
