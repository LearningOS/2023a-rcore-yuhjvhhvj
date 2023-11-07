//! Task management implementation
//!
//! Everything about task management, like starting and switching tasks is
//! implemented here.
//!
//! A single global instance of [`TaskManager`] called `TASK_MANAGER` controls
//! all the tasks in the operating system.
//!
//! Be careful when you see `__switch` ASM function in `switch.S`. Control flow around this function
//! might not be what you expect.

mod context;
mod switch;
#[allow(clippy::module_inception)]
mod task;

//
// use VPNRange;
//
use crate::loader::{get_app_data, get_num_app};
use crate::sync::UPSafeCell;
use crate::trap::TrapContext;
use alloc::vec::Vec;
use lazy_static::*;
use switch::__switch;
pub use task::{TaskControlBlock, TaskStatus};

pub use context::TaskContext;

/// The task manager, where all the tasks are managed.
///
/// Functions implemented on `TaskManager` deals with all task state transitions
/// and task context switching. For convenience, you can find wrappers around it
/// in the module level.
///
/// Most of `TaskManager` are hidden behind the field `inner`, to defer
/// borrowing checks to runtime. You can see examples on how to use `inner` in
/// existing functions on `TaskManager`.
pub struct TaskManager {
    /// total number of tasks
    num_app: usize,
    /// use inner value to get mutable access
    inner: UPSafeCell<TaskManagerInner>,
}

/// The task manager inner in 'UPSafeCell'
struct TaskManagerInner {
    /// task list
    tasks: Vec<TaskControlBlock>,
    /// id of current `Running` task
    current_task: usize,
}

lazy_static! {
    /// a `TaskManager` global instance through lazy_static!
    pub static ref TASK_MANAGER: TaskManager = {
        println!("init TASK_MANAGER");
        let num_app = get_num_app();
        println!("num_app = {}", num_app);
        let mut tasks: Vec<TaskControlBlock> = Vec::new();
        for i in 0..num_app {
            tasks.push(TaskControlBlock::new(get_app_data(i), i));
        }
        TaskManager {
            num_app,
            inner: unsafe {
                UPSafeCell::new(TaskManagerInner {
                    tasks,
                    current_task: 0,
                })
            },
        }
    };
}

impl TaskManager {
    /// Run the first task in task list.
    ///
    /// Generally, the first task in task list is an idle task (we call it zero process later).
    /// But in ch4, we load apps statically, so the first task is a real app.
    fn run_first_task(&self) -> ! {
        let mut inner = self.inner.exclusive_access();
        let next_task = &mut inner.tasks[0];
        next_task.task_status = TaskStatus::Running;
        // ch4:start_time
        next_task.start_time = crate::timer::get_time_ms();
        let next_task_cx_ptr = &next_task.task_cx as *const TaskContext;
        drop(inner);
        let mut _unused = TaskContext::zero_init();
        // before this, we should drop local variables that must be dropped manually
        unsafe {
            __switch(&mut _unused as *mut _, next_task_cx_ptr);
        }
        panic!("unreachable in run_first_task!");
    }

    /// Change the status of current `Running` task into `Ready`.
    fn mark_current_suspended(&self) {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].task_status = TaskStatus::Ready;
    }

    /// Change the status of current `Running` task into `Exited`.
    fn mark_current_exited(&self) {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].task_status = TaskStatus::Exited;
    }

    /// Find next task to run and return task id.
    ///
    /// In this case, we only return the first `Ready` task in task list.
    fn find_next_task(&self) -> Option<usize> {
        let inner = self.inner.exclusive_access();
        let current = inner.current_task;
        (current + 1..current + self.num_app + 1)
            .map(|id| id % self.num_app)
            .find(|id| inner.tasks[*id].task_status == TaskStatus::Ready)
    }

    /// Get the current 'Running' task's token.
    fn get_current_token(&self) -> usize {
        let inner = self.inner.exclusive_access();
        inner.tasks[inner.current_task].get_user_token()
    }

    /// Get the current 'Running' task's trap contexts.
    fn get_current_trap_cx(&self) -> &'static mut TrapContext {
        let inner = self.inner.exclusive_access();
        inner.tasks[inner.current_task].get_trap_cx()
    }

    /// Change the current 'Running' task's program break
    pub fn change_current_program_brk(&self, size: i32) -> Option<usize> {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].change_program_brk(size)
    }

    /// Switch current `Running` task to the task we have found,
    /// or there is no `Ready` task and we can exit with all applications completed
    fn run_next_task(&self) {
        if let Some(next) = self.find_next_task() {
            let mut inner = self.inner.exclusive_access();
            let current = inner.current_task;
            inner.tasks[next].task_status = TaskStatus::Running;
            // ch4:assign start_time
            inner.tasks[next].start_time = crate::timer::get_time_ms();
            inner.current_task = next;
            let current_task_cx_ptr = &mut inner.tasks[current].task_cx as *mut TaskContext;
            let next_task_cx_ptr = &inner.tasks[next].task_cx as *const TaskContext;
            drop(inner);
            // before this, we should drop local variables that must be dropped manually
            unsafe {
                __switch(current_task_cx_ptr, next_task_cx_ptr);
            }
            // go back to user mode
        } else {
            panic!("All applications completed!");
        }
    }

    /// 增加计数
    fn increase_current_syscall_count(&self, s_id: usize) {
        let mut inner = self.inner.exclusive_access();
        let ict = inner.current_task;
        inner.tasks[ict].syscall_times[s_id] += 1;
    }

    /// 获取TCB信息
    fn get_current_task_info(&self) -> (usize, [u32; crate::config::MAX_SYSCALL_NUM], TaskStatus) {
        let inner = self.inner.exclusive_access();
        (
            inner.tasks[inner.current_task].start_time,
            inner.tasks[inner.current_task].syscall_times,
            inner.tasks[inner.current_task].task_status,
        )
    }

    /// MAP
    fn get_mm(&self, _start: usize, _len: usize, _port: usize) -> isize {
        //检查起始地址是否对齐
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
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        let start_vpn: crate::mm::VirtPageNum =
            (<usize as Into<crate::mm::VirtAddr>>::into(_start)).floor();
        let end_vpn: crate::mm::VirtPageNum =
            (<usize as Into<crate::mm::VirtAddr>>::into(_start + _len)).ceil();
        let vpn_range = crate::mm::address::VPNRange::new(start_vpn, end_vpn);
        for vpn in vpn_range {
            if inner.tasks[cur].memory_set.translate(vpn).is_some()
                && inner.tasks[cur].memory_set.translate(vpn).unwrap().bits != 0
            {
                return -1;
            }
        }
        inner.tasks[cur].memory_set.insert_framed_area(
            _start.into(),
            (_start + _len).into(),
            permission,
        );
        return 0;
    }

    ///unmap
    fn get_unmap(&self, _start: usize, _len: usize) -> isize {
        //检查起始地址是否对齐
        if (_start % crate::config::PAGE_SIZE) != 0 {
            return -1;
        };
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        let start_vpn: crate::mm::VirtPageNum =
            (<usize as Into<crate::mm::VirtAddr>>::into(_start)).floor();
        let end_vpn: crate::mm::VirtPageNum =
            (<usize as Into<crate::mm::VirtAddr>>::into(_start + _len)).ceil();
        let vpn_range = crate::mm::address::VPNRange::new(start_vpn, end_vpn);
        //检查是否无映射
        let mut id = 0;
        let mut flag = -1;
        for (index, area) in inner.tasks[cur].memory_set.areas.iter().enumerate() {
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
        let ms: &mut crate::mm::MemorySet = &mut inner.tasks[cur].memory_set;
        ms.areas[id].unmap(&mut ms.page_table);
        inner.tasks[cur].memory_set.areas.remove(id);
        return 0;
    }
}

/// Run the first task in task list.
pub fn run_first_task() {
    TASK_MANAGER.run_first_task();
}

/// Switch current `Running` task to the task we have found,
/// or there is no `Ready` task and we can exit with all applications completed
fn run_next_task() {
    TASK_MANAGER.run_next_task();
}

/// Change the status of current `Running` task into `Ready`.
fn mark_current_suspended() {
    TASK_MANAGER.mark_current_suspended();
}

/// Change the status of current `Running` task into `Exited`.
fn mark_current_exited() {
    TASK_MANAGER.mark_current_exited();
}

/// Suspend the current 'Running' task and run the next task in task list.
pub fn suspend_current_and_run_next() {
    mark_current_suspended();
    run_next_task();
}

/// Exit the current 'Running' task and run the next task in task list.
pub fn exit_current_and_run_next() {
    mark_current_exited();
    run_next_task();
}

/// Get the current 'Running' task's token.
pub fn current_user_token() -> usize {
    TASK_MANAGER.get_current_token()
}

/// Get the current 'Running' task's trap contexts.
pub fn current_trap_cx() -> &'static mut TrapContext {
    TASK_MANAGER.get_current_trap_cx()
}

/// Change the current 'Running' task's program break
pub fn change_program_brk(size: i32) -> Option<usize> {
    TASK_MANAGER.change_current_program_brk(size)
}

/// 增加计数
pub fn increase_syscall_count(syscall_id: usize) {
    if syscall_id >= crate::config::MAX_SYSCALL_NUM {
        return;
    }
    TASK_MANAGER.increase_current_syscall_count(syscall_id);
}

/// 获取信息
pub fn get_current_task_info() -> (usize, [u32; crate::config::MAX_SYSCALL_NUM], TaskStatus) {
    TASK_MANAGER.get_current_task_info()
}

/// 获取内存
pub fn get_mm(_start: usize, _len: usize, _port: usize) -> isize {
    TASK_MANAGER.get_mm(_start, _len, _port)
}

/// 销毁已分配内存
pub fn get_unmap(_start: usize, _len: usize) -> isize {
    TASK_MANAGER.get_unmap(_start, _len)
}
