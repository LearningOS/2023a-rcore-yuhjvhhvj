//!Implementation of [`TaskManager`]
use super::TaskControlBlock;
use crate::sync::UPSafeCell;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use lazy_static::*;
///A array of `TaskControlBlock` that is thread-safe
pub struct TaskManager {
    ready_queue: VecDeque<Arc<TaskControlBlock>>,
    sum: usize,
}

/// A simple FIFO scheduler.
impl TaskManager {
    ///Creat an empty TaskManager
    pub fn new() -> Self {
        Self {
            ready_queue: VecDeque::new(),
            sum: 0,
        }
    }
    /// Add process back to ready queue
    pub fn add(&mut self, task: Arc<TaskControlBlock>) {
        self.ready_queue.push_back(task);
        self.sum += 1;
    }

    // /// push_front
    // pub fn add_front(&mut self, task: Arc<TaskControlBlock>) {
    //     self.ready_queue.push_front(task);
    //     self.sum += 1;
    // }

    /// Take a process out of the ready queue
    pub fn fetch(&mut self) -> Option<Arc<TaskControlBlock>> {
        if self.sum != 0 {
            self.sum -= 1;
        }
        self.ready_queue.pop_front()
    }

    ///获取总数
    pub fn get_task_sum_in_ready(&self) -> usize {
        self.sum
    }
}

lazy_static! {
    /// TASK_MANAGER instance through lazy_static!
    pub static ref TASK_MANAGER: UPSafeCell<TaskManager> =
        unsafe { UPSafeCell::new(TaskManager::new()) };
}

/// Add process to ready queue
pub fn add_task(task: Arc<TaskControlBlock>) {
    //trace!("kernel: TaskManager::add_task");
    TASK_MANAGER.exclusive_access().add(task);
}

// ///
// pub fn add_front_task(task: Arc<TaskControlBlock>) {
//     //trace!("kernel: TaskManager::add_task");
//     TASK_MANAGER.exclusive_access().add_front(task);
// }

/// Take a process out of the ready queue
pub fn fetch_task() -> Option<Arc<TaskControlBlock>> {
    //trace!("kernel: TaskManager::fetch_task");
    TASK_MANAGER.exclusive_access().fetch()
}

/// 获取总数
pub fn get_task_sum_in_ready() -> usize {
    TASK_MANAGER.exclusive_access().get_task_sum_in_ready()
}
