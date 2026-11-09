//! Thread priorities. The scan and import readers and workers run below normal, or idle, so the app keeps its processor time.

/// A thread's priority. Windows schedules by them; on Linux they are nice values, per thread (below); elsewhere a thread
/// keeps its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    Normal,
    BelowNormal,
    Idle,
}

/// Set the calling thread's priority.
#[cfg(windows)]
pub fn set_current_thread(priority: Priority) {
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL, THREAD_PRIORITY_IDLE, THREAD_PRIORITY_NORMAL,
    };
    let value = match priority {
        Priority::Normal => THREAD_PRIORITY_NORMAL,
        Priority::BelowNormal => THREAD_PRIORITY_BELOW_NORMAL,
        Priority::Idle => THREAD_PRIORITY_IDLE,
    };
    // SAFETY: GetCurrentThread's pseudo-handle is always valid for the calling thread.
    unsafe { SetThreadPriority(GetCurrentThread(), value) };
}

/// On Linux a thread's nice value is its own (setpriority with a thread id names that thread). Idle is nice 19, not
/// SCHED_IDLE: an idle-class thread gets no processor time while any normal thread can run, so one that holds a lock a
/// normal thread waits for would stall it for as long as the machine is busy. Nice 19 still gets a small share.
/// Normal is left as it is, and a thread that can't be given a lower nice value (an unprivileged one can't) keeps its own:
/// the threads that are set here end with their run.
#[cfg(target_os = "linux")]
pub fn set_current_thread(priority: Priority) {
    let nice = match priority {
        Priority::Normal => return,
        Priority::BelowNormal => 5,
        Priority::Idle => 19,
    };
    // SAFETY: setpriority takes a thread id and a nice value; a failure is a thread that keeps its priority.
    unsafe { libc::setpriority(libc::PRIO_PROCESS, current_thread_id(), nice) };
}

/// The calling thread's id, as setpriority and getpriority take it on Linux.
#[cfg(target_os = "linux")]
fn current_thread_id() -> libc::id_t {
    // SAFETY: gettid takes no arguments and cannot fail.
    unsafe { libc::syscall(libc::SYS_gettid) as libc::id_t }
}

/// Elsewhere the priority is not changed.
#[cfg(not(any(windows, target_os = "linux")))]
pub fn set_current_thread(_priority: Priority) {}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    /// The calling thread's nice value.
    fn nice_of_current_thread() -> i32 {
        // SAFETY: getpriority takes a thread id and returns the nice value of that thread.
        unsafe { libc::getpriority(libc::PRIO_PROCESS, current_thread_id()) }
    }

    /// The nice value a new thread has after it sets `priority`. Each thread has its own, so each test gets one.
    fn nice_after(priority: Priority) -> i32 {
        std::thread::spawn(move || {
            set_current_thread(priority);
            nice_of_current_thread()
        })
        .join()
        .unwrap()
    }

    #[test]
    fn idle_and_below_normal_are_nice_values() {
        assert_eq!(nice_after(Priority::Idle), 19);
        assert_eq!(nice_after(Priority::BelowNormal), 5);
    }

    #[test]
    fn normal_leaves_the_thread_as_it_was() {
        let before = std::thread::spawn(nice_of_current_thread).join().unwrap();
        assert_eq!(nice_after(Priority::Normal), before);
    }
}
