use crate::{fs, print, sched::mycpu};

pub fn handle() {
    let task = mycpu().get_task().unwrap();
    let tf = task.get_trap_frame().unwrap();
    // print!("====> svc\n");

    // for i in 0..tf.regs.len() {
    //     print!("[{}] {}\n", i, tf.regs[i]);
    // }
    tf.regs[0] = match tf.regs[8] {
        63 => fs::sys_read(),
        64 => fs::sys_write(),
        _ => panic!("unimplemented syscall {}\n", tf.regs[8]),
    }
}
