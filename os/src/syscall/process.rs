//! Process management syscalls
use alloc::sync::Arc;

use crate::{
    config::MAX_SYSCALL_NUM,
    loader::get_app_data_by_name,
    mm::{translated_refmut, translated_str},
    task::{
        add_task, current_task, current_user_token, exit_current_and_run_next,
        suspend_current_and_run_next, TaskStatus,
    },
};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// Task information
#[allow(dead_code)]
pub struct TaskInfo {
    /// Task status in it's life cycle
    status: TaskStatus,
    /// The numbers of syscall called by task
    syscall_times: [u32; MAX_SYSCALL_NUM],
    /// Total running time of task
    time: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(exit_code: i32) -> ! {
    trace!("kernel:pid[{}] sys_exit", current_task().unwrap().pid.0);
    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel:pid[{}] sys_yield", current_task().unwrap().pid.0);
    suspend_current_and_run_next();
    0
}

pub fn sys_getpid() -> isize {
    trace!("kernel: sys_getpid pid:{}", current_task().unwrap().pid.0);
    current_task().unwrap().pid.0 as isize
}

pub fn sys_fork() -> isize {
    trace!("kernel:pid[{}] sys_fork", current_task().unwrap().pid.0);
    let current_task = current_task().unwrap();
    let new_task = current_task.fork();
    let new_pid = new_task.pid.0;
    // modify trap context of new_task, because it returns immediately after switching
    let trap_cx = new_task.inner_exclusive_access().get_trap_cx();
    // we do not have to move to next instruction since we have done it before
    // for child process, fork returns 0
    trap_cx.x[10] = 0;
    // add new task to scheduler
    add_task(new_task);
    new_pid as isize
}

pub fn sys_exec(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_exec", current_task().unwrap().pid.0);
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(data) = get_app_data_by_name(path.as_str()) {
        let task = current_task().unwrap();
        task.exec(data);
        0
    } else {
        -1
    }
}

/// If there is not a child process whose pid is same as given, return -1.
/// Else if there is a child process but it is still running, return -2.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    trace!(
        "kernel::pid[{}] sys_waitpid [{}]",
        current_task().unwrap().pid.0,
        pid
    );
    let task = current_task().unwrap();
    // find a child process

    // ---- access current PCB exclusively
    let mut inner = task.inner_exclusive_access();
    if !inner
        .children
        .iter()
        .any(|p| pid == -1 || pid as usize == p.getpid())
    {
        return -1;
        // ---- release current PCB
    }
    let pair = inner.children.iter().enumerate().find(|(_, p)| {
        // ++++ temporarily access child PCB exclusively
        p.inner_exclusive_access().is_zombie() && (pid == -1 || pid as usize == p.getpid())
        // ++++ release child PCB
    });
    if let Some((idx, _)) = pair {
        let child = inner.children.remove(idx);
        // confirm that child will be deallocated after being removed from children list
        assert_eq!(Arc::strong_count(&child), 1);
        let found_pid = child.getpid();
        // ++++ temporarily access child PCB exclusively
        let exit_code = child.inner_exclusive_access().exit_code;
        // ++++ release child PCB
        *translated_refmut(inner.memory_set.token(), exit_code_ptr) = exit_code;
        found_pid as isize
    } else {
        -2
    }
    // ---- release current PCB automatically
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(_ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel:pid[{}] sys_get_time", current_task().unwrap().pid.0);
    let us = crate::timer::get_time_us();
    let bufs = crate::mm::translated_byte_buffer(
        current_task().unwrap().get_user_token(),
        _ts as *const u8,
        core::mem::size_of::<TimeVal>(),
    );
    let time_val = TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };
    let mut ptr = &time_val as *const _ as *const u8;
    for buf in bufs {
        unsafe {
            ptr.copy_to(buf.as_mut_ptr(), buf.len());
            ptr = ptr.add(buf.len());
        }
    }
    0
}

/// YOUR JOB: Finish sys_task_info to pass testcases
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TaskInfo`] is splitted by two pages ?
pub fn sys_task_info(_ti: *mut TaskInfo) -> isize {
    trace!(
        "kernel:pid[{}] sys_task_info NOT IMPLEMENTED",
        current_task().unwrap().pid.0
    );
    -1
}

/// YOUR JOB: Implement mmap.
pub fn sys_mmap(_start: usize, _len: usize, _port: usize) -> isize {
    trace!("kernel:pid[{}] sys_mmap", current_task().unwrap().pid.0);
    if (_start % crate::config::PAGE_SIZE) != 0 {
        return -1;
    };
    if _port != 1 && _port != 2 && _port != 3 {
        return -1;
    }
    //设置标志位
    let mut permission = crate::mm::MapPermission::from_bits((_port as u8) << 1).unwrap();
    permission.set(crate::mm::MapPermission::U, true);
    //TCB
    let tcb = current_task().unwrap();
    let mut inner = tcb.inner.exclusive_access();
    let start_vpn: crate::mm::VirtPageNum =
        (<usize as Into<crate::mm::VirtAddr>>::into(_start)).floor();
    let end_vpn: crate::mm::VirtPageNum =
        (<usize as Into<crate::mm::VirtAddr>>::into(_start + _len)).ceil();
    let vpn_range = crate::mm::address::VPNRange::new(start_vpn, end_vpn);
    for vpn in vpn_range {
        if inner.memory_set.translate(vpn).is_some()
            && inner.memory_set.translate(vpn).unwrap().bits != 0
        {
            return -1;
        }
    }
    inner
        .memory_set
        .insert_framed_area(_start.into(), (_start + _len).into(), permission);
    return 0;
}

/// YOUR JOB: Implement munmap.
pub fn sys_munmap(_start: usize, _len: usize) -> isize {
    trace!("kernel:pid[{}] sys_munmap", current_task().unwrap().pid.0);
    if (_start % crate::config::PAGE_SIZE) != 0 {
        return -1;
    };
    //TCB
    let tcb = current_task().unwrap();
    let mut inner = tcb.inner.exclusive_access();
    let start_vpn: crate::mm::VirtPageNum =
        (<usize as Into<crate::mm::VirtAddr>>::into(_start)).floor();
    let end_vpn: crate::mm::VirtPageNum =
        (<usize as Into<crate::mm::VirtAddr>>::into(_start + _len)).ceil();
    let vpn_range = crate::mm::address::VPNRange::new(start_vpn, end_vpn);
    //检查是否无映射
    let mut id = 0;
    let mut flag = -1;
    for (index, area) in inner.memory_set.areas.iter().enumerate() {
        if area.vpn_range.get_start() == vpn_range.get_start()
            && area.vpn_range.get_end() == vpn_range.get_end()
        {
            id = index;
            flag = 0;
        }
    }
    if flag == -1 {
        return -1;
    }
    let ms: &mut crate::mm::MemorySet = &mut inner.memory_set;
    ms.areas[id].unmap(&mut ms.page_table);
    inner.memory_set.areas.remove(id);
    return 0;
}

/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel:pid[{}] sys_sbrk", current_task().unwrap().pid.0);
    if let Some(old_brk) = current_task().unwrap().change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}

/// YOUR JOB: Implement spawn.
/// HINT: fork + exec =/= spawn
pub fn sys_spawn(_path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_spawn", current_task().unwrap().pid.0);
    // let current_task = current_task().unwrap();
    let token = current_user_token();
    let path = translated_str(token, _path);
    if let Some(data) = get_app_data_by_name(path.as_str()) {
        let task = current_task().unwrap();
        let new_task = task.span(data);
        let new_pid = new_task.pid.0;
        add_task(new_task);
        new_pid as isize
    } else {
        -1
    }
}

// YOUR JOB: Set task priority.
pub fn sys_set_priority(_prio: isize) -> isize {
    trace!(
        "kernel:pid[{}] sys_set_priority",
        current_task().unwrap().pid.0
    );
    if _prio <= 1 {
        return -1;
    }
    let task = current_task().unwrap();
    let mut task_inner = task.inner_exclusive_access();
    task_inner.task_priority = _prio;
    _prio
}
